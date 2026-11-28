//! Retouching: red-eye removal, content-aware healing, cloning, and blending
//! a drawn layer (lines, shapes, text) onto the picture.
//!
//! * Healing uses photocraft's content-aware fill (`craft_develop::inpaint`:
//!   PatchMatch completion, with the smooth membrane fill as its fallback), on
//!   a crop around each group of spots, so a small repair on a big photo stays
//!   fast.
//! * Red-eye follows the develop model's red-eye settings (pupil size and
//!   darken, 0..100, 50 = default): only red pixels inside the eye are changed,
//!   with a soft edge, so eyelids and skin are left alone.
//!
//! All coordinates are pixels of the picture as it is now.

use craft_develop::inpaint::{CompleteParams, complete, membrane_fill};
use cs_image::{ChannelLayout, Image, SampleType};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// One eye: an ellipse around it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eye {
    pub cx: f64,
    pub cy: f64,
    pub rx: f64,
    pub ry: f64,
    /// 0..100, 50 = the whole red area found.
    #[serde(default = "fifty")]
    pub pupil_size: f64,
    /// 0..100: how dark the pupil becomes.
    #[serde(default = "fifty")]
    pub darken: f64,
}

fn fifty() -> f64 {
    50.0
}

/// One round brush dab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dab {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// Pixels as normalised `f32` (any depth), with how to write them back.
pub(crate) struct Buf {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) ch: usize,
    pub(crate) v: Vec<f32>,
    pub(crate) layout: ChannelLayout,
    pub(crate) sample: SampleType,
}

pub(crate) fn buf(img: &Image) -> Buf {
    let alpha =
        matches!(img.layout(), ChannelLayout::GrayA | ChannelLayout::Rgba | ChannelLayout::CmykA | ChannelLayout::LabA);
    let layout = if alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let sample = if img.sample_type() == SampleType::U8 { SampleType::U8 } else { SampleType::U16 };
    let src = img.converted(layout, sample);
    let (w, h) = img.dimensions();
    let v: Vec<f32> = if sample == SampleType::U8 {
        src.data().iter().map(|&b| f32::from(b) / 255.0).collect()
    } else {
        src.data().as_chunks::<2>().0.iter().map(|c| f32::from(u16::from_ne_bytes(*c)) / 65535.0).collect()
    };
    Buf { w: w as usize, h: h as usize, ch: if alpha { 4 } else { 3 }, v, layout, sample }
}

pub(crate) fn image_of(like: &Image, b: &Buf) -> Result<Image, String> {
    let data: Vec<u8> = if b.sample == SampleType::U8 {
        b.v.iter().map(|x| (x * 255.0).round().clamp(0.0, 255.0) as u8).collect()
    } else {
        b.v.iter().flat_map(|x| ((x * 65535.0).round().clamp(0.0, 65535.0) as u16).to_ne_bytes()).collect()
    };
    Ok(Image::from_raw(b.w as u32, b.h as u32, b.layout, b.sample, data)
        .map_err(|e| e.to_string())?
        .with_icc(like.icc.clone())
        .with_meta(like.meta.clone()))
}

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Remove red eye inside each ellipse.
pub fn red_eye(img: &Image, eyes: &[Eye]) -> Result<Image, String> {
    let mut b = buf(img);
    let (w, h, ch) = (b.w, b.h, b.ch);
    for e in eyes {
        if !(e.cx.is_finite() && e.cy.is_finite() && e.rx > 0.5 && e.ry > 0.5) {
            continue;
        }
        // Pupil size scales the ellipse that may change (50 = all of it).
        let k = (0.6 + 0.8 * e.pupil_size.clamp(0.0, 100.0) / 100.0).min(1.2);
        let (rx, ry) = (e.rx * k, e.ry * k);
        let dark = (e.darken.clamp(0.0, 100.0) / 100.0) as f32;
        let x0 = (e.cx - rx).floor().max(0.0) as usize;
        let x1 = ((e.cx + rx).ceil().max(0.0) as usize).min(w);
        let y0 = (e.cy - ry).floor().max(0.0) as usize;
        let y1 = ((e.cy + ry).ceil().max(0.0) as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = ((x as f64 + 0.5 - e.cx) / rx, (y as f64 + 0.5 - e.cy) / ry);
                let d = (dx * dx + dy * dy).sqrt() as f32;
                if d >= 1.0 {
                    continue;
                }
                let i = (y * w + x) * ch;
                let (r, g, bl) = (b.v[i], b.v[i + 1], b.v[i + 2]);
                // Redness: red well above both other channels.
                let other = g.max(bl);
                let red = smooth(0.08, 0.25, r - other) * smooth(1.25, 1.8, r / other.max(1e-3));
                let m = red * (1.0 - smooth(0.75, 1.0, d));
                if m <= 0.0 {
                    continue;
                }
                // The pupil's own grey from green and blue, then darkened.
                let grey = (g + bl) * 0.5 * (1.0 - 0.7 * dark);
                let nv = [grey, g.min(grey.max(g * (1.0 - 0.7 * dark))), bl.min(grey.max(bl * (1.0 - 0.7 * dark)))];
                for (v, n) in b.v[i..i + 3].iter_mut().zip(nv) {
                    *v += (n - *v) * m;
                }
            }
        }
    }
    image_of(img, &b)
}

