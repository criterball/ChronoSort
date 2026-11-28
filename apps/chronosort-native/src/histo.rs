//! Histogram of the photo in the viewer (Ctrl+H): red, green, blue and brightness, worked out by the loader
//! from the same 1280-pixel preview the viewer shows first (the Tauri app's `histogram.rs` rules).

use crate::{bold, colors};
use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, pos2, vec2};

pub struct Histo {
    pub r: [u32; 256],
    pub g: [u32; 256],
    pub b: [u32; 256],
    /// Brightness (Rec. 709 weights on the stored values).
    pub l: [u32; 256],
    /// Share of pixels that are pure black / pure white in some channel (0..1).
    pub dark: f32,
    pub light: f32,
}

impl Histo {
    pub fn of_rgba(rgba: &[u8]) -> Self {
        let mut h = Histo { r: [0; 256], g: [0; 256], b: [0; 256], l: [0; 256], dark: 0.0, light: 0.0 };
        let (mut dark, mut light, mut n) = (0u32, 0u32, 0u32);
        for p in rgba.as_chunks::<4>().0 {
            let (r, g, b) = (p[0], p[1], p[2]);
            h.r[r as usize] += 1;
            h.g[g as usize] += 1;
            h.b[b as usize] += 1;
            // Integer Rec. 709: 2126 / 7152 / 722 of 10 000, rounded.
            let l = (2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b) + 5000) / 10_000;
            h.l[l.min(255) as usize] += 1;
            dark += u32::from(r == 0 || g == 0 || b == 0);
            light += u32::from(r == 255 || g == 255 || b == 255);
            n += 1;
        }
        if n > 0 {
            h.dark = dark as f32 / n as f32;
            h.light = light as f32 / n as f32;
        }
        h
    }

    /// The box: brightness filled in grey, the three colours as lines, and the clipped shares below.
    pub fn paint(&self, p: &egui::Painter, r: Rect, note: Option<&str>) {
        p.rect_filled(r, 8.0, Color32::from_rgba_premultiplied(20, 22, 26, 235));
        p.text(r.min + vec2(12.0, 8.0), Align2::LEFT_TOP, "Histogram", bold(14.0), colors::TEXT);
        p.text(
            r.right_top() + vec2(-12.0, 8.0),
            Align2::RIGHT_TOP,
            "Ctrl+H",
            FontId::proportional(12.0),
            colors::MUTED,
        );
        let plot = Rect::from_min_max(r.min + vec2(12.0, 30.0), r.max - vec2(12.0, 34.0));
        p.rect_filled(plot, 2.0, Color32::from_black_alpha(160));
        // Scale to the 99.5th highest bar so one huge spike (a white sky) does not flatten the rest.
        let mut all: Vec<u32> = self.l.iter().chain(&self.r).chain(&self.g).chain(&self.b).copied().collect();
        all.sort_unstable();
        let top = all[(all.len() as f32 * 0.995) as usize].max(1) as f32;
        let pt = |i: usize, v: u32| -> Pos2 {
            let x = plot.min.x + plot.width() * i as f32 / 255.0;
            pos2(x, plot.max.y - plot.height() * (v as f32 / top).min(1.0))
        };
        // Brightness as thin filled bars (a concave polygon cannot be filled directly).
        let grey = Color32::from_gray(150).gamma_multiply(0.55);
        for i in 0..255 {
            let (a, b) = (pt(i, self.l[i]), pt(i + 1, self.l[i + 1]));
            p.rect_filled(Rect::from_min_max(pos2(a.x, a.y.min(b.y)), pos2(b.x + 0.6, plot.max.y)), 0.0, grey);
        }
        for (ch, c) in [
            (&self.r, Color32::from_rgb(235, 80, 80)),
            (&self.g, Color32::from_rgb(90, 210, 110)),
            (&self.b, Color32::from_rgb(90, 140, 240)),
        ] {
            let line: Vec<Pos2> = (0..256).map(|i| pt(i, ch[i])).collect();
            p.add(Shape::line(line, Stroke::new(1.2, c)));
        }
        let pct = |f: f32| if f > 0.0 && f < 0.001 { "< 0.1 %".to_string() } else { format!("{:.1} %", f * 100.0) };
        let warn = |f: f32| if f >= 0.02 { colors::FOLDER } else { colors::MUTED };
        let y = r.max.y - 22.0;
        p.text(
            pos2(plot.min.x, y),
            Align2::LEFT_CENTER,
            format!("Black {}", pct(self.dark)),
            FontId::proportional(12.5),
            warn(self.dark),
        );
        p.text(
            pos2(plot.max.x, y),
            Align2::RIGHT_CENTER,
            format!("White {}", pct(self.light)),
            FontId::proportional(12.5),
            warn(self.light),
        );
        if let Some(n) = note {
            p.text(pos2(plot.center().x, y), Align2::CENTER_CENTER, n, FontId::proportional(12.0), colors::MUTED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_clipping() {
        // Two pixels: pure white and a mid grey.
        let h = Histo::of_rgba(&[255, 255, 255, 255, 128, 128, 128, 255]);
        assert_eq!((h.r[255], h.r[128], h.l[255], h.l[128]), (1, 1, 1, 1));
        assert_eq!((h.dark, h.light), (0.0, 0.5));
    }
}
