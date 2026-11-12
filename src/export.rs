//! Export of steps as pictures: one PNG, a PNG per step, a PDF or a PowerPoint file.
//!
//! Slides are drawn off-screen with a private egui context and a small software
//! rasterizer, so the result does not depend on the window size and never
//! contains the on-screen controls.

use crate::trace::Trace;
use crate::viz;
use eframe::egui::{
    self, epaint::Primitive, pos2, vec2, Align2, Color32, FontId, Rect, RichText, TextureId,
    UiBuilder, Vec2,
};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Slide size in points (16:9) and the pixel density it is rendered at.
const SLIDE: Vec2 = vec2(1280.0, 720.0);
const PPP: f32 = 1.5;
const PAD: f32 = 28.0;
const GAP: f32 = 28.0;
const MAX_OUTPUT_LINES: usize = 6;

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    /// The current step as one PNG file.
    Png,
    /// Every step as a numbered PNG file in a folder.
    PngFolder,
    Pdf,
    Pptx,
    /// Every step in one self-contained web page that can be stepped through.
    Html,
}

pub struct Job {
    /// A name for the program, used as the title of a web page.
    pub title: String,
    pub code: String,
    pub trace: Trace,
    pub step: usize,
    pub dark: bool,
    pub format: Format,
    pub path: PathBuf,
}

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

/// Runs an export job. Returns a sentence for the status bar.
pub fn run(job: Job) -> Result<String, String> {
    let all: Vec<usize> = (0..job.trace.steps.len()).collect();
    let io = |e: std::io::Error| format!("Could not write {}: {e}", job.path.display());
    if job.format == Format::Html {
        let page = crate::web::page(&job.title, &job.code, &job.trace);
        std::fs::write(&job.path, page).map_err(io)?;
        return Ok(format!("{} steps exported to {}", all.len(), job.path.display()));
    }
    let steps: &[usize] = if job.format == Format::Png { &[job.step] } else { &all };
    let images = render(&job.code, &job.trace, steps, job.dark);
    match job.format {
        Format::Png => std::fs::write(&job.path, png(&images[0])?).map_err(io)?,
        Format::PngFolder => {
            for (i, img) in images.iter().enumerate() {
                let file = job.path.join(format!("step-{:03}.png", i + 1));
                std::fs::write(file, png(img)?).map_err(io)?;
            }
        }
        Format::Pdf => std::fs::write(&job.path, pdf(&images).map_err(io)?).map_err(io)?,
        Format::Pptx => pptx(&job.path, &images).map_err(|e| format!("Could not write {}: {e}", job.path.display()))?,
        Format::Html => unreachable!("written above"),
    }
    let what = match (job.format, images.len()) {
        (Format::Png, _) => "Step".to_owned(),
        (_, n) => format!("{n} steps"),
    };
    Ok(format!("{what} exported to {}", job.path.display()))
}

// ------------------------------------------------------------------ drawing

struct Fit {
    scale: f32,
    left_w: f32,
    output_lines: usize,
    /// Draw "Step 3 of 20" in the corner.
    counter: bool,
}

fn input() -> egui::RawInput {
    let mut raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), SLIDE)),
        ..Default::default()
    };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(PPP);
    raw
}

/// Text height reserved under the code, at scale 1: legend, sentence and output.
fn below_code(output_lines: usize) -> f32 {
    let output = if output_lines == 0 { 0.0 } else { 24.0 + output_lines as f32 * 19.0 };
    26.0 + 15.5 * 2.7 + output + 12.0
}

