//! A small Python syntax highlighter producing an egui `LayoutJob`.

use eframe::egui::{text::LayoutJob, Color32, FontId, TextFormat};

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "match", "case",
];
const BUILTINS: &[&str] = &[
    "print", "len", "range", "input", "int", "float", "str", "bool", "list", "dict", "set",
    "tuple", "sum", "min", "max", "abs", "sorted", "enumerate", "zip", "map", "filter", "open",
    "type", "isinstance", "round", "reversed", "any", "all", "super", "self",
];

struct Colors {
    plain: Color32,
    keyword: Color32,
    string: Color32,
    comment: Color32,
    number: Color32,
    def: Color32,
    builtin: Color32,
}

fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            plain: Color32::from_rgb(0xDD, 0xE1, 0xE8),
            keyword: Color32::from_rgb(0xC7, 0x92, 0xEA),
            string: Color32::from_rgb(0xA5, 0xD6, 0x8A),
            comment: Color32::from_rgb(0x7A, 0x84, 0x94),
            number: Color32::from_rgb(0xF2, 0xA8, 0x6B),
            def: Color32::from_rgb(0x7F, 0xB4, 0xF7),
            builtin: Color32::from_rgb(0x6C, 0xCF, 0xC6),
        }
    } else {
        Colors {
            plain: Color32::from_rgb(0x1F, 0x23, 0x2B),
            keyword: Color32::from_rgb(0x8B, 0x2F, 0xC9),
            string: Color32::from_rgb(0x1E, 0x7D, 0x32),
            comment: Color32::from_rgb(0x7C, 0x85, 0x93),
            number: Color32::from_rgb(0xB8, 0x5C, 0x00),
            def: Color32::from_rgb(0x1A, 0x5F, 0xC4),
            builtin: Color32::from_rgb(0x0E, 0x7C, 0x86),
        }
    }
}

pub fn highlight(code: &str, font: FontId, dark: bool) -> LayoutJob {
    let c = colors(dark);
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let push = |job: &mut LayoutJob, s: &str, color: Color32| {
        if !s.is_empty() {
            job.append(s, 0.0, TextFormat::simple(font.clone(), color));
        }
    };

    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut after_def = false;
    // Start of a run of plain text (flushed lazily so multi-byte characters stay intact).
    let mut plain_start = 0;

    while i < n {
        let ch = b[i];
        let start = i;
        let (end, color) = if ch == b'#' {
            let e = code[i..].find('\n').map_or(n, |p| i + p);
            (e, c.comment)
        } else if ch == b'"' || ch == b'\'' {
            (string_end(b, i), c.string)
        } else if ch.is_ascii_digit() {
            let mut e = i;
            while e < n && (b[e].is_ascii_alphanumeric() || b[e] == b'.' || b[e] == b'_') {
                e += 1;
            }
            (e, c.number)
        } else if ch.is_ascii_alphabetic() || ch == b'_' {
            let mut e = i;
            while e < n && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
                e += 1;
            }
            let word = &code[i..e];
            let is_prefix = word.len() <= 2
                && word.bytes().all(|x| b"rbfuRBFU".contains(&x))
                && e < n
                && (b[e] == b'"' || b[e] == b'\'');
            if is_prefix {
                (string_end(b, e), c.string)
            } else if after_def {
                (e, c.def)
            } else if KEYWORDS.contains(&word) {
                (e, c.keyword)
            } else if BUILTINS.contains(&word) {
                (e, c.builtin)
            } else {
                (e, c.plain)
            }
        } else {
            // Punctuation, whitespace or a non-ASCII byte: part of a plain run.
            i += 1;
            continue;
        };
        push(&mut job, &code[plain_start..start], c.plain);
        push(&mut job, &code[start..end], color);
        let word = &code[start..end];
        after_def = word == "def" || word == "class";
        i = end;
        plain_start = end;
    }
    push(&mut job, &code[plain_start..], c.plain);
    job
}

/// Index just past the string literal whose opening quote is at `i`.
fn string_end(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let q = b[i];
    let triple = i + 2 < n && b[i + 1] == q && b[i + 2] == q;
    let mut e = if triple { i + 3 } else { i + 1 };
    while e < n {
        if b[e] == b'\\' {
            e += 2;
            continue;
        }
        if triple {
            if b[e] == q && e + 2 < n && b[e + 1] == q && b[e + 2] == q {
                return e + 3;
            }
        } else if b[e] == q {
            return e + 1;
        } else if b[e] == b'\n' {
            return e;
        }
        e += 1;
    }
    n
}