/// The dabs grouped so that each group can be repaired in its own crop.
fn groups(dabs: &[Dab]) -> Vec<Vec<Dab>> {
    let mut out: Vec<Vec<Dab>> = vec![];
    for d in dabs.iter().filter(|d| d.x.is_finite() && d.y.is_finite() && d.r >= 0.5) {
        let near = out.iter().position(|g| g.iter().any(|o| (o.x - d.x).hypot(o.y - d.y) < (o.r + d.r) * 3.0));
        match near {
            Some(i) => out[i].push(*d),
            None => out.push(vec![*d]),
        }
    }
    out
}

/// Content-aware repair of everything under the dabs (blemishes, dust, scratches).
pub fn heal(img: &Image, dabs: &[Dab]) -> Result<Image, String> {
    let mut b = buf(img);
    let (w, h, ch) = (b.w, b.h, b.ch);
    for g in groups(dabs) {
        // The group's box plus room around it to copy from.
        let rmax = g.iter().map(|d| d.r).fold(0.0, f64::max);
        let pad = (rmax * 6.0).max(24.0);
        let bx0 = (g.iter().map(|d| d.x - d.r).fold(f64::MAX, f64::min) - pad).max(0.0) as usize;
        let by0 = (g.iter().map(|d| d.y - d.r).fold(f64::MAX, f64::min) - pad).max(0.0) as usize;
        let bx1 = ((g.iter().map(|d| d.x + d.r).fold(f64::MIN, f64::max) + pad).max(0.0) as usize).min(w);
        let by1 = ((g.iter().map(|d| d.y + d.r).fold(f64::MIN, f64::max) + pad).max(0.0) as usize).min(h);
        if bx1 <= bx0 + 2 || by1 <= by0 + 2 {
            continue;
        }
        let (cw, chh) = (bx1 - bx0, by1 - by0);
        let mut crop = vec![0.0f32; cw * chh * ch];
        for y in 0..chh {
            let s = ((by0 + y) * w + bx0) * ch;
            crop[y * cw * ch..(y + 1) * cw * ch].copy_from_slice(&b.v[s..s + cw * ch]);
        }
        // The hole: the dabs grown by one pixel so their soft edge is replaced too.
        let inside = |x: usize, y: usize, grow: f64| {
            let (px, py) = ((bx0 + x) as f64 + 0.5, (by0 + y) as f64 + 0.5);
            g.iter().map(|d| (px - d.x).hypot(py - d.y) - d.r).fold(f64::MAX, f64::min) < grow
        };
        let hole: Vec<bool> = (0..cw * chh).map(|i| inside(i % cw, i / cw, 1.0)).collect();
        if !hole.iter().any(|v| *v) {
            continue;
        }
        let p = CompleteParams { patch_radius: if rmax > 12.0 { 4 } else { 3 }, ..Default::default() };
        let filled =
            complete(cw, chh, ch, &crop, &hole, &p).unwrap_or_else(|| membrane_fill(cw, chh, ch, &crop, &hole));
        if filled.len() != crop.len() {
            continue;
        }
        // Blend back: the hole fully, a 2 px ring around it softly.
        for y in 0..chh {
            for x in 0..cw {
                let (px, py) = ((bx0 + x) as f64 + 0.5, (by0 + y) as f64 + 0.5);
                let dist = g.iter().map(|d| (px - d.x).hypot(py - d.y) - d.r).fold(f64::MAX, f64::min);
                let a = (1.0 - smooth(1.0, 3.0, dist as f32)).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let (ci, bi) = ((y * cw + x) * ch, ((by0 + y) * w + bx0 + x) * ch);
                for c in 0..ch.min(3) {
                    b.v[bi + c] += (filled[ci + c] - b.v[bi + c]) * a;
                }
            }
        }
    }
    image_of(img, &b)
}