/// What the slide drawing hands back: font-texture updates, and each finished slide.
enum Pass<'a> {
    Textures(&'a egui::TexturesDelta),
    Slide(&'a egui::Context, Vec<egui::epaint::ClippedShape>),
}

/// Draws `steps` as slides with a private egui context, all at the same scale
/// so that consecutive slides line up. `counter` adds "Step 3 of 20" in a corner.
fn draw_slides(code: &str, trace: &Trace, steps: &[usize], dark: bool, counter: bool, each: &mut dyn FnMut(Pass<'_>)) {
    let ctx = egui::Context::default();
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });

    // Measure at scale 1, then pick the largest scale at which every step fits.
    let mut code_size = Vec2::ZERO;
    let mut mem_size = Vec2::ZERO;
    for _ in 0..2 {
        let out = ctx.run_ui(input(), |ui| {
            code_size = viz::code_size(ui, code, trace, 1.0);
            mem_size = steps
                .iter()
                .map(|&i| viz::memory_size(ui, trace, i, 1.0))
                .fold(Vec2::ZERO, Vec2::max);
        });
        each(Pass::Textures(&out.textures_delta));
    }
    let output_lines = steps
        .iter()
        .map(|&i| trace.output_at(i).lines().count())
        .max()
        .unwrap_or(0)
        .min(MAX_OUTPUT_LINES);
    let inner = SLIDE - vec2(PAD * 2.0 + GAP, PAD * 2.0);
    let left_frac = (code_size.x / (code_size.x + mem_size.x).max(1.0)).clamp(0.28, 0.5);
    let left_w = inner.x * left_frac;
    let scale = (left_w / code_size.x.max(1.0))
        .min(inner.y / (code_size.y + below_code(output_lines)))
        .min((inner.x - left_w) / mem_size.x.max(1.0))
        .min(inner.y / mem_size.y.max(1.0));
    let fit = Fit { scale: (scale * 0.97).clamp(0.3, 1.8), left_w, output_lines, counter };

    for &step in steps {
        // The first pass settles the layout; the second is the one we keep.
        let mut out = ctx.run_ui(input(), |ui| slide(ui, code, trace, step, &fit));
        each(Pass::Textures(&out.textures_delta));
        out = ctx.run_ui(input(), |ui| slide(ui, code, trace, step, &fit));
        each(Pass::Textures(&out.textures_delta));
        each(Pass::Slide(&ctx, out.shapes));
    }
}

/// Draws `steps` as pictures.
pub fn render(code: &str, trace: &Trace, steps: &[usize], dark: bool) -> Vec<Image> {
    let mut raster = Raster::new();
    let mut images = Vec::new();
    draw_slides(code, trace, steps, dark, true, &mut |pass| match pass {
        Pass::Textures(delta) => raster.update_textures(delta),
        Pass::Slide(ctx, shapes) => {
            let prims = ctx.tessellate(shapes, PPP);
            images.push(raster.draw(&prims, ctx.global_style().visuals.panel_fill));
        }
    });
    images
}

/// Every step of the run as the shapes of its slide, for the web page. No
/// step counter: the page has its own.
pub fn slide_shapes(code: &str, trace: &Trace, dark: bool) -> Vec<Vec<egui::epaint::ClippedShape>> {
    let all: Vec<usize> = (0..trace.steps.len()).collect();
    let mut slides = Vec::new();
    draw_slides(code, trace, &all, dark, false, &mut |pass| {
        if let Pass::Slide(_, shapes) = pass {
            slides.push(shapes);
        }
    });
    slides
}

/// The size of a slide, in the units its shapes are drawn in.
pub const SLIDE_SIZE: Vec2 = SLIDE;

