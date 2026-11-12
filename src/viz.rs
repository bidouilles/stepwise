//! Drawing of one execution step: the code with its arrows, the stack frames,
//! the heap objects and the pointers between them.

use crate::highlight::highlight;
use crate::trace::{Obj, Step, Trace, Val};
use eframe::egui::{
    self, epaint::CubicBezierShape, pos2, vec2, Align2, Color32, FontId, Galley, Painter, Pos2,
    Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

struct Pal {
    text: Color32,
    dim: Color32,
    frame_bg: Color32,
    frame_active: Color32,
    border: Color32,
    cell: Color32,
    obj_cell: Color32,
    obj_key: Color32,
    obj_dim: Color32,
    arrow: Color32,
    arrow_active: Color32,
    changed: Color32,
    changed_mark: Color32,
    string: Color32,
    number: Color32,
    constant: Color32,
    ret: Color32,
    /// Function names, as in the code.
    def: Color32,
    line_next: Color32,
    line_prev: Color32,
    line_bg: Color32,
    error_bg: Color32,
}

fn pal(dark: bool) -> Pal {
    let c = Color32::from_rgb;
    if dark {
        Pal {
            text: c(0xE4, 0xE7, 0xEC),
            dim: c(0x8D, 0x96, 0xA5),
            frame_bg: c(0x2A, 0x2E, 0x36),
            frame_active: c(0x1F, 0x3A, 0x5F),
            border: c(0x5A, 0x63, 0x72),
            cell: c(0x33, 0x38, 0x42),
            obj_cell: c(0x45, 0x40, 0x22),
            obj_key: c(0x5A, 0x48, 0x24),
            obj_dim: c(0xC2, 0xB4, 0x86),
            arrow: c(0x7F, 0xA0, 0xD0),
            arrow_active: c(0x9C, 0xC8, 0xFF),
            changed: c(0x7A, 0x5A, 0x10),
            changed_mark: c(0xFF, 0xC8, 0x66),
            string: c(0xA5, 0xD6, 0x8A),
            number: c(0xF2, 0xA8, 0x6B),
            constant: c(0xC7, 0x92, 0xEA),
            ret: c(0xFF, 0x8A, 0x80),
            def: c(0x7F, 0xB4, 0xF7),
            line_next: c(0xFF, 0x5C, 0x5C),
            line_prev: c(0x6F, 0xD0, 0x8C),
            line_bg: c(0x4A, 0x44, 0x1E),
            error_bg: c(0x5A, 0x24, 0x24),
        }
    } else {
        Pal {
            text: c(0x1F, 0x23, 0x2B),
            dim: c(0x6B, 0x74, 0x83),
            frame_bg: c(0xEE, 0xF0, 0xF4),
            frame_active: c(0xD6, 0xE6, 0xFB),
            border: c(0xA4, 0xAC, 0xB9),
            cell: c(0xFF, 0xFF, 0xFF),
            obj_cell: c(0xFF, 0xFF, 0xC6),
            obj_key: c(0xFA, 0xEB, 0xBF),
            obj_dim: c(0x7A, 0x6A, 0x2E),
            arrow: c(0x5B, 0x7F, 0xB5),
            arrow_active: c(0x12, 0x5C, 0xD6),
            changed: c(0xFF, 0xDD, 0x8A),
            changed_mark: c(0xB3, 0x5C, 0x00),
            string: c(0x1E, 0x7D, 0x32),
            number: c(0xB8, 0x5C, 0x00),
            constant: c(0x8B, 0x2F, 0xC9),
            ret: c(0xC6, 0x28, 0x28),
            def: c(0x1A, 0x5F, 0xC4),
            line_next: c(0xE0, 0x30, 0x30),
            line_prev: c(0x3F, 0xB5, 0x68),
            line_bg: c(0xFF, 0xF3, 0xB8),
            error_bg: c(0xFF, 0xD9, 0xD6),
        }
    }
}

// ---------------------------------------------------------------- captions

fn val_text(v: &Val, step: &Step) -> String {
    match v {
        Val::Prim { p, .. } => p.clone(),
        Val::Ref { r } => match step.heap.get(r) {
            Some(o) if o.kind == "instance" => format!("a {} instance", o.ty),
            Some(o) if o.kind == "class" => format!("the class {}", o.ty),
            Some(o) => format!("a {}", o.ty),
            None => "an object".to_owned(),
        },
    }
}

/// A plain-language sentence describing what the step shows.
pub fn caption(trace: &Trace, idx: usize) -> String {
    let Some(step) = trace.steps.get(idx) else {
        return String::new();
    };
    // The last step of a run that was cut short must say so: this sentence
    // is what the audience and the exported slides see.
    if idx + 1 == trace.steps.len() {
        match trace.limit.as_deref() {
            Some("time") => {
                return "Stopped here: the run reached the time limit. The program had not finished.".to_owned();
            }
            Some(_) => {
                return format!(
                    "Stopped here: the run reached the limit of {} steps. The program had not finished.",
                    crate::trace::MAX_STEPS
                );
            }
            None => {}
        }
    }
    step_caption(step)
}

fn step_caption(step: &Step) -> String {
    let place = if step.func.is_empty() {
        String::new()
    } else {
        let call = if step.func.starts_with("class ") { "" } else { "()" };
        format!(" in {}{call}", step.func)
    };
    let class_body = step.func.strip_prefix("class ");
    let exc = step.msg.as_deref().unwrap_or("exception");
    match (step.event.as_str(), class_body) {
        ("call", Some(c)) => format!("Python runs the body of class {c} to build the class object."),
        ("return", Some(c)) => format!("The body of class {c} is done: the class object is created."),
        ("call", None) => format!(
            "Calling {}(): a new frame is pushed onto the call stack for its local variables.",
            step.func
        ),
        ("return", None) => {
            let ret = step.frames.last().and_then(|f| f.ret.as_ref());
            match ret {
                Some(v) => format!(
                    "{}() returns {}. Its frame is popped off the call stack.",
                    step.func,
                    val_text(v, step)
                ),
                None => format!("{}() returns.", step.func),
            }
        }
        ("exception", _) => format!("Exception raised on line {}{}: {}", step.line, place, exc),
        ("uncaught", _) => format!("Uncaught exception, the program stops: {exc}"),
        ("end", _) => "The program has finished.".to_owned(),
        _ => format!("Line {}{} is about to run.", step.line, place),
    }
}

// --------------------------------------------------------------- code view

/// The traced program with line numbers, the current line highlighted, a red
/// arrow on the next line to run and a green one on the line that just ran.
pub fn code_view(ui: &mut Ui, code: &str, trace: &Trace, step_idx: usize, scale: f32) {
    code_impl(ui, code, trace, step_idx, scale, true);
}

/// The size the code needs at `scale`, without drawing it.
pub fn code_size(ui: &mut Ui, code: &str, trace: &Trace, scale: f32) -> Vec2 {
    code_impl(ui, code, trace, 0, scale, false)
}

fn code_impl(ui: &mut Ui, code: &str, trace: &Trace, step_idx: usize, scale: f32, paint: bool) -> Vec2 {
    let dark = ui.visuals().dark_mode;
    let p = pal(dark);
    let Some(step) = trace.steps.get(step_idx) else {
        return Vec2::ZERO;
    };
    let font = FontId::monospace(15.0 * scale);
    let code = code.trim_end();
    let n = code.split('\n').count().max(1);
    let galley = ui.painter().layout_job(highlight(code, font.clone(), dark));
    let row_h = (galley.size().y / n as f32).max(1.0);
    let digits = n.to_string().len() as f32;
    let char_w = ui.painter().layout_no_wrap("0".into(), font.clone(), p.dim).size().x;
    let arrow_w = row_h * 1.1;
    let gutter = arrow_w + digits * char_w + char_w * 1.5;
    let natural = vec2(gutter + galley.size().x + 12.0, n as f32 * row_h + 8.0);
    if !paint {
        return natural;
    }
    let size = vec2(natural.x.max(ui.available_width()), natural.y);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    let line_rect = |line: usize| {
        let y = rect.top() + 4.0 + (line.saturating_sub(1)) as f32 * row_h;
        Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), row_h))
    };

    let finished = step.event == "end";
    let is_error = step.event == "exception" || step.event == "uncaught";
    let cur = step.line;
    let prev = step_idx.checked_sub(1).map(|i| trace.steps[i].line);

    if (1..=n).contains(&cur) {
        let bg = if is_error { p.error_bg } else { p.line_bg };
        painter.rect_filled(line_rect(cur), 3.0, bg);
    }
    for i in 1..=n {
        let r = line_rect(i);
        painter.text(
            pos2(r.left() + arrow_w + digits * char_w, r.top()),
            Align2::RIGHT_TOP,
            i.to_string(),
            font.clone(),
            if i == cur { p.text } else { p.dim },
        );
    }
    painter.galley(pos2(rect.left() + gutter, rect.top() + 4.0), galley, p.text);

    let arrow = |line: usize, color: Color32, solid: bool| {
        if (1..=n).contains(&line) {
            let r = line_rect(line);
            line_arrow(&painter, pos2(r.left() + 2.0, r.center().y), arrow_w * 0.72, row_h * 0.26, color, solid);
        }
    };
    if finished {
        arrow(cur, p.line_prev, false);
    } else {
        if let Some(pl) = prev {
            if pl != cur {
                arrow(pl, p.line_prev, false);
            }
        }
        arrow(cur, p.line_next, true);
    }

    // Keep the current line in view, but only when the step changes so the
    // viewer can still scroll around freely.
    let key = ui.scope_id().with("code_view_step");
    let last: Option<usize> = ui.data(|d| d.get_temp(key));
    if last != Some(step_idx) {
        ui.data_mut(|d| d.insert_temp(key, step_idx));
        if (1..=n).contains(&cur) {
            ui.scroll_to_rect(line_rect(cur).expand2(vec2(0.0, row_h * 2.0)), None);
        }
    }
    natural
}

