//! Export of a run as one self-contained web page that can be stepped
//! through in a browser and embedded in a course.
//!
//! The page is drawn by the same code as the app: each step's slide is turned
//! from egui shapes into SVG elements. Elements that repeat from step to step
//! (most of them) are stored once, and every colour is stored as a light and a
//! dark value, so one file follows the reader's theme.

use crate::export::{slide_shapes, SLIDE_SIZE};
use crate::trace::Trace;
use crate::viz;
use eframe::egui::{
    self,
    epaint::{ClippedShape, ColorMode, PathStroke, Shape, StrokeKind, TextShape},
    Color32, FontFamily, Pos2, Rect, Stroke,
};
use std::collections::HashMap;
use std::fmt::Write as _;

const TEMPLATE: &str = include_str!("player.html");

/// The complete HTML page for a run.
pub fn page(title: &str, code: &str, trace: &Trace) -> String {
    let light = slide_shapes(code, trace, false);
    let dark = slide_shapes(code, trace, true);

    let mut clips: Vec<Rect> = Vec::new();
    let mut colors = Colors::default();
    let mut dict: Vec<String> = Vec::new();
    let mut index: HashMap<String, u32> = HashMap::new();
    let mut steps: Vec<Vec<u32>> = Vec::new();

    for (l, d) in light.iter().zip(&dark) {
        let l = pieces(l, &mut clips);
        let d = pieces(d, &mut clips);
        // The two themes draw the same things in different colours. If they
        // ever differ in structure, the light colours are used for both.
        let same = l.len() == d.len() && l.iter().zip(&d).all(|(a, b)| a.parts == b.parts);
        let mut step = Vec::with_capacity(l.len());
        for (i, piece) in l.iter().enumerate() {
            let mut svg = String::new();
            for (k, part) in piece.parts.iter().enumerate() {
                svg += part;
                if let Some(c) = piece.colors.get(k) {
                    let other = if same { d[i].colors[k] } else { *c };
                    let _ = write!(svg, "{}", colors.id(*c, other));
                }
            }
            let next = dict.len() as u32;
            let id = *index.entry(svg.clone()).or_insert(next);
            if id == next {
                dict.push(svg);
            }
            step.push(id);
        }
        steps.push(step);
    }

    let captions: Vec<String> = (0..trace.steps.len()).map(|i| viz::caption(trace, i)).collect();
    let data = serde_json::json!({
        "w": SLIDE_SIZE.x,
        "h": SLIDE_SIZE.y,
        "title": title,
        "code": code,
        "captions": captions,
        "light": colors.light,
        "dark": colors.dark,
        "bg": [css(egui::Visuals::light().panel_fill), css(egui::Visuals::dark().panel_fill)],
        "clips": clips.iter().map(|r| [r.left(), r.top(), r.width(), r.height()]).collect::<Vec<_>>(),
        "dict": dict,
        "steps": steps,
    });
    // "<" only occurs inside JSON strings; escaped, the data cannot close its script tag.
    let data = data.to_string().replace('<', "\\u003c");
    TEMPLATE.replace("__TITLE__", &escape(title)).replace("__DATA__", &data)
}

/// Pairs of (light, dark) colours, each given a number used in class names.
#[derive(Default)]
struct Colors {
    ids: HashMap<(Color32, Color32), usize>,
    light: Vec<String>,
    dark: Vec<String>,
}

impl Colors {
    fn id(&mut self, light: Color32, dark: Color32) -> usize {
        let next = self.light.len();
        let id = *self.ids.entry((light, dark)).or_insert(next);
        if id == next {
            self.light.push(css(light));
            self.dark.push(css(dark));
        }
        id
    }
}

fn css(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// An SVG element with a hole wherever one of its colours goes: the text is
/// `parts[0]`, colour 0, `parts[1]`, colour 1, … and a last part.
struct Piece {
    parts: Vec<String>,
    colors: Vec<Color32>,
}

struct Builder {
    parts: Vec<String>,
    colors: Vec<Color32>,
    text: String,
}

impl Builder {
    fn new(open: &str) -> Self {
        Self { parts: Vec::new(), colors: Vec::new(), text: open.to_owned() }
    }
    /// Adds the class that gives the element a fill (`f`) or stroke (`s`) colour.
    fn color(&mut self, kind: char, c: Color32) {
        self.text.push(' ');
        self.text.push(kind);
        self.parts.push(std::mem::take(&mut self.text));
        self.colors.push(c);
    }
    fn push(&mut self, s: &str) {
        self.text += s;
    }
    fn finish(mut self) -> Piece {
        self.parts.push(self.text);
        Piece { parts: self.parts, colors: self.colors }
    }
}

/// A number as short as it can be written.
fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".to_owned() } else { s.to_owned() }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn visible(c: Color32) -> bool {
    c.a() > 0
}