/// Clone: copy the picture from `(dx, dy)` away into the dabs, with a soft
/// edge (`hardness` 0..100: 100 = hard edge).
pub fn clone_stamp(img: &Image, dabs: &[Dab], dx: f64, dy: f64, hardness: f64) -> Result<Image, String> {
    if !(dx.is_finite() && dy.is_finite()) {
        return Err("No clone source was set.".into());
    }
    let src = buf(img);
    let mut b = buf(img);
    let (w, h, ch) = (b.w, b.h, b.ch);
    let hard = (hardness.clamp(0.0, 100.0) / 100.0) as f32;
    // Strength per pixel: the strongest dab covering it (dabs overlap along a stroke).
    let mut alpha = vec![0.0f32; w * h];
    for d in dabs.iter().filter(|d| d.x.is_finite() && d.y.is_finite() && d.r >= 0.5) {
        let x0 = (d.x - d.r).floor().max(0.0) as usize;
        let x1 = ((d.x + d.r).ceil().max(0.0) as usize).min(w);
        let y0 = (d.y - d.r).floor().max(0.0) as usize;
        let y1 = ((d.y + d.r).ceil().max(0.0) as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let t = ((x as f64 + 0.5 - d.x).hypot(y as f64 + 0.5 - d.y) / d.r) as f32;
                let a = 1.0 - smooth(hard * 0.95, 1.0, t);
                let p = &mut alpha[y * w + x];
                *p = p.max(a);
            }
        }
    }
    let (ox, oy) = (dx.round() as isize, dy.round() as isize);
    b.v.par_chunks_mut(w * ch).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let a = alpha[y * w + x];
            if a <= 0.0 {
                continue;
            }
            let (sx, sy) = (x as isize + ox, y as isize + oy);
            if sx < 0 || sy < 0 || sx >= w as isize || sy >= h as isize {
                continue;
            }
            let si = (sy as usize * w + sx as usize) * ch;
            for c in 0..ch.min(3) {
                row[x * ch + c] += (src.v[si + c] - row[x * ch + c]) * a;
            }
        }
    });
    image_of(img, &b)
}