fn slide(ui: &mut egui::Ui, code: &str, trace: &Trace, step: usize, fit: &Fit) {
    let s = fit.scale;
    let fill = ui.visuals().panel_fill;
    let weak = ui.visuals().weak_text_color();
    let strong = ui.visuals().strong_text_color();
    let frame = egui::Frame::new().fill(fill).inner_margin(PAD);
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        let full = ui.max_rect();
        let left = Rect::from_min_size(full.min, vec2(fit.left_w, full.height()));
        let right = Rect::from_min_max(pos2(left.right() + GAP, full.top()), full.max);

        ui.scope_builder(UiBuilder::new().max_rect(left), |ui| {
            ui.set_clip_rect(left);
            viz::code_view(ui, code, trace, step, s);
            ui.add_space(8.0 * s);
            viz::legend(ui, s);
            ui.add_space(4.0 * s);
            ui.label(RichText::new(viz::caption(trace, step)).size(15.5 * s).color(strong));
            let output = trace.output_at(step);
            if fit.output_lines > 0 {
                // Pinned to the bottom so it stays put from slide to slide.
                let h = (24.0 + fit.output_lines as f32 * 19.0) * s;
                let area = Rect::from_min_max(pos2(left.left(), left.bottom() - h), left.max);
                ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
                    ui.label(RichText::new("Output of print()").size(12.0 * s).color(weak));
                    let lines: Vec<&str> = output.lines().collect();
                    let tail = lines[lines.len().saturating_sub(fit.output_lines)..].join("\n");
                    ui.add(
                        egui::Label::new(RichText::new(tail).monospace().size(14.0 * s))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
            }
        });
        ui.scope_builder(UiBuilder::new().max_rect(right), |ui| {
            ui.set_clip_rect(right);
            viz::memory_view(ui, trace, step, s);
        });
        if fit.counter {
            ui.painter().text(
                full.max + vec2(0.0, PAD * 0.6),
                Align2::RIGHT_BOTTOM,
                format!("Step {} of {}", step + 1, trace.steps.len()),
                FontId::proportional(11.0),
                weak,
            );
        }
    });
}

// --------------------------------------------------------------- rasterizer

struct Texture {
    w: usize,
    h: usize,
    px: Vec<Color32>,
}

/// Draws egui's triangle meshes into an RGB image.
struct Raster {
    textures: HashMap<TextureId, Texture>,
    w: usize,
    h: usize,
}

impl Raster {
    fn new() -> Self {
        Self {
            textures: HashMap::new(),
            w: (SLIDE.x * PPP).round() as usize,
            h: (SLIDE.y * PPP).round() as usize,
        }
    }