/// The elements of one slide, in painting order.
fn pieces(shapes: &[ClippedShape], clips: &mut Vec<Rect>) -> Vec<Piece> {
    let mut out = Vec::new();
    for clipped in shapes {
        // A clip that covers the whole slide clips nothing.
        let r = clipped.clip_rect;
        let whole = r.left() <= 0.0 && r.top() <= 0.0 && r.right() >= SLIDE_SIZE.x && r.bottom() >= SLIDE_SIZE.y;
        let clip = if whole {
            String::new()
        } else {
            let same = |c: &Rect| (c.min - r.min).length() < 0.5 && (c.max - r.max).length() < 0.5;
            let id = clips.iter().position(same).unwrap_or_else(|| {
                clips.push(r);
                clips.len() - 1
            });
            format!(" clip-path=\"url(#k{id})\"")
        };
        shape(&clipped.shape, &clip, &mut out);
    }
    out
}

fn stroke_of(stroke: &PathStroke) -> Option<(f32, Color32)> {
    match &stroke.color {
        ColorMode::Solid(c) if stroke.width > 0.0 && visible(*c) => Some((stroke.width, *c)),
        _ => None,
    }
}

fn plain_stroke(stroke: &Stroke) -> Option<(f32, Color32)> {
    (stroke.width > 0.0 && visible(stroke.color)).then_some((stroke.width, stroke.color))
}

/// Opens an element with its fill and stroke classes; the caller adds the
/// geometry and closes it.
fn open(tag: &str, fill: Option<Color32>, stroke: Option<(f32, Color32)>) -> Builder {
    let mut b = Builder::new(&format!("<{tag} class=\""));
    if let Some(c) = fill {
        b.color('f', c);
    }
    if let Some((_, c)) = stroke {
        b.color('s', c);
    }
    b.push("\"");
    if let Some((w, _)) = stroke {
        b.push(&format!(" stroke-width=\"{}\"", num(w)));
    }
    b
}

fn shape(shape: &Shape, clip: &str, out: &mut Vec<Piece>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| self::shape(s, clip, out)),
        Shape::Rect(r) => {
            let fill = visible(r.fill).then_some(r.fill);
            let stroke = plain_stroke(&r.stroke);
            if fill.is_none() && stroke.is_none() {
                return;
            }
            let w = stroke.map_or(0.0, |s| s.0);
            let rect = match r.stroke_kind {
                StrokeKind::Inside => r.rect.shrink(w / 2.0),
                StrokeKind::Outside => r.rect.expand(w / 2.0),
                StrokeKind::Middle => r.rect,
            };
            let mut b = open("rect", fill, stroke);
            b.push(&format!(
                " x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                num(rect.left()),
                num(rect.top()),
                num(rect.width()),
                num(rect.height())
            ));
            if r.corner_radius.nw > 0 {
                b.push(&format!(" rx=\"{}\"", r.corner_radius.nw));
            }
            b.push(clip);
            b.push("/>");
            out.push(b.finish());
        }
        Shape::Circle(c) => {
            let fill = visible(c.fill).then_some(c.fill);
            let stroke = plain_stroke(&c.stroke);
            if fill.is_none() && stroke.is_none() {
                return;
            }
            let mut b = open("circle", fill, stroke);
            b.push(&format!(" cx=\"{}\" cy=\"{}\" r=\"{}\"", num(c.center.x), num(c.center.y), num(c.radius)));
            b.push(clip);
            b.push("/>");
            out.push(b.finish());
        }
        Shape::LineSegment { points, stroke } => {
            let Some(stroke) = plain_stroke(stroke) else { return };
            let mut b = open("path", None, Some(stroke));
            b.push(&format!(" d=\"{}\"", path(points, false)));
            b.push(clip);
            b.push("/>");
            out.push(b.finish());
        }
        Shape::Path(p) => {
            let fill = (p.closed && visible(p.fill)).then_some(p.fill);
            let stroke = stroke_of(&p.stroke);
            if p.points.len() < 2 || (fill.is_none() && stroke.is_none()) {
                return;
            }
            let mut b = open("path", fill, stroke);
            b.push(&format!(" d=\"{}\"", path(&p.points, p.closed)));
            b.push(clip);
            b.push("/>");
            out.push(b.finish());
        }
        Shape::CubicBezier(c) => {
            let Some(stroke) = stroke_of(&c.stroke) else { return };
            let [a, b1, b2, e] = c.points;
            let mut b = open("path", None, Some(stroke));
            b.push(&format!(
                " d=\"M{} {}C{} {} {} {} {} {}\"",
                num(a.x),
                num(a.y),
                num(b1.x),
                num(b1.y),
                num(b2.x),
                num(b2.y),
                num(e.x),
                num(e.y)
            ));
            b.push(clip);
            b.push("/>");
            out.push(b.finish());
        }
        Shape::Text(t) => text(t, clip, out),
        // Nothing else is drawn by a slide.
        _ => {}
    }
}