/// An arrow pointing right from `left`. Solid marks the next line to run;
/// outlined marks the line that just ran, so the two differ without colour.
fn line_arrow(painter: &Painter, left: Pos2, len: f32, half: f32, color: Color32, solid: bool) {
    let (x0, x1, cy) = (left.x, left.x + len, left.y);
    let xm = x1 - half * 1.3;
    let shaft = half * 0.4;
    if solid {
        painter.rect_filled(Rect::from_min_max(pos2(x0, cy - shaft), pos2(xm + 0.5, cy + shaft)), 0.0, color);
        painter.add(Shape::convex_polygon(
            vec![pos2(xm, cy - half), pos2(x1, cy), pos2(xm, cy + half)],
            color,
            Stroke::NONE,
        ));
    } else {
        painter.add(Shape::closed_line(
            vec![
                pos2(x0, cy - shaft),
                pos2(xm, cy - shaft),
                pos2(xm, cy - half),
                pos2(x1, cy),
                pos2(xm, cy + half),
                pos2(xm, cy + shaft),
                pos2(x0, cy + shaft),
            ],
            Stroke::new((half * 0.3).max(1.5), color),
        ));
    }
}

/// The small triangle in the corner of a value that just changed.
fn corner_mark(painter: &Painter, corner: Pos2, size: f32, color: Color32) {
    painter.add(Shape::convex_polygon(
        vec![corner, corner + vec2(0.0, size), corner - vec2(size, 0.0)],
        color,
        Stroke::NONE,
    ));
}