    fn update_textures(&mut self, delta: &egui::TexturesDelta) {
        for (id, patches) in &delta.set {
            for patch in patches {
                let egui::ImageData::Color(img) = &patch.image;
                let [w, h] = img.size;
                match patch.pos {
                    None => {
                        self.textures.insert(*id, Texture { w, h, px: img.pixels.clone() });
                    }
                    Some([x0, y0]) => {
                        let Some(tex) = self.textures.get_mut(id) else { continue };
                        for y in 0..h {
                            if y0 + y >= tex.h || x0 + w > tex.w {
                                break;
                            }
                            let dst = (y0 + y) * tex.w + x0;
                            tex.px[dst..dst + w].copy_from_slice(&img.pixels[y * w..(y + 1) * w]);
                        }
                    }
                }
            }
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    fn draw(&self, prims: &[egui::ClippedPrimitive], background: Color32) -> Image {
        let (w, h) = (self.w, self.h);
        // Premultiplied RGBA in gamma space, as egui blends.
        let bg = [background.r() as f32, background.g() as f32, background.b() as f32];
        let mut buf = vec![bg; w * h];
        for prim in prims {
            let Primitive::Mesh(mesh) = &prim.primitive else { continue };
            let Some(tex) = self.textures.get(&mesh.texture_id) else { continue };
            let clip = prim.clip_rect;
            let cx0 = ((clip.min.x * PPP).round().max(0.0)) as usize;
            let cy0 = ((clip.min.y * PPP).round().max(0.0)) as usize;
            let cx1 = ((clip.max.x * PPP).round().max(0.0) as usize).min(w);
            let cy1 = ((clip.max.y * PPP).round().max(0.0) as usize).min(h);
            for tri in mesh.indices.chunks_exact(3) {
                let v = [
                    &mesh.vertices[tri[0] as usize],
                    &mesh.vertices[tri[1] as usize],
                    &mesh.vertices[tri[2] as usize],
                ];
                let p = v.map(|v| (v.pos.x * PPP, v.pos.y * PPP));
                let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
                if area.abs() < 1e-6 {
                    continue;
                }
                let min_x = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(cx0 as f32) as usize;
                let min_y = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(cy0 as f32) as usize;
                let max_x = (p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(cx1);
                let max_y = (p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(cy1);
                let col = v.map(|v| v.color.to_array().map(|c| c as f32));
                let flat_uv = v[0].uv == v[1].uv && v[1].uv == v[2].uv;
                for y in min_y..max_y {
                    let py = y as f32 + 0.5;
                    for x in min_x..max_x {
                        let px = x as f32 + 0.5;
                        let w0 = ((p[1].0 - px) * (p[2].1 - py) - (p[2].0 - px) * (p[1].1 - py)) / area;
                        let w1 = ((p[2].0 - px) * (p[0].1 - py) - (p[0].0 - px) * (p[2].1 - py)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let mut c = [0.0f32; 4];
                        for i in 0..4 {
                            c[i] = w0 * col[0][i] + w1 * col[1][i] + w2 * col[2][i];
                        }
                        let t = if flat_uv {
                            tex.sample(v[0].uv.x, v[0].uv.y)
                        } else {
                            tex.sample(
                                w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x,
                                w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y,
                            )
                        };
                        let a = c[3] * t[3] / 255.0;
                        if a <= 0.0 {
                            continue;
                        }
                        let dst = &mut buf[y * w + x];
                        let keep = 1.0 - a / 255.0;
                        for i in 0..3 {
                            dst[i] = c[i] * t[i] / 255.0 + dst[i] * keep;
                        }
                    }
                }
            }
        }
        let rgb = buf
            .iter()
            .flat_map(|p| p.map(|c| c.round().clamp(0.0, 255.0) as u8))
            .collect();
        Image { w, h, rgb }
    }
}

impl Texture {
    /// Bilinear sample of a premultiplied texture at normalized coordinates.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = (u * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (v * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| self.px[y * self.w + x].to_array().map(|c| c as f32);
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        let mut out = [0.0; 4];
        for i in 0..4 {
            out[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
        }
        out
    }
}

// ------------------------------------------------------------------ writers

fn png(img: &Image) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&img.rgb, img.w as u32, img.h as u32, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("Could not encode the picture: {e}"))?;
    Ok(out)
}

/// A PDF with one full-page picture per step.
fn pdf(images: &[Image]) -> std::io::Result<Vec<u8>> {
    let (pw, ph) = (960, 540);
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let n = images.len();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    // 1: catalog, 2: page tree, then page / content / image for every step.
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect();
    obj(&mut out, format!("<< /Type /Pages /Count {n} /Kids [{}] >>", kids.join(" ")).as_bytes());
    for (i, img) in images.iter().enumerate() {
        let base = 3 + i * 3;
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw} {ph}] /Contents {} 0 R \
                 /Resources << /XObject << /Im0 {} 0 R >> >> >>",
                base + 1,
                base + 2
            )
            .as_bytes(),
        );
        let content = format!("q {pw} 0 0 {ph} 0 0 cm /Im0 Do Q");
        obj(&mut out, format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).as_bytes());
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&img.rgb)?;
        let data = z.finish()?;
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB \
             /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            img.w,
            img.h,
            data.len()
        )
        .into_bytes();
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &body);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes(),
    );
    Ok(out)
}

const NS: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";
const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const EMPTY_TREE: &str = "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

fn rels(items: &[(&str, String)]) -> String {
    let mut s = format!("{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    for (i, (kind, target)) in items.iter().enumerate() {
        s += &format!("<Relationship Id=\"rId{}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>", i + 1);
    }
    s + "</Relationships>"
}

fn theme() -> String {
    let colors = [
        ("dk2", "44546A"), ("lt2", "E7E6E6"), ("accent1", "4472C4"), ("accent2", "ED7D31"),
        ("accent3", "A5A5A5"), ("accent4", "FFC000"), ("accent5", "5B9BD5"), ("accent6", "70AD47"),
        ("hlink", "0563C1"), ("folHlink", "954F72"),
    ];
    let scheme: String = colors
        .iter()
        .map(|(n, c)| format!("<a:{n}><a:srgbClr val=\"{c}\"/></a:{n}>"))
        .collect();
    let font = "<a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/>";
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let line = format!("<a:ln w=\"9525\">{fill}</a:ln>");
    let effect = "<a:effectStyle><a:effectLst/></a:effectStyle>";
    format!(
        "{XML}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Stepwise\">\
<a:themeElements><a:clrScheme name=\"Stepwise\">\
<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>\
<a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>{scheme}</a:clrScheme>\
<a:fontScheme name=\"Stepwise\"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"Stepwise\"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst>\
<a:lnStyleLst>{line}{line}{line}</a:lnStyleLst>\
<a:effectStyleLst>{effect}{effect}{effect}</a:effectStyleLst>\
<a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"
    )
}