fn path(points: &[Pos2], closed: bool) -> String {
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { "L" }, num(p.x), num(p.y));
    }
    if closed {
        d.push('Z');
    }
    d
}

/// Text becomes one `<text>` per run of one colour and font, placed where
/// egui placed it. Each run is given the width egui measured, so the layout
/// holds whichever font the browser has.
fn text(t: &TextShape, clip: &str, out: &mut Vec<Piece>) {
    let job = &t.galley.job;
    // Which section (colour and font) each character of the text belongs to.
    let mut section_of: Vec<usize> = Vec::with_capacity(job.text.len());
    let mut s = 0;
    for (byte, _) in job.text.char_indices() {
        while s + 1 < job.sections.len() && byte >= job.sections[s].byte_range.end.0 {
            s += 1;
        }
        section_of.push(s);
    }
    let chars: Vec<char> = job.text.chars().collect();

    let mut at = 0; // index of the next character
    for row in &t.galley.rows {
        let glyphs = &row.row.glyphs;
        let origin = t.pos + row.pos.to_vec2();
        let mut i = 0;
        while i < glyphs.len() {
            // Glyphs follow the characters of the text; if they ever do not,
            // the whole row is drawn in its first style.
            let aligned = |k: usize| chars.get(at + k) == Some(&glyphs[k].chr);
            let section = |k: usize| if aligned(k) { section_of[at + k] } else { 0 };
            let sec = section(i);
            let mut j = i;
            while j < glyphs.len() && section(j) == sec {
                j += 1;
            }
            // Spaces at the ends are left out; the run starts at its first ink.
            let (mut a, mut b) = (i, j);
            while a < b && glyphs[a].chr.is_whitespace() {
                a += 1;
            }
            while b > a && glyphs[b - 1].chr.is_whitespace() {
                b -= 1;
            }
            if a < b {
                let Some(format) = job.sections.get(sec).map(|s| &s.format) else { break };
                let mut color = t.override_text_color.unwrap_or(format.color);
                if color == Color32::PLACEHOLDER {
                    color = t.fallback_color;
                }
                let first = &glyphs[a];
                let last = &glyphs[b - 1];
                let content: String = glyphs[a..b].iter().map(|g| g.chr).collect();
                let family = match format.font_id.family {
                    FontFamily::Monospace => "m",
                    _ => "p",
                };
                let mut e = Builder::new(&format!("<text class=\"{family}"));
                e.color('f', color);
                e.push(&format!(
                    "\" x=\"{}\" y=\"{}\" font-size=\"{}\"",
                    num(origin.x + first.pos.x),
                    num(origin.y + first.pos.y),
                    num(format.font_id.size)
                ));
                if b - a > 1 {
                    e.push(&format!(" textLength=\"{}\"", num(last.max_x() - first.pos.x)));
                }
                if format.italics {
                    e.push(" font-style=\"italic\"");
                }
                e.push(clip);
                e.push(">");
                e.push(&escape(&content));
                e.push("</text>");
                out.push(e.finish());
            }
            i = j;
        }
        at += glyphs.len() + usize::from(row.ends_with_newline);
    }
}