/// Explains the three marks used in the picture.
pub fn legend(ui: &mut Ui, scale: f32) {
    let p = pal(ui.visuals().dark_mode);
    let size = 11.5 * scale.min(1.4);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (kind, text) in [(0, "just ran"), (1, "runs next"), (2, "just changed")] {
            let (r, _) = ui.allocate_exact_size(vec2(size * 1.5, size), Sense::hover());
            let painter = ui.painter();
            match kind {
                0 => line_arrow(painter, r.left_center(), r.width(), size * 0.36, p.line_prev, false),
                1 => line_arrow(painter, r.left_center(), r.width(), size * 0.36, p.line_next, true),
                _ => {
                    let cell = r.shrink2(vec2(size * 0.1, 0.0));
                    painter.rect(cell, 2.0, p.changed, Stroke::new(1.0, p.border), StrokeKind::Middle);
                    corner_mark(painter, cell.right_top(), size * 0.5, p.changed_mark);
                }
            }
            ui.label(egui::RichText::new(text).size(size).color(p.dim));
            ui.add_space(8.0);
        }
    });
}

// ------------------------------------------------------------- memory view

#[derive(Clone)]
enum Item {
    Rect { r: Rect, fill: Color32, stroke: Stroke, round: f32 },
    Text { pos: Pos2, g: Arc<Galley> },
    Dot { c: Pos2, to: u64, active: bool },
    /// The mark on a value that just changed.
    Corner { at: Pos2, size: f32 },
}