/// Blend a drawn layer (RGBA, the picture's size; PNG bytes) over the picture.
pub fn overlay(img: &Image, png: &[u8]) -> Result<Image, String> {
    let layer = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| format!("The drawing could not be read: {e}"))?
        .to_rgba32f();
    let mut b = buf(img);
    if (layer.width() as usize, layer.height() as usize) != (b.w, b.h) {
        return Err("The drawing does not match the picture's size.".into());
    }
    let ch = b.ch;
    let lv = layer.as_raw();
    b.v.par_chunks_mut(ch).zip(lv.par_chunks(4)).for_each(|(p, l)| {
        let a = l[3].clamp(0.0, 1.0);
        if a > 0.0 {
            for c in 0..3 {
                p[c] = l[c] * a + p[c] * (1.0 - a);
            }
            if ch == 4 {
                p[3] = a + p[3] * (1.0 - a);
            }
        }
    });
    image_of(img, &b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Image {
        let mut d = Vec::new();
        for y in 0..h {
            for x in 0..w {
                d.extend(f(x, y));
            }
        }
        Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, d).unwrap()
    }
    fn px(i: &Image, x: u32, y: u32) -> [u8; 3] {
        let w = i.dimensions().0;
        let o = ((y * w + x) * 3) as usize;
        [i.data()[o], i.data()[o + 1], i.data()[o + 2]]
    }

    #[test]
    fn red_eye_turns_a_red_pupil_dark_and_leaves_skin_alone() {
        // Skin-coloured face with a red pupil at (30, 20), radius 5.
        let p = rgb(60, 40, |x, y| {
            if (f64::from(x) - 30.0).hypot(f64::from(y) - 20.0) < 5.0 { [210, 30, 40] } else { [225, 180, 160] }
        });
        let e = Eye { cx: 30.0, cy: 20.0, rx: 9.0, ry: 9.0, pupil_size: 50.0, darken: 50.0 };
        let o = red_eye(&p, &[e]).unwrap();
        let c = px(&o, 30, 20);
        assert!(c[0] < 60 && c[0] <= c[1] + 10, "pupil no longer red: {c:?}");
        assert_eq!(px(&o, 30, 13), px(&p, 30, 13), "skin inside the ellipse untouched");
        assert_eq!(px(&o, 5, 5), px(&p, 5, 5));
    }

    #[test]
    fn heal_removes_a_spot_from_a_pattern() {
        // Stripes with a black blob: after healing, the blob area looks like the stripes.
        let p = rgb(96, 96, |x, y| {
            if (f64::from(x) - 48.0).hypot(f64::from(y) - 48.0) < 6.0 {
                [0, 0, 0]
            } else if (x / 4) % 2 == 0 {
                [200, 200, 200]
            } else {
                [120, 120, 120]
            }
        });
        let o = heal(&p, &[Dab { x: 48.0, y: 48.0, r: 7.0 }]).unwrap();
        let c = px(&o, 48, 48);
        assert!(c[0] > 90, "the black blob is filled: {c:?}");
        assert_eq!(px(&o, 5, 5), px(&p, 5, 5), "far away untouched");
        // Several spots far apart are each repaired.
        let o = heal(&p, &[Dab { x: 48.0, y: 48.0, r: 7.0 }, Dab { x: 10.0, y: 85.0, r: 3.0 }]).unwrap();
        assert!(px(&o, 48, 48)[0] > 90);
    }

    #[test]
    fn clone_copies_from_the_source_offset() {
        let p = rgb(50, 50, |x, _| if x < 25 { [250, 0, 0] } else { [0, 0, 250] });
        let o = clone_stamp(&p, &[Dab { x: 40.0, y: 25.0, r: 4.0 }], -30.0, 0.0, 100.0).unwrap();
        assert_eq!(px(&o, 40, 25), [250, 0, 0], "red copied from the left");
        assert_eq!(px(&o, 40, 10), [0, 0, 250], "outside the dab unchanged");
        assert!(clone_stamp(&p, &[], f64::NAN, 0.0, 50.0).is_err());
    }

    #[test]
    fn overlay_blends_by_alpha() {
        let p = rgb(4, 2, |_, _| [0, 0, 0]);
        let mut layer = image::RgbaImage::new(4, 2);
        layer.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
        layer.put_pixel(2, 0, image::Rgba([255, 255, 255, 128]));
        let mut png = std::io::Cursor::new(Vec::new());
        layer.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let o = overlay(&p, png.get_ref()).unwrap();
        assert_eq!(px(&o, 1, 0), [255, 255, 255]);
        assert!((120..136).contains(&px(&o, 2, 0)[0]));
        assert_eq!(px(&o, 0, 0), [0, 0, 0]);
        let small = image::RgbaImage::new(2, 2);
        let mut png = std::io::Cursor::new(Vec::new());
        small.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert!(overlay(&p, png.get_ref()).is_err(), "wrong size refused");
    }
}
