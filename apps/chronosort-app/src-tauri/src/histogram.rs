//! Histogram of the picture shown in the viewer (Ctrl+H): red, green, blue and
//! brightness, from the same preview the viewer shows (or the edited preview).
//! Worked out here because the page may not read pixels from the image protocols.

use crate::{AppState, Res, err};
use serde::Serialize;
use tauri::State;

#[derive(Serialize, Debug)]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    /// Brightness (Rec. 709 weights on the stored values).
    pub l: Vec<u32>,
    /// Share of pixels that are pure black / pure white in some channel (0..1).
    pub dark: f32,
    pub light: f32,
    pub pixels: u32,
}

pub fn histogram_of(img: &image::RgbImage) -> Histogram {
    let mut h = Histogram {
        r: vec![0; 256],
        g: vec![0; 256],
        b: vec![0; 256],
        l: vec![0; 256],
        dark: 0.0,
        light: 0.0,
        pixels: 0,
    };
    let (mut dark, mut light) = (0u32, 0u32);
    for p in img.pixels() {
        let [r, g, b] = p.0;
        h.r[r as usize] += 1;
        h.g[g as usize] += 1;
        h.b[b as usize] += 1;
        let l = (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)).round() as usize;
        h.l[l.min(255)] += 1;
        if r == 0 || g == 0 || b == 0 {
            dark += 1;
        }
        if r == 255 || g == 255 || b == 255 {
            light += 1;
        }
    }
    let n = img.width() * img.height();
    h.pixels = n;
    if n > 0 {
        h.dark = dark as f32 / n as f32;
        h.light = light as f32 / n as f32;
    }
    h
}

/// Histogram of photo `id` as previewed, or of the picture being edited (`edit`).
#[tauri::command]
pub async fn image_histogram(state: State<'_, AppState>, id: u64, edit: bool) -> Res<Histogram> {
    let bytes = if edit { None } else { Some(crate::browse::preview_bytes(&state, id)?) };
    tauri::async_runtime::spawn_blocking(move || {
        let rgb = match bytes {
            Some(b) => image::load_from_memory(&b).map_err(err)?.to_rgb8(),
            None => crate::edit::edit_preview(false)?,
        };
        Ok(histogram_of(&rgb))
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_and_clipping() {
        let img = image::RgbImage::from_fn(4, 2, |x, _| match x {
            0 => image::Rgb([0, 0, 0]),
            1 => image::Rgb([255, 255, 255]),
            2 => image::Rgb([255, 0, 0]),
            _ => image::Rgb([128, 128, 128]),
        });
        let h = super::histogram_of(&img);
        assert_eq!(h.pixels, 8);
        assert_eq!(h.r[255], 4);
        assert_eq!(h.g[0], 4);
        assert_eq!(h.l[0], 2);
        assert_eq!(h.l[255], 2);
        assert_eq!(h.l[128], 2);
        assert_eq!(h.l[54], 2); // pure red is dark-ish
        assert!((h.dark - 0.5).abs() < 1e-6, "{}", h.dark); // black + red
        assert!((h.light - 0.5).abs() < 1e-6, "{}", h.light); // white + red
    }
}