#[derive(Default)]
struct Block {
    size: Vec2,
    items: Vec<Item>,
    /// Height of the type label above the object's cells.
    body_top: f32,
}

impl Block {
    fn rect(&mut self, r: Rect, fill: Color32, stroke: Stroke, round: f32) {
        self.items.push(Item::Rect { r, fill, stroke, round });
    }
    fn text(&mut self, pos: Pos2, g: Arc<Galley>) {
        self.items.push(Item::Text { pos, g });
    }
    /// A cell, drawn as changed (fill plus corner mark) or plain.
    fn cell(&mut self, r: Rect, fill: Color32, stroke: Stroke, round: f32, changed: Option<(Color32, f32)>) {
        self.rect(r, changed.map_or(fill, |c| c.0), stroke, round);
        if let Some((_, size)) = changed {
            self.items.push(Item::Corner { at: r.right_top(), size });
        }
    }
    fn add(&mut self, other: Block, off: Vec2) {
        for it in other.items {
            self.items.push(match it {
                Item::Rect { r, fill, stroke, round } => Item::Rect { r: r.translate(off), fill, stroke, round },
                Item::Text { pos, g } => Item::Text { pos: pos + off, g },
                Item::Dot { c, to, active } => Item::Dot { c: c + off, to, active },
                Item::Corner { at, size } => Item::Corner { at: at + off, size },
            });
        }
    }
}

struct Builder<'a> {
    painter: &'a Painter,
    p: Pal,
    fs: f32,
    prev: Option<&'a Step>,
    heap: &'a HashMap<u64, Obj>,
}

