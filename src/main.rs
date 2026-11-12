//! Stepwise — see Python code run, one step at a time.
//!
//! The main window is the editor (the teacher's console). The visualization
//! opens in a second window that can be moved to a projector and stepped
//! through like a slide deck.

mod examples;
mod export;
mod highlight;
mod trace;
mod viz;

use eframe::egui::{
    self,
    text::{CCursor, CCursorRange},
    Button, CentralPanel, Color32, FontId, Key, Label, Modifiers, Panel, RichText, ScrollArea,
    TextEdit, TextStyle, Ui, ViewportBuilder, ViewportCommand, ViewportId,
};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use trace::Trace;

const EDITOR_FONT: f32 = 15.0;
const PLAY_INTERVAL: f64 = 0.9;
const ERROR_RED: Color32 = Color32::from_rgb(0xD6, 0x3A, 0x2F);
const OK_GREEN: Color32 = Color32::from_rgb(0x1E, 0x7D, 0x32);
/// How long a status message stays up in the presentation window.
const NOTICE_SECONDS: f64 = 8.0;

/// `stepwise program.py --export out.pdf|.pptx|.png|folder [--step N] [--dark]`
/// exports without opening a window. Returns None when no export was asked for.
fn export_from_command_line() -> Option<Result<String, String>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let out = PathBuf::from(value("--export")?);
    let run = || {
        let source = args
            .iter()
            .find(|a| a.ends_with(".py"))
            .ok_or("Give the Python file to run, e.g. stepwise program.py --export out.pdf")?;
        let code = std::fs::read_to_string(source).map_err(|e| format!("Could not open {source}: {e}"))?;
        let code = code.replace("\r\n", "\n").replace('\t', "    ");
        let stdin = value("--input").map_or(String::new(), |s| s.replace("\\n", "\n"));
        let trace = trace::run_trace(&code, &stdin, std::path::Path::new(source).parent().filter(|p| !p.as_os_str().is_empty()))?;
        if let Some(e) = &trace.error {
            return Err(format!("Line {}: {}", e.line, e.msg));
        }
        let format = match out.extension().and_then(|e| e.to_str()) {
            Some("pdf") => export::Format::Pdf,
            Some("pptx") => export::Format::Pptx,
            Some("png") => export::Format::Png,
            _ => {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                export::Format::PngFolder
            }
        };
        let last = trace.steps.len().saturating_sub(1);
        let step = value("--step").and_then(|s| s.parse::<usize>().ok()).map_or(last, |n| n.saturating_sub(1).min(last));
        let dark = args.iter().any(|a| a == "--dark");
        let note = if trace.limit.is_some() { " (the run was cut short at its limit)" } else { "" };
        export::run(export::Job { code, trace, step, dark, format, path: out.clone() }).map(|msg| msg + note)
    };
    Some(run())
}

