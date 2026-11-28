//! Drawing into pictures without a browser: text in the ChronoSort UI font (ab_glyph), thick lines, boxes,
//! ovals, arrows. Used for "Draw / text" in Retouch (a layer the size of the picture) and for the text
//! watermark (a PNG the effects code places on the photo). Replaces the web canvas the Tauri app drew with.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

static REGULAR: &[u8] = include_bytes!("../fonts/ChronoSortUI-Regular.ttf");
static SEMIBOLD: &[u8] = include_bytes!("../fonts/ChronoSortUI-SemiBold.ttf");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Pen,
    Line,
    Arrow,
    Box,
    Oval,
    Text,
}

impl Shape {
    pub const ALL: [(Shape, &'static str); 6] = [
        (Shape::Pen, "Pen"),
        (Shape::Line, "Line"),
        (Shape::Arrow, "Arrow"),
        (Shape::Box, "Box"),
        (Shape::Oval, "Oval"),
        (Shape::Text, "Text"),
    ];
}

/// One drawn mark, in picture pixels.
#[derive(Clone, Debug)]
pub enum Mark {
    /// Pen strokes, lines and arrows are polylines; boxes and ovals too (closed).
    Path {
        points: Vec<(f32, f32)>,
        width: f32,
        color: [u8; 3],
        arrow: bool,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        color: [u8; 3],
    },
}

impl Mark {
    /// A shape from `a` to `b`.
    pub fn shape(shape: Shape, a: (f32, f32), b: (f32, f32), width: f32, color: [u8; 3]) -> Mark {
        let points = match shape {
            Shape::Box => vec![a, (b.0, a.1), b, (a.0, b.1), a],
            Shape::Oval => {
                let (cx, cy, rx, ry) =
                    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, (b.0 - a.0).abs() / 2.0, (b.1 - a.1).abs() / 2.0);
                (0..=72)
                    .map(|i| {
                        let t = i as f32 / 72.0 * std::f32::consts::TAU;
                        (cx + rx * t.cos(), cy + ry * t.sin())
                    })
                    .collect()
            }
            _ => vec![a, b],
        };
        Mark::Path { points, width, color, arrow: shape == Shape::Arrow }
    }
}

/// The marks drawn into a transparent layer of the picture's size (for `Retouch::Draw`), as PNG bytes.
pub fn layer_png(w: u32, h: u32, marks: &[Mark]) -> Result<Vec<u8>, String> {
    let mut img = RgbaImage::new(w, h);
    for m in marks {
        match m {
            Mark::Path { points, width, color, arrow } => {
                for p in points.windows(2) {
                    thick_line(&mut img, p[0], p[1], *width, *color, 255);
                }
                if *arrow && points.len() >= 2 {
                    let (a, b) = (points[0], points[points.len() - 1]);
                    let ang = (b.1 - a.1).atan2(b.0 - a.0);
                    let len = (width * 4.0).max(12.0);
                    for s in [-0.5f32, 0.5] {
                        let t = ang + std::f32::consts::PI + s;
                        thick_line(&mut img, b, (b.0 + len * t.cos(), b.1 + len * t.sin()), *width, *color, 255);
                    }
                }
            }
            Mark::Text { x, y, text, size, color } => draw_text(&mut img, *x, *y, text, *size, *color, false),
        }
    }
    png(&img)
}

/// A contact sheet's text layer: the marks (title), plus centred captions cut to the cell width.
pub fn sheet_layer(
    w: u32,
    h: u32,
    marks: &[Mark],
    captions: &[(f32, f32, String)],
    cell_w: f32,
    size: f32,
    color: [u8; 3],
) -> Result<Vec<u8>, String> {
    let mut img = RgbaImage::new(w, h);
    for m in marks {
        if let Mark::Text { x, y, text, size, color } = m {
            draw_text(&mut img, *x, *y, text, *size, *color, false);
        }
    }
    for (x, y, text) in captions {
        let t = fit_text(text, size, cell_w);
        let (tw, _) = measure(&t, size, false);
        draw_text(&mut img, x + ((cell_w - tw) / 2.0).max(0.0), *y, &t, size, color, false);
    }
    png(&img)
}