impl Builder<'_> {
    fn mono(&self, s: &str, color: Color32) -> Arc<Galley> {
        self.painter.layout_no_wrap(s.to_owned(), FontId::monospace(self.fs), color)
    }
    fn small(&self, s: &str, color: Color32) -> Arc<Galley> {
        self.painter.layout_no_wrap(s.to_owned(), FontId::proportional(self.fs * 0.72), color)
    }
    fn line(&self) -> Stroke {
        Stroke::new((self.fs * 0.07).max(1.0), self.p.border)
    }

    fn val(&self, v: &Val) -> Block {
        let pad = vec2(self.fs * 0.45, self.fs * 0.22);
        let mut b = Block::default();
        match v {
            Val::Prim { p, k } => {
                let color = match k.as_str() {
                    "str" => self.p.string,
                    "int" | "float" | "complex" => self.p.number,
                    "bool" | "NoneType" => self.p.constant,
                    "more" => self.p.dim,
                    _ => self.p.text,
                };
                let g = self.mono(p, color);
                b.size = g.size() + pad * 2.0;
                b.text(pad.to_pos2(), g);
            }
            Val::Ref { r } => {
                b.size = vec2(self.fs * 1.7, self.fs * 1.25) + vec2(0.0, pad.y * 2.0);
                b.items.push(Item::Dot { c: (b.size * 0.5).to_pos2(), to: *r, active: false });
            }
        }
        b
    }

    fn obj(&self, id: u64, o: &Obj) -> Block {
        // The same object one step earlier, to mark the elements that changed.
        // A brand-new object gets no marks: the variable that refers to it has one.
        let before = self.prev.and_then(|p| p.heap.get(&id)).filter(|b| b.kind == o.kind);
        let mark = |changed: bool| changed.then_some((self.p.changed, self.fs * 0.45));
        let label = match o.kind.as_str() {
            "instance" => format!("{} instance", o.ty),
            "class" if o.text.is_empty() => format!("{} class", o.ty),
            "class" => format!("{} class (inherits from {})", o.ty, o.text),
            _ => o.ty.clone(),
        };
        let lg = self.small(&label, self.p.dim);
        let top = lg.size().y + 2.0;
        let mut b = Block::default();
        b.text(pos2(0.0, 0.0), lg.clone());
        let line = self.line();
        let mut w = lg.size().x;
        let h;

        match o.kind.as_str() {
            "list" | "tuple" | "set" => {
                let indexed = o.kind != "set";
                let fill = if o.kind == "tuple" { self.p.obj_key } else { self.p.obj_cell };
                let mut cells: Vec<(Option<Arc<Galley>>, Block, bool)> = o
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let changed = before.is_some_and(|b| {
                            if indexed { b.items.get(i) != Some(v) } else { !b.items.contains(v) }
                        });
                        (indexed.then(|| self.small(&i.to_string(), self.p.obj_dim)), self.val(v), changed)
                    })
                    .collect();
                if o.more > 0 {
                    let more = Val::Prim { p: format!("… +{} more", o.more), k: "more".into() };
                    cells.push((None, self.val(&more), false));
                }
                if cells.is_empty() {
                    let more = Val::Prim { p: format!("empty {}", o.kind), k: "more".into() };
                    cells.push((None, self.val(&more), false));
                }
                let idx_h = if indexed { self.fs * 0.8 } else { 0.0 };
                let cell_h = cells.iter().map(|c| c.1.size.y).fold(0.0, f32::max) + idx_h;
                let mut x = 0.0;
                for (idx, vb, changed) in cells {
                    let cw = vb.size.x.max(self.fs * 1.7);
                    let r = Rect::from_min_size(pos2(x, top), vec2(cw, cell_h));
                    b.cell(r, fill, line, 0.0, mark(changed));
                    if let Some(g) = idx {
                        b.text(pos2(x + self.fs * 0.25, top + 1.0), g);
                    }
                    let off = vec2(x + (cw - vb.size.x) * 0.5, top + idx_h);
                    b.add(vb, off);
                    x += cw;
                }
                w = w.max(x);
                h = top + cell_h;
            }
            "dict" | "instance" | "class" => {
                let rows: Vec<(Block, Block, bool)> = o
                    .pairs
                    .iter()
                    .map(|(k, v)| {
                        let changed = before
                            .is_some_and(|b| b.pairs.iter().find(|(bk, _)| bk == k).is_none_or(|(_, bv)| bv != v));
                        (self.val(k), self.val(v), changed)
                    })
                    .collect();
                if rows.is_empty() {
                    let text = if o.kind == "dict" { "empty dict" } else { "no attributes" };
                    let vb = self.val(&Val::Prim { p: text.into(), k: "more".into() });
                    let r = Rect::from_min_size(pos2(0.0, top), vb.size);
                    b.rect(r, self.p.obj_cell, line, 0.0);
                    w = w.max(vb.size.x);
                    h = top + vb.size.y;
                    b.add(vb, vec2(0.0, top));
                } else {
                    let kw = rows.iter().map(|r| r.0.size.x).fold(0.0, f32::max);
                    let vw = rows.iter().map(|r| r.1.size.x).fold(self.fs * 1.7, f32::max);
                    let mut y = top;
                    for (kb, vb, changed) in rows {
                        let rh = kb.size.y.max(vb.size.y);
                        b.rect(Rect::from_min_size(pos2(0.0, y), vec2(kw, rh)), self.p.obj_key, line, 0.0);
                        b.cell(Rect::from_min_size(pos2(kw, y), vec2(vw, rh)), self.p.obj_cell, line, 0.0, mark(changed));
                        let ky = y + (rh - kb.size.y) * 0.5;
                        let vy = y + (rh - vb.size.y) * 0.5;
                        b.add(kb, vec2(0.0, ky));
                        b.add(vb, vec2(kw, vy));
                        y += rh;
                    }
                    w = w.max(kw + vw);
                    h = y;
                }
            }
            _ => {
                let g = self.mono(&o.text, self.p.text);
                let pad = vec2(self.fs * 0.45, self.fs * 0.22);
                let r = Rect::from_min_size(pos2(0.0, top), g.size() + pad * 2.0);
                b.rect(r, self.p.frame_bg, Stroke::NONE, self.fs * 0.3);
                b.text(pos2(pad.x, top + pad.y), g);
                w = w.max(r.width());
                h = r.bottom();
            }
        }
        b.size = vec2(w, h);
        b.body_top = top;
        b
    }
}