/// A PowerPoint file with one full-slide picture per step.
fn pptx(path: &Path, images: &[Image]) -> Result<(), String> {
    let e = |e: zip::result::ZipError| e.to_string();
    let io = |e: std::io::Error| e.to_string();
    let file = std::fs::File::create(path).map_err(io)?;
    let mut zip = zip::ZipWriter::new(file);
    let text = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let put = |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, data: &[u8], stored_file: bool| {
        zip.start_file(name, if stored_file { stored } else { text }).map_err(e)?;
        zip.write_all(data).map_err(io)
    };
    let n = images.len();
    let (cx, cy) = (12_192_000, 6_858_000);

    let mut types = format!(
        "{XML}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>"
    );
    for i in 1..=n {
        types += &format!("<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>");
    }
    types += "</Types>";
    put(&mut zip, "[Content_Types].xml", types.as_bytes(), false)?;
    put(&mut zip, "_rels/.rels", rels(&[("officeDocument", "ppt/presentation.xml".into())]).as_bytes(), false)?;

    let mut pres_rels = vec![
        ("slideMaster", "slideMasters/slideMaster1.xml".to_owned()),
        ("theme", "theme/theme1.xml".to_owned()),
    ];
    let mut ids = String::new();
    for i in 1..=n {
        pres_rels.push(("slide", format!("slides/slide{i}.xml")));
        ids += &format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 255 + i, 2 + i);
    }
    let presentation = format!(
        "{XML}<p:presentation {NS}><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>\
<p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"{cx}\" cy=\"{cy}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    );
    put(&mut zip, "ppt/presentation.xml", presentation.as_bytes(), false)?;
    put(&mut zip, "ppt/_rels/presentation.xml.rels", rels(&pres_rels).as_bytes(), false)?;
    put(&mut zip, "ppt/theme/theme1.xml", theme().as_bytes(), false)?;

    let master = format!(
        "{XML}<p:sldMaster {NS}><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg>{EMPTY_TREE}</p:spTree></p:cSld>\
<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" \
accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>"
    );
    put(&mut zip, "ppt/slideMasters/slideMaster1.xml", master.as_bytes(), false)?;
    put(
        &mut zip,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[("slideLayout", "../slideLayouts/slideLayout1.xml".into()), ("theme", "../theme/theme1.xml".into())]).as_bytes(),
        false,
    )?;
    let layout = format!(
        "{XML}<p:sldLayout {NS} type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\">{EMPTY_TREE}</p:spTree></p:cSld>\
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    );
    put(&mut zip, "ppt/slideLayouts/slideLayout1.xml", layout.as_bytes(), false)?;
    put(
        &mut zip,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[("slideMaster", "../slideMasters/slideMaster1.xml".into())]).as_bytes(),
        false,
    )?;

    for (i, img) in images.iter().enumerate() {
        let i = i + 1;
        let slide = format!(
            "{XML}<p:sld {NS}><p:cSld>{EMPTY_TREE}<p:pic><p:nvPicPr><p:cNvPr id=\"2\" name=\"Step {i}\"/>\
<p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"rId2\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic></p:spTree></p:cSld>\
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
        );
        put(&mut zip, &format!("ppt/slides/slide{i}.xml"), slide.as_bytes(), false)?;
        put(
            &mut zip,
            &format!("ppt/slides/_rels/slide{i}.xml.rels"),
            rels(&[("slideLayout", "../slideLayouts/slideLayout1.xml".into()), ("image", format!("../media/image{i}.png"))]).as_bytes(),
            false,
        )?;
        put(&mut zip, &format!("ppt/media/image{i}.png"), &png(img)?, true)?;
    }
    zip.finish().map_err(e)?;
    Ok(())
}