/// `text`, cut with "…" so it is at most `max` pixels wide.
fn fit_text(text: &str, size: f32, max: f32) -> String {
    if measure(text, size, false).0 <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let t: String = chars[..mid].iter().collect::<String>() + "…";
        if measure(&t, size, false).0 <= max { lo = mid } else { hi = mid - 1 }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// The text watermark: large (stays sharp on big photos), optional soft halo for readability.
pub fn text_watermark_png(text: &str, bold: bool, color: [u8; 3], outline: bool) -> Option<Vec<u8>> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let size: f32 = 160.0;
    let pad = (size * 0.25).ceil();
    let (tw, th) = measure(t, size, bold);
    let (w, h) = ((tw + 2.0 * pad).ceil() as u32, (th + 2.0 * pad).ceil() as u32);
    let mut img = RgbaImage::new(w.max(1), h.max(1));
    if outline {
        // A soft dark (or light) halo keeps the text readable on any background.
        let light = color.iter().map(|&c| u32::from(c)).sum::<u32>() > 382;
        let halo = if light { [0, 0, 0] } else { [255, 255, 255] };
        let r = size * 0.06;
        for (dx, dy) in [(-r, 0.0), (r, 0.0), (0.0, -r), (0.0, r), (-r, -r), (r, r), (-r, r), (r, -r)] {
            draw_text_alpha(&mut img, pad + dx, pad + dy, t, size, halo, bold, 70);
        }
    }
    draw_text_alpha(&mut img, pad, pad, t, size, color, bold, 255);
    png(&img).ok()
}

fn font(bold: bool) -> FontRef<'static> {
    FontRef::try_from_slice(if bold { SEMIBOLD } else { REGULAR }).expect("embedded font")
}

/// Width and height of (multi-line) text at `size` px.
pub fn measure(text: &str, size: f32, bold: bool) -> (f32, f32) {
    let f = font(bold);
    let sf = f.as_scaled(PxScale::from(size));
    let mut w: f32 = 0.0;
    let lines: Vec<&str> = text.split('\n').collect();
    for l in &lines {
        let lw: f32 = l.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum();
        w = w.max(lw);
    }
    (w, lines.len() as f32 * size * 1.2)
}

fn draw_text(img: &mut RgbaImage, x: f32, y: f32, text: &str, size: f32, color: [u8; 3], bold: bool) {
    draw_text_alpha(img, x, y, text, size, color, bold, 255);
}

#[allow(clippy::too_many_arguments)]
fn draw_text_alpha(img: &mut RgbaImage, x: f32, y: f32, text: &str, size: f32, color: [u8; 3], bold: bool, alpha: u8) {
    let f = font(bold);
    let scale = PxScale::from(size);
    let sf = f.as_scaled(scale);
    for (n, line) in text.split('\n').enumerate() {
        let mut caret = x;
        let base = y + n as f32 * size * 1.2 + sf.ascent();
        for c in line.chars() {
            let id = sf.glyph_id(c);
            let g = id.with_scale_and_position(scale, ab_glyph::point(caret, base));
            caret += sf.h_advance(id);
            if let Some(o) = f.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let (px, py) = (b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64);
                    blend(img, px, py, color, (cov * f32::from(alpha)) as u8);
                });
            }
        }
    }
}

/// A round-capped thick line (stamped discs; good enough for marks on photos).
fn thick_line(img: &mut RgbaImage, a: (f32, f32), b: (f32, f32), width: f32, color: [u8; 3], alpha: u8) {
    let r = (width / 2.0).max(0.5);
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let steps = (len / (r * 0.4).max(0.5)).ceil().max(1.0) as usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        disc(img, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, r, color, alpha);
    }
}