struct FrameSpec {
    title: String,
    rows: Vec<(Arc<Galley>, Block, bool)>,
    active: bool,
}

fn refs_of(o: &Obj) -> impl Iterator<Item = u64> + '_ {
    let pick = |v: &Val| match v {
        Val::Ref { r } => Some(*r),
        Val::Prim { .. } => None,
    };
    o.items
        .iter()
        .filter_map(pick)
        .chain(o.pairs.iter().flat_map(move |(k, v)| [pick(k), pick(v)].into_iter().flatten()))
}

struct HeapLayout {
    seen: HashSet<u64>,
    anchors: HashMap<u64, Rect>,
    placed: Vec<(Vec2, Block)>,
    right: f32,
}

impl HeapLayout {
    /// Places object `id` at (x, y) and its not-yet-placed children to its
    /// right. Returns the height of the whole subtree.
    fn place(&mut self, b: &Builder<'_>, id: u64, x: f32, y: f32) -> f32 {
        let Some(o) = b.heap.get(&id) else { return 0.0 };
        self.seen.insert(id);
        let block = b.obj(id, o);
        let size = block.size;
        self.anchors.insert(id, Rect::from_min_max(pos2(x, y + block.body_top), pos2(x + size.x, y + size.y)));
        self.right = self.right.max(x + size.x);
        self.placed.push((vec2(x, y), block));
        let gap = b.fs * 0.9;
        let mut cy = y;
        for kid in refs_of(o) {
            if !self.seen.contains(&kid) {
                cy += self.place(b, kid, x + size.x + b.fs * 3.0, cy) + gap;
            }
        }
        size.y.max(cy - y - gap)
    }
}

/// The call stack ("Frames") and the heap ("Objects") with the references between them.
pub fn memory_view(ui: &mut Ui, trace: &Trace, step_idx: usize, scale: f32) {
    memory_impl(ui, trace, step_idx, scale, true);
}

/// The size the call stack and heap need at `scale`, without drawing them.
pub fn memory_size(ui: &mut Ui, trace: &Trace, step_idx: usize, scale: f32) -> Vec2 {
    memory_impl(ui, trace, step_idx, scale, false)
}