fn main() -> eframe::Result {
    // The app runs the student's program in a copy of itself that acts as the
    // Python interpreter, so a runaway program can be stopped without harm.
    if std::env::var_os(trace::INTERPRETER_ENV).is_some() {
        use rustpython::InterpreterBuilderExt;
        let _ = rustpython::run(rustpython::InterpreterBuilder::new().init_stdlib());
        std::process::exit(0);
    }
    match export_from_command_line() {
        Some(Ok(msg)) => {
            println!("{msg}");
            return Ok(());
        }
        Some(Err(e)) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
        None => {}
    }
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Stepwise — Editor")
            .with_icon(app_icon())
            .with_inner_size([780.0, 780.0])
            .with_min_inner_size([520.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("Stepwise", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

/// How many spaces the program uses per indentation level (4 if it has none yet).
fn indent_unit(code: &str) -> usize {
    code.lines()
        .map(|l| l.len() - l.trim_start_matches(' ').len())
        .find(|n| *n > 0)
        .map_or(4, |n| n.clamp(1, 8))
}

fn app_icon() -> std::sync::Arc<egui::IconData> {
    let icon = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .map(|img| {
            let img = img.into_rgba8();
            egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
        })
        .unwrap_or_default();
    std::sync::Arc::new(icon)
}

fn presentation_id() -> ViewportId {
    ViewportId::from_hash_of("stepwise_presentation")
}

struct App {
    code: String,
    stdin: String,
    path: Option<PathBuf>,

    /// The last successful run, with the exact code that produced it.
    trace: Option<Trace>,
    traced_code: String,
    step: usize,
    error: Option<String>,
    running: Option<Receiver<(String, Result<Trace, String>)>>,
    pending_export: Option<export::Format>,
    exporting: Option<Receiver<Result<String, String>>>,
    /// The latest status message, and when it was set (for the presentation
    /// window, where it is shown only briefly).
    notice: Option<String>,
    notice_is_error: bool,
    notice_at: f64,

    /// Show the visualization in its own window (true) or beside the editor.
    separate: bool,
    window_open: bool,
    focus_window: bool,
    scale: f32,
    playing: bool,
    next_tick: f64,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.all_styles_mut(|style| {
            for (text_style, font) in style.text_styles.iter_mut() {
                font.size = match text_style {
                    TextStyle::Heading => 20.0,
                    TextStyle::Small => 11.5,
                    _ => 14.5,
                };
            }
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
        });
        // A lit classroom and a projector: light is the readable default.
        cc.egui_ctx.set_theme(egui::Theme::Light);
        let mut app = Self {
            code: examples::EXAMPLES[0].1.to_owned(),
            stdin: String::new(),
            path: None,
            trace: None,
            traced_code: String::new(),
            step: 0,
            error: None,
            running: None,
            pending_export: None,
            exporting: None,
            notice: None,
            notice_is_error: false,
            notice_at: 0.0,
            separate: true,
            window_open: false,
            focus_window: false,
            scale: 1.25,
            playing: false,
            next_tick: 0.0,
        };
        // `stepwise program.py` opens a file directly.
        if let Some(arg) = std::env::args().skip(1).find(|a| !a.starts_with("--")) {
            app.open(PathBuf::from(arg));
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.code = text.replace("\r\n", "\n").replace('\t', "    ");
                self.path = Some(path);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Could not open {}: {e}", path.display())),
        }
    }

    fn save(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("Python", &["py"]);
        dialog = match &self.path {
            Some(p) => dialog.set_file_name(p.file_name().unwrap_or_default().to_string_lossy()),
            None => dialog.set_file_name("program.py"),
        };
        if let Some(path) = dialog.save_file() {
            match std::fs::write(&path, &self.code) {
                Ok(()) => self.path = Some(path),
                Err(e) => self.error = Some(format!("Could not save: {e}")),
            }
        }
    }

    fn run(&mut self, ctx: &egui::Context) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let code = self.code.clone();
        let stdin = self.stdin.clone();
        let cwd = self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = trace::run_trace(&code, &stdin, cwd.as_deref());
            let _ = tx.send((code, result));
            ctx.request_repaint();
        });
        self.running = Some(rx);
        self.error = None;
        self.notice = None;
        self.playing = false;
    }

    fn set_notice(&mut self, ctx: &egui::Context, text: String, is_error: bool) {
        self.notice = Some(text);
        self.notice_is_error = is_error;
        self.notice_at = ctx.input(|i| i.time);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.exporting.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.exporting = None;
            match result {
                Ok(msg) => self.set_notice(ctx, msg, false),
                Err(e) => self.set_notice(ctx, e, true),
            }
        }
        let Some(rx) = &self.running else { return };
        let Ok((code, result)) = rx.try_recv() else { return };
        self.running = None;
        match result {
            Err(e) => self.error = Some(e),
            Ok(t) => {
                if let Some(e) = &t.error {
                    self.error = Some(format!("Line {}: {}", e.line, e.msg));
                } else if !t.steps.is_empty() {
                    if let Some(reason) = &t.limit {
                        let limit = match reason.as_str() {
                            "time" => "the time limit".to_owned(),
                            _ => format!("the limit of {} steps", trace::MAX_STEPS),
                        };
                        self.error = Some(format!(
                            "The run reached {limit}, so only its beginning is shown. Is there an infinite loop?"
                        ));
                    }
                    self.trace = Some(t);
                    self.traced_code = code;
                    self.step = 0;
                    self.window_open = true;
                    self.focus_window = true;
                }
            }
        }
    }

    fn last_step(&self) -> usize {
        self.trace.as_ref().map_or(0, |t| t.steps.len().saturating_sub(1))
    }

    fn go(&mut self, step: usize) {
        self.step = step.min(self.last_step());
    }

    fn tick(&mut self, ctx: &egui::Context) {
        if !self.playing {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now >= self.next_tick {
            self.go(self.step + 1);
            self.next_tick = now + PLAY_INTERVAL;
            if self.step >= self.last_step() {
                self.playing = false;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    /// Slide-deck style keyboard navigation.
    fn keys(&mut self, ctx: &egui::Context, in_window: bool) {
        if self.trace.is_none() || ctx.text_edit_focused() {
            return;
        }
        let (next, prev, first, last, play, bigger, smaller, full, esc) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::PageDown),
                i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::PageUp),
                i.key_pressed(Key::Home),
                i.key_pressed(Key::End),
                i.key_pressed(Key::Space),
                i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals),
                i.key_pressed(Key::Minus),
                i.key_pressed(Key::F),
                i.key_pressed(Key::Escape),
            )
        });
        if next {
            self.playing = false;
            self.go(self.step + 1);
        }
        if prev {
            self.playing = false;
            self.go(self.step.saturating_sub(1));
        }
        if first {
            self.go(0);
        }
        if last {
            self.go(usize::MAX);
        }
        if play {
            self.toggle_play(ctx);
        }
        if bigger {
            self.scale = (self.scale + 0.1).min(3.0);
        }
        if smaller {
            self.scale = (self.scale - 0.1).max(0.6);
        }
        if in_window {
            let is_full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            if full || (esc && is_full) {
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!is_full));
            }
        }
    }

    fn toggle_play(&mut self, ctx: &egui::Context) {
        self.playing = !self.playing;
        if self.playing {
            if self.step >= self.last_step() {
                self.step = 0;
            }
            self.next_tick = ctx.input(|i| i.time) + PLAY_INTERVAL;
        }
    }

    // ------------------------------------------------------------ editor

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Stepwise").strong().size(18.0));
            ui.add_space(6.0);
            let busy = self.running.is_some();
            let label = if busy { "Running…" } else { "Visualize" };
            let run = Button::new(RichText::new(label).strong().color(Color32::WHITE))
                .fill(Color32::from_rgb(0x1F, 0x6F, 0xD6));
            if ui
                .add_enabled(!busy, run)
                .on_hover_text("Run the program and show it step by step (Ctrl/Cmd + Enter)")
                .clicked()
            {
                self.run(ui.ctx());
            }
            ui.menu_button("Examples", |ui| {
                for (name, code) in examples::EXAMPLES {
                    if ui.button(*name).clicked() {
                        self.code = (*code).to_owned();
                        self.path = None;
                        ui.close();
                    }
                }
            });
            if ui.button("Open…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Python", &["py"]).pick_file() {
                    self.open(p);
                }
            }
            if ui.button("Save…").clicked() {
                self.save();
            }
            self.export_menu(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::widgets::global_theme_preference_buttons(ui);
                ui.checkbox(&mut self.separate, "Presentation window")
                    .on_hover_text("Show the visualization in its own window (for a projector) or beside the editor");
            });
        });
        ui.add_space(4.0);
    }

    fn status(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        if let Some(e) = &self.error {
            ui.label(RichText::new(e).color(ERROR_RED).strong());
        }
        if let Some(n) = &self.notice {
            ui.label(RichText::new(n).color(if self.notice_is_error { ERROR_RED } else { OK_GREEN }));
        }
        ui.horizontal(|ui| {
            if self.trace.is_none() {
                ui.label(RichText::new("Write Python, then press Visualize.").weak());
                return;
            }
            if self.separate {
                // Remote control for the presentation window.
                if !self.window_open && ui.button("Show presentation window").clicked() {
                    self.window_open = true;
                    self.focus_window = true;
                }
                if ui.button("Prev").clicked() {
                    self.go(self.step.saturating_sub(1));
                }
                if ui.button("Next").clicked() {
                    self.go(self.step + 1);
                }
                ui.label(format!("Step {} of {}", self.step + 1, self.last_step() + 1));
            }
            if let Some(t) = &self.trace {
                ui.label(RichText::new(&t.python).weak());
            }
            if self.code != self.traced_code {
                ui.label(RichText::new("Code changed — press Visualize to update.").weak().italics());
            }
        });
        ui.add_space(4.0);
    }

    fn editor(&mut self, ui: &mut Ui) {
        Panel::bottom("stdin_panel").show(ui, |ui| {
            ui.add_space(2.0);
            ui.collapsing("Input for input()  — one line per call", |ui| {
                ui.add(
                    TextEdit::multiline(&mut self.stdin)
                        .font(TextStyle::Monospace)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("Ana\n12"),
                );
            });
            ui.add_space(2.0);
        });
        CentralPanel::default().show(ui, |ui| {
            if let Some(p) = &self.path {
                ui.label(RichText::new(p.file_name().unwrap_or_default().to_string_lossy()).weak().small())
                    .on_hover_text(p.display().to_string());
            }
            let dark = ui.visuals().dark_mode;
            let font = FontId::monospace(EDITOR_FONT);
            let weak = ui.visuals().weak_text_color();
            let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
                ui.painter().layout_job(highlight::highlight(text.as_str(), font.clone(), dark))
            };
            ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let n = self.code.split('\n').count();
                    let w = n.to_string().len().max(2);
                    let nums: Vec<String> = (1..=n).map(|i| format!("{i:>w$}")).collect();
                    ui.add(
                        Label::new(RichText::new(nums.join("\n")).font(font.clone()).color(weak))
                            .selectable(false),
                    );
                    let mut out = TextEdit::multiline(&mut self.code)
                        .code_editor()
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_width(f32::INFINITY)
                        .desired_rows(24)
                        .layouter(&mut layouter)
                        .show(ui);
                    if !out.response.changed() {
                        return;
                    }
                    // Python-friendly typing: Tab gives four spaces, Enter keeps the indentation.
                    let Some(cursor) = out.cursor_range.map(|r| r.primary.index.0) else { return };
                    let mut new_cursor = None;
                    let unit = " ".repeat(indent_unit(&self.code));
                    if self.code.contains('\t') {
                        let tabs = self.code.chars().take(cursor).filter(|c| *c == '\t').count();
                        self.code = self.code.replace('\t', &unit);
                        new_cursor = Some(cursor + (unit.len() - 1) * tabs);
                    } else if ui.input(|i| i.key_pressed(Key::Enter)) && cursor > 0 {
                        let byte = self.code.char_indices().nth(cursor).map_or(self.code.len(), |(b, _)| b);
                        if self.code[..byte].ends_with('\n') {
                            let before = &self.code[..byte - 1];
                            let line = &before[before.rfind('\n').map_or(0, |p| p + 1)..];
                            let mut indent: String =
                                line.chars().take_while(|c| *c == ' ').collect();
                            if line.trim_end().ends_with(':') {
                                indent.push_str(&unit);
                            }
                            if !indent.is_empty() {
                                self.code.insert_str(byte, &indent);
                                new_cursor = Some(cursor + indent.len());
                            }
                        }
                    }
                    if let Some(c) = new_cursor {
                        out.state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(c))));
                        out.state.store(ui.ctx(), out.response.id);
                    }
                });
            });
        });
    }

    // ------------------------------------------------------ presentation

    /// The sentence describing the step, under the code. Two lines are always
    /// reserved so the code above does not jump when the sentence wraps.
    fn caption_block(&mut self, ui: &mut Ui) {
        let Some(trace) = &self.trace else { return };
        let text = viz::caption(trace, self.step.min(trace.steps.len() - 1));
        let s = self.scale.min(1.6);
        ui.add_space(4.0);
        viz::legend(ui, self.scale);
        ui.add_space(6.0);
        ui.scope(|ui| {
            ui.set_min_height(15.5 * s * 2.6);
            ui.label(RichText::new(text).size(15.5 * s).color(ui.visuals().strong_text_color()));
        });
    }

    /// Step controls: one row along the bottom edge, out of the way of the content.
    fn nav_bar(&mut self, ui: &mut Ui, in_window: bool) {
        let last = self.last_step();
        let cut_short = self.trace.as_ref().is_some_and(|t| t.limit.is_some());
        // In its own window the presentation has no status bar, so the result
        // of an export is shown here for a few seconds.
        if in_window {
            if let Some(n) = &self.notice {
                let left = self.notice_at + NOTICE_SECONDS - ui.input(|i| i.time);
                if self.exporting.is_some() || left > 0.0 {
                    ui.add_space(4.0);
                    ui.label(RichText::new(n).color(if self.notice_is_error { ERROR_RED } else { OK_GREEN }));
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left.max(0.2)));
                }
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let btn = |ui: &mut Ui, text: &str, key: &str, enabled: bool| {
                ui.add_enabled(enabled, Button::new(text)).on_hover_text(key).clicked()
            };
            if btn(ui, "First", "Home", self.step > 0) {
                self.step = 0;
            }
            if btn(ui, "Prev", "Left arrow", self.step > 0) {
                self.playing = false;
                self.step -= 1;
            }
            if btn(ui, "Next", "Right arrow", self.step < last) {
                self.playing = false;
                self.step += 1;
            }
            if btn(ui, "Last", "End", self.step < last) {
                self.step = last;
            }
            if btn(ui, if self.playing { "Pause" } else { "Play" }, "Space", true) {
                self.toggle_play(ui.ctx());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.export_menu(ui);
                if in_window && btn(ui, "Fullscreen", "F", true) {
                    let is_full = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    ui.ctx().send_viewport_cmd(ViewportCommand::Fullscreen(!is_full));
                }
                if btn(ui, "A+", "Bigger text (+)", self.scale < 2.99) {
                    self.scale = (self.scale + 0.1).min(3.0);
                }
                if btn(ui, "A−", "Smaller text (−)", self.scale > 0.61) {
                    self.scale = (self.scale - 0.1).max(0.6);
                }
                ui.separator();
                if cut_short {
                    let warn = if ui.visuals().dark_mode {
                        Color32::from_rgb(0xFF, 0xC8, 0x66)
                    } else {
                        Color32::from_rgb(0xA3, 0x4F, 0x00)
                    };
                    ui.label(RichText::new("run cut short").color(warn).strong())
                        .on_hover_text("The run reached its limit before the program finished. Only the beginning is shown.");
                }
                ui.add(
                    Label::new(RichText::new(format!("Step {} of {}", self.step + 1, last + 1)).strong())
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
                ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(40.0);
                let mut step = self.step;
                if ui.add(egui::Slider::new(&mut step, 0..=last).show_value(false)).changed() {
                    self.playing = false;
                    self.step = step;
                }
            });
        });
        ui.add_space(6.0);
    }

    fn export_menu(&mut self, ui: &mut Ui) {
        let ready = self.trace.is_some() && self.exporting.is_none();
        let reason = if self.exporting.is_some() { "An export is running" } else { "Press Visualize first" };
        ui.add_enabled_ui(ready, |ui| {
            ui.menu_button("Export", |ui| {
                for (label, format) in [
                    ("This step as PNG…", export::Format::Png),
                    ("All steps as PDF…", export::Format::Pdf),
                    ("All steps as PowerPoint…", export::Format::Pptx),
                    ("All steps as PNG files…", export::Format::PngFolder),
                ] {
                    if ui.button(label).clicked() {
                        // The file dialog opens from the main window's next frame.
                        self.pending_export = Some(format);
                        ui.close();
                    }
                }
            });
        })
        .response
        .on_disabled_hover_text(reason);
    }

    fn start_export(&mut self, ctx: &egui::Context, format: export::Format) {
        let Some(trace) = &self.trace else { return };
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("program".to_owned(), |s| s.to_string_lossy().into_owned());
        let dialog = rfd::FileDialog::new();
        let path = match format {
            export::Format::Png => dialog
                .add_filter("PNG image", &["png"])
                .set_file_name(format!("{stem}-step-{}.png", self.step + 1))
                .save_file(),
            export::Format::Pdf => dialog.add_filter("PDF", &["pdf"]).set_file_name(format!("{stem}.pdf")).save_file(),
            export::Format::Pptx => dialog
                .add_filter("PowerPoint", &["pptx"])
                .set_file_name(format!("{stem}.pptx"))
                .save_file(),
            export::Format::PngFolder => dialog.set_title("Folder for the PNG files").pick_folder(),
        };
        let Some(path) = path else { return };
        let job = export::Job {
            code: self.traced_code.clone(),
            trace: trace.clone(),
            step: self.step,
            dark: ctx.global_style().visuals.dark_mode,
            format,
            path,
        };
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(export::run(job));
            repaint.request_repaint();
        });
        self.exporting = Some(rx);
        self.set_notice(ctx, "Exporting…".to_owned(), false);
        self.error = None;
    }

    fn presentation(&mut self, ui: &mut Ui, in_window: bool) {
        let scale = self.scale;
        let default_w = (ui.available_width() * 0.4).clamp(300.0, 640.0);
        Panel::bottom("pres_bar").show(ui, |ui| self.nav_bar(ui, in_window));
        Panel::left("pres_code").default_size(default_w).show(ui, |ui| {
            Panel::bottom("pres_output").resizable(true).default_size(130.0).show(ui, |ui| {
                ui.add_space(4.0);
                ui.label(RichText::new("Output of print()").weak().size(12.0 * scale.min(1.4)));
                ScrollArea::both().id_salt("out").auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
                    if let Some(t) = &self.trace {
                        let text = t.output_at(self.step);
                        ui.add(
                            Label::new(RichText::new(text).monospace().size(14.0 * scale))
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    }
                });
            });
            Panel::bottom("pres_caption").show(ui, |ui| self.caption_block(ui));
            CentralPanel::default().show(ui, |ui| {
                ScrollArea::both().id_salt("code").auto_shrink(false).show(ui, |ui| {
                    if let Some(t) = &self.trace {
                        viz::code_view(ui, &self.traced_code, t, self.step, scale);
                    }
                });
            });
        });
        CentralPanel::default().show(ui, |ui| {
            ScrollArea::both().id_salt("memory").auto_shrink(false).show(ui, |ui| {
                if let Some(t) = &self.trace {
                    viz::memory_view(ui, t, self.step, scale);
                }
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.tick(&ctx);
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open(path);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
            self.run(&ctx);
        }
        self.keys(&ctx, false);
        if let Some(format) = self.pending_export.take() {
            self.start_export(&ctx, format);
        }

        Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        Panel::bottom("status").show(ui, |ui| self.status(ui));
        if !self.separate && self.trace.is_some() {
            Panel::left("editor_panel").default_size(400.0).show(ui, |ui| self.editor(ui));
            self.presentation(ui, false);
        } else {
            CentralPanel::default().show(ui, |ui| self.editor(ui));
        }

        if self.separate && self.window_open && self.trace.is_some() {
            let builder = ViewportBuilder::default()
                .with_title("Stepwise — Presentation")
                .with_icon(app_icon())
                .with_inner_size([1280.0, 800.0]);
            ctx.show_viewport_immediate(presentation_id(), builder, |ui, _class| {
                let ctx = ui.ctx().clone();
                self.keys(&ctx, true);
                self.presentation(ui, true);
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.window_open = false;
                    self.playing = false;
                }
            });
            if self.focus_window {
                self.focus_window = false;
                ctx.send_viewport_cmd_to(presentation_id(), ViewportCommand::Focus);
            }
        }
    }
}