fn disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, color: [u8; 3], alpha: u8) {
    let (x0, x1) = ((cx - r - 1.0).floor() as i64, (cx + r + 1.0).ceil() as i64);
    let (y0, y1) = ((cy - r - 1.0).floor() as i64, (cy + r + 1.0).ceil() as i64);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cov = (r + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend(img, x, y, color, (cov * f32::from(alpha)) as u8);
            }
        }
    }
}

/// Paint `color` at `a` (0..255) over the pixel, keeping the stronger coverage (no darkening at overlaps).
fn blend(img: &mut RgbaImage, x: i64, y: i64, color: [u8; 3], a: u8) {
    if x < 0 || y < 0 || x >= i64::from(img.width()) || y >= i64::from(img.height()) || a == 0 {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    if p.0[3] == 0 {
        *p = Rgba([color[0], color[1], color[2], a]);
        return;
    }
    // Source-over in straight alpha.
    let (sa, da) = (f32::from(a) / 255.0, f32::from(p.0[3]) / 255.0);
    let oa = sa + da * (1.0 - sa);
    for (d, &s) in p.0[..3].iter_mut().zip(&color[..3]) {
        let c = (f32::from(s) * sa + f32::from(*d) * da * (1.0 - sa)) / oa.max(1e-6);
        *d = c.round() as u8;
    }
    p.0[3] = (oa * 255.0).round() as u8;
}

fn png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out)
}

/// A caption on a printed page: dark grey text centred in the box (x, y, w), `px` pixels high.
pub fn caption_rgb(img: &mut image::RgbImage, x: f32, y: f32, w: f32, text: &str, px: f32) {
    // A long name gets a little smaller to fit under its photo (down to 60 %), then is cut.
    let (full, _) = measure(text, px, false);
    let px = if full > w { px * (w / full).max(0.6) } else { px };
    let (tw, _) = measure(text, px, false);
    let (bw, bh) = (w.max(1.0).ceil() as u32, (px * 1.6).ceil() as u32);
    // A strip just the size of the caption, drawn and then blended onto the page.
    let mut strip = RgbaImage::new(bw, bh);
    draw_text_alpha(&mut strip, ((w - tw) / 2.0).max(0.0), 0.0, text, px, [60, 60, 60], false, 255);
    let (ox, oy) = (x.round() as i64, y.round() as i64);
    for (sx, sy, p) in strip.enumerate_pixels() {
        let a = u32::from(p.0[3]);
        let (dx, dy) = (ox + i64::from(sx), oy + i64::from(sy));
        if a == 0 || dx < 0 || dy < 0 || dx >= i64::from(img.width()) || dy >= i64::from(img.height()) {
            continue;
        }
        let d = img.get_pixel_mut(dx as u32, dy as u32);
        for i in 0..3 {
            d.0[i] = ((u32::from(p.0[i]) * a + u32::from(d.0[i]) * (255 - a)) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_has_marks_where_drawn_and_nothing_elsewhere() {
        let marks = vec![
            Mark::shape(Shape::Box, (10.0, 10.0), (50.0, 40.0), 4.0, [255, 0, 0]),
            Mark::Text { x: 60.0, y: 10.0, text: "Hi".into(), size: 30.0, color: [255, 255, 255] },
        ];
        let png = layer_png(120, 60, &marks).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(10, 25).0, [255, 0, 0, 255], "left edge of the box");
        assert_eq!(img.get_pixel(30, 25).0[3], 0, "inside the box stays clear");
        assert!(img.pixels().any(|p| p.0 == [255, 255, 255, 255]), "text drawn");
    }

    #[test]
    fn watermark_text_is_sized_to_the_text() {
        assert!(text_watermark_png("  ", false, [255, 255, 255], true).is_none());
        let png = text_watermark_png("© Family 2024", true, [255, 255, 255], true).unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert!(img.width() > img.height() * 3, "a wide strip of text");
    }
}