fn memory_impl(ui: &mut Ui, trace: &Trace, step_idx: usize, scale: f32, paint: bool) -> Vec2 {
    let Some(step) = trace.steps.get(step_idx) else {
        return Vec2::ZERO;
    };
    let prev = step_idx.checked_sub(1).map(|i| &trace.steps[i]);
    let fs = 15.0 * scale;
    let b = Builder { painter: ui.painter(), p: pal(ui.visuals().dark_mode), fs, prev, heap: &step.heap };
    let p = &b.p;

    // ---- frames
    let mut roots: Vec<u64> = Vec::new();
    let mut specs: Vec<FrameSpec> = Vec::new();
    let mut make = |title: String, vars: &[(String, Val)], ret: Option<&Val>, old: Option<&[(String, Val)]>, is_new: bool| {
        let mut rows = Vec::new();
        for (name, v) in vars {
            let was = old.and_then(|o| o.iter().find(|(n, _)| n == name)).map(|(_, v)| v);
            let changed = prev.is_some() && (is_new || was != Some(v));
            if let Val::Ref { r } = v {
                roots.push(*r);
            }
            rows.push((b.mono(name, p.text), b.val(v), changed));
        }
        if let Some(v) = ret {
            if let Val::Ref { r } = v {
                roots.push(*r);
            }
            rows.push((b.mono("Return value", p.ret), b.val(v), true));
        }
        specs.push(FrameSpec { title, rows, active: false });
    };
    make("Global frame".into(), &step.globals, None, prev.map(|s| s.globals.as_slice()), false);
    for f in &step.frames {
        let old = prev.and_then(|s| s.frames.iter().find(|o| o.id == f.id));
        make(f.name.clone(), &f.locals, f.ret.as_ref(), old.map(|o| o.locals.as_slice()), old.is_none());
    }
    if let Some(last) = specs.last_mut() {
        last.active = true;
    }

    let pad = fs * 0.6;
    let gap = fs * 0.5;
    // A frame is titled like the call it belongs to, `name()`, in the colour
    // function names have in the code, so it does not read as one more variable.
    let titles: Vec<Arc<Galley>> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let is_call = i > 0 && s.title.chars().all(|c| c.is_alphanumeric() || c == '_');
            match (i, is_call) {
                (0, _) => b.mono(&s.title, p.text),
                (_, true) => b.mono(&format!("{}()", s.title), p.def),
                _ => b.mono(&s.title, p.def),
            }
        })
        .collect();
    let name_w = specs.iter().flat_map(|s| &s.rows).map(|r| r.0.size().x).fold(0.0, f32::max);
    let val_w = specs.iter().flat_map(|s| &s.rows).map(|r| r.1.size.x).fold(fs * 2.2, f32::max);
    let title_w = titles.iter().map(|t| t.size().x).fold(0.0, f32::max);
    let frame_w = (name_w + gap + val_w).max(title_w).max(fs * 9.0) + pad * 2.0;
    let val_x = frame_w - pad - val_w;

    let header_h = fs * 1.6;
    let mut running_frame = Rect::NOTHING; // the last frame placed
    let mut placed: Vec<(Vec2, Block)> = Vec::new(); // frames, then objects
    let mut y = header_h;
    for (spec, title) in specs.into_iter().zip(titles) {
        let mut fb = Block::default();
        let mut fy = pad * 0.7;
        let title_h = title.size().y;
        fb.text(pos2(pad, fy), title);
        fy += title_h + fs * 0.3;
        if !spec.rows.is_empty() {
            // A rule between the title and the variables.
            let rule = Rect::from_min_size(pos2(pad * 0.6, fy), vec2(frame_w - pad * 1.2, 1.0));
            fb.rect(rule, p.border.gamma_multiply(0.55), Stroke::NONE, 0.0);
            fy += fs * 0.4;
        }
        let mut rows = Block::default();
        for (name, mut vb, changed) in spec.rows {
            let rh = name.size().y.max(vb.size.y) + fs * 0.2;
            let cell = Rect::from_min_size(pos2(val_x, fy), vec2(val_w, rh));
            rows.cell(cell, p.cell, b.line(), 2.0, changed.then_some((p.changed, fs * 0.45)));
            rows.text(pos2(val_x - gap - name.size().x, fy + (rh - name.size().y) * 0.5), name);
            for it in &mut vb.items {
                if let Item::Dot { active, .. } = it {
                    *active = spec.active;
                }
            }
            let off = vec2(val_x, fy + (rh - vb.size.y) * 0.5);
            rows.add(vb, off);
            fy += rh + fs * 0.25;
        }
        fb.size = vec2(frame_w, fy + pad * 0.5);
        let bg = Rect::from_min_size(Pos2::ZERO, fb.size);
        let (fill, stroke) = if spec.active {
            (p.frame_active, Stroke::new(1.5, p.arrow_active))
        } else {
            (p.frame_bg, Stroke::NONE)
        };
        fb.items.insert(0, Item::Rect { r: bg, fill, stroke, round: fs * 0.35 });
        fb.add(rows, Vec2::ZERO);
        let h = fb.size.y;
        running_frame = Rect::from_min_size(pos2(0.0, y), fb.size);
        placed.push((vec2(0.0, y), fb));
        y += h + fs * 0.7;
    }
    let frames_bottom = y;

    // ---- heap: each object a variable refers to starts a row; the objects it
    // refers to are stacked in a column to its right, like a tree.
    let objects_x = frame_w + fs * 4.0;
    let mut lay = HeapLayout {
        seen: HashSet::new(),
        anchors: HashMap::new(),
        placed: Vec::new(),
        right: objects_x + fs * 6.0,
    };
    let mut oy = header_h;
    for root in roots {
        if !lay.seen.contains(&root) {
            oy += lay.place(&b, root, objects_x, oy) + fs * 1.3;
        }
    }
    let HeapLayout { anchors, placed: objects, right, .. } = lay;
    placed.extend(objects);

    // ---- paint
    let margin = vec2(fs, fs * 0.5);
    let natural = vec2(right, frames_bottom.max(oy)) + margin * 2.0;
    if !paint {
        return natural;
    }
    let total = natural.max(ui.available_size());
    let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
    let painter = ui.painter_at(rect);
    let origin = rect.min + margin;
    // When the picture is larger than its panel, bring the running frame into
    // view at each new step (the viewer can still scroll away from it).
    let key = ui.scope_id().with("memory_view_step");
    let last: Option<usize> = ui.data(|d| d.get_temp(key));
    if last != Some(step_idx) {
        ui.data_mut(|d| d.insert_temp(key, step_idx));
        if running_frame.is_positive() {
            ui.scroll_to_rect(running_frame.translate(origin.to_vec2()).expand(fs * 0.6), None);
        }
    }
    let head_font = FontId::proportional(fs * 0.95);
    painter.text(origin, Align2::LEFT_TOP, "Call stack", head_font.clone(), p.dim);
    painter.text(origin + vec2(objects_x, 0.0), Align2::LEFT_TOP, "Heap", head_font, p.dim);
    let sep_x = origin.x + frame_w + fs * 2.0;
    painter.line_segment(
        [pos2(sep_x, rect.top() + fs * 0.4), pos2(sep_x, rect.bottom() - fs * 0.4)],
        Stroke::new(1.0, p.border.gamma_multiply(0.35)),
    );

    let mut dots: Vec<(Pos2, u64, bool)> = Vec::new();
    for (off, block) in placed {
        let o = origin.to_vec2() + off;
        for it in block.items {
            match it {
                Item::Rect { r, fill, stroke, round } => {
                    painter.rect(r.translate(o), round, fill, stroke, StrokeKind::Middle);
                }
                Item::Text { pos, g } => painter.galley(pos + o, g, p.text),
                Item::Dot { c, to, active } => dots.push((c + o, to, active)),
                Item::Corner { at, size } => corner_mark(&painter, at + o, size, p.changed_mark),
            }
        }
    }
    let width = (fs * 0.11).max(1.5);
    let head = fs * 0.55;
    for (from, to, active) in dots {
        let Some(a) = anchors.get(&to) else { continue };
        let target = a.translate(origin.to_vec2());
        let color = if active { p.arrow_active } else { p.arrow };
        let stroke = Stroke::new(width, color);
        painter.circle_filled(from, fs * 0.24, color);
        // A reference that starts in a frame first leaves the frame
        // horizontally, so the curve never runs across other variables.
        let exit_x = origin.x + frame_w + fs * 0.4;
        let from = if from.x < exit_x {
            let out = pos2(exit_x, from.y);
            painter.line_segment([from, out], stroke);
            out
        } else {
            from
        };
        let (curve, tip, dir) = if from.x < target.left() - fs {
            // The object is to the right: arrive on its left edge.
            let tip = pos2(target.left() - 1.0, target.top() + (target.height() * 0.5).min(fs));
            let stop = tip - vec2(head * 0.8, 0.0);
            let d = ((stop.x - from.x) * 0.5).clamp(fs * 1.2, fs * 8.0);
            ([from, from + vec2(d, 0.0), stop - vec2(d, 0.0), stop], tip, vec2(1.0, 0.0))
        } else {
            // The object is above or below (an alias to something already
            // drawn): go straight to its nearest horizontal edge.
            let down = target.center().y > from.y;
            let (edge, sign) = if down { (target.top() - 1.0, 1.0) } else { (target.bottom() + 1.0, -1.0) };
            let x = from.x.clamp(target.left() + fs * 0.5, (target.right() - fs * 0.5).max(target.left() + fs * 0.5));
            let tip = pos2(x, edge);
            let stop = tip - vec2(0.0, sign * head * 0.8);
            let d = ((stop.y - from.y).abs() * 0.5).min(fs * 6.0);
            ([from, from + vec2(0.0, sign * d), stop - vec2(0.0, sign * d), stop], tip, vec2(0.0, sign))
        };
        painter.add(CubicBezierShape::from_points_stroke(curve, false, Color32::TRANSPARENT, stroke));
        let side = vec2(-dir.y, dir.x) * head * 0.45;
        painter.add(Shape::convex_polygon(
            vec![tip, tip - dir * head + side, tip - dir * head - side],
            color,
            Stroke::NONE,
        ));
    }
    natural
}
