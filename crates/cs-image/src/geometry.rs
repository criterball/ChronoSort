//! Straighten (rotate by any angle, cropped to the largest rectangle of the
//! picture's own shape, so no blank corners) and canvas size (a border, or
//! trimming from one side), for 8- and 16-bit pictures with or without alpha.

use crate::{ChannelLayout, Image, MAX_PIXELS, SampleType, err};
use rayon::prelude::*;

/// Pixels as `f32` samples (any depth), `ch` channels (3 or 4), plus how to
/// write them back.
struct Raster {
    w: usize,
    h: usize,
    ch: usize,
    v: Vec<f32>,
    layout: ChannelLayout,
    sample: SampleType,
}

fn raster(img: &Image) -> Raster {
    let alpha =
        matches!(img.layout(), ChannelLayout::GrayA | ChannelLayout::Rgba | ChannelLayout::CmykA | ChannelLayout::LabA);
    let layout = if alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let sample = if img.sample_type() == SampleType::U8 { SampleType::U8 } else { SampleType::U16 };
    let src = img.converted(layout, sample);
    let (w, h) = img.dimensions();
    let v: Vec<f32> = if sample == SampleType::U8 {
        src.data().iter().map(|&b| f32::from(b)).collect()
    } else {
        src.data().as_chunks::<2>().0.iter().map(|c| f32::from(u16::from_ne_bytes(*c))).collect()
    };
    Raster { w: w as usize, h: h as usize, ch: if alpha { 4 } else { 3 }, v, layout, sample }
}

fn image_of(like: &Image, r: &Raster, w: usize, h: usize, v: &[f32]) -> Result<Image, String> {
    let data: Vec<u8> = if r.sample == SampleType::U8 {
        v.iter().map(|x| x.round().clamp(0.0, 255.0) as u8).collect()
    } else {
        v.iter().flat_map(|x| (x.round().clamp(0.0, 65535.0) as u16).to_ne_bytes()).collect()
    };
    Ok(Image::from_raw(w as u32, h as u32, r.layout, r.sample, data)
        .map_err(err)?
        .with_icc(like.icc.clone())
        .with_meta(like.meta.clone()))
}

/// Catmull-Rom weights for the fractional offset `t`.
#[inline]
fn cubic(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [(-t3 + 2.0 * t2 - t) * 0.5, (3.0 * t3 - 5.0 * t2 + 2.0) * 0.5, (-3.0 * t3 + 4.0 * t2 + t) * 0.5, (t3 - t2) * 0.5]
}

/// The largest centred rectangle with the picture's own aspect ratio that
/// fits inside the picture turned by `rad`: as a fraction of the size.
pub fn straighten_scale(w: f64, h: f64, rad: f64) -> f64 {
    let (s, c) = (rad.sin().abs(), rad.cos().abs());
    (w / (w * c + h * s)).min(h / (w * s + h * c))
}

/// Turn the picture by `degrees` (positive = clockwise) and crop to the
/// largest rectangle of its own shape.
pub fn straighten(img: &Image, degrees: f64) -> Result<Image, String> {
    if !degrees.is_finite() || degrees.abs() > 45.0 {
        return Err("Straighten works for angles up to 45° either way.".into());
    }
    if degrees == 0.0 {
        return Ok(img.clone());
    }
    let r = raster(img);
    let rad = degrees.to_radians();
    let k = straighten_scale(r.w as f64, r.h as f64, rad);
    let (ow, oh) = (((r.w as f64 * k).floor() as usize).max(1), ((r.h as f64 * k).floor() as usize).max(1));
    let (cos, sin) = (rad.cos() as f32, rad.sin() as f32);
    let (cx, cy) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
    let (ocx, ocy) = (ow as f32 / 2.0, oh as f32 / 2.0);
    let ch = r.ch;
    let at = |x: isize, y: isize, c: usize| -> f32 {
        let x = x.clamp(0, r.w as isize - 1) as usize;
        let y = y.clamp(0, r.h as isize - 1) as usize;
        r.v[(y * r.w + x) * ch + c]
    };
    let max = if r.sample == SampleType::U8 { 255.0 } else { 65535.0 };
    let mut out = vec![0.0f32; ow * oh * ch];
    out.par_chunks_mut(ow * ch).enumerate().for_each(|(oy, row)| {
        for ox in 0..ow {
            // Output pixel centre → source position: turn back by the angle.
            let (dx, dy) = (ox as f32 + 0.5 - ocx, oy as f32 + 0.5 - ocy);
            let sx = cos * dx + sin * dy + cx - 0.5;
            let sy = -sin * dx + cos * dy + cy - 0.5;
            let (ix, iy) = (sx.floor(), sy.floor());
            let (wx, wy) = (cubic(sx - ix), cubic(sy - iy));
            let (ix, iy) = (ix as isize, iy as isize);
            for c in 0..ch {
                let mut acc = 0.0;
                for (j, wyj) in wy.iter().enumerate() {
                    let mut line = 0.0;
                    for (i, wxi) in wx.iter().enumerate() {
                        line += wxi * at(ix - 1 + i as isize, iy - 1 + j as isize, c);
                    }
                    acc += wyj * line;
                }
                row[ox * ch + c] = acc.clamp(0.0, max);
            }
        }
    });
    image_of(img, &r, ow, oh, &out)
}

/// A new canvas `w` × `h`: the picture placed by `anchor` (0..=8, row by row
/// from top left; 4 = centre). Space that is added is filled with `color`
/// (opaque); a smaller canvas trims the picture from the other sides.
pub fn canvas(img: &Image, w: u32, h: u32, anchor: u8, color: [u8; 3]) -> Result<Image, String> {
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS || anchor > 8 {
        return Err("That canvas size is not possible.".into());
    }
    let r = raster(img);
    let (nw, nh) = (w as usize, h as usize);
    let off = |old: usize, new: usize, a: u8| -> isize {
        match a {
            0 => 0,
            1 => (new as isize - old as isize) / 2,
            _ => new as isize - old as isize,
        }
    };
    let (ox, oy) = (off(r.w, nw, anchor % 3), off(r.h, nh, anchor / 3));
    let scale = if r.sample == SampleType::U8 { 1.0 } else { 257.0 };
    let mut fill: Vec<f32> = color.iter().map(|&c| f32::from(c) * scale).collect();
    if r.ch == 4 {
        fill.push(255.0 * scale);
    }
    let ch = r.ch;
    let mut out = vec![0.0f32; nw * nh * ch];
    out.par_chunks_mut(nw * ch).enumerate().for_each(|(y, row)| {
        let sy = y as isize - oy;
        for x in 0..nw {
            let sx = x as isize - ox;
            let px = &mut row[x * ch..(x + 1) * ch];
            if sx >= 0 && sy >= 0 && (sx as usize) < r.w && (sy as usize) < r.h {
                let i = (sy as usize * r.w + sx as usize) * ch;
                px.copy_from_slice(&r.v[i..i + ch]);
            } else {
                px.copy_from_slice(&fill);
            }
        }
    });
    image_of(img, &r, nw, nh, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> Image {
        let d: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i / w * 3 % 256) as u8, 128]).collect();
        Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, d).unwrap()
    }

    #[test]
    fn straighten_keeps_the_shape_and_has_no_blank_corners() {
        // A white picture: after turning, every pixel must still be white (no fill corners).
        let p = Image::from_raw(300, 200, ChannelLayout::Rgb, SampleType::U8, vec![255; 300 * 200 * 3]).unwrap();
        for deg in [-30.0, -5.5, 0.3, 12.0, 45.0] {
            let o = straighten(&p, deg).unwrap();
            let (w, h) = o.dimensions();
            assert!(((w as f64 / h as f64) - 1.5).abs() < 0.02, "{deg}: {w}x{h} keeps 3:2");
            assert!(o.data().iter().all(|&v| v >= 254), "{deg}: no blank corners");
        }
        let k = straighten_scale(300.0, 200.0, 10f64.to_radians());
        assert!(k < 1.0 && k > 0.7);
        assert!(straighten(&p, 50.0).is_err() && straighten(&p, f64::NAN).is_err());
        assert_eq!(straighten(&p, 0.0).unwrap().data(), p.data());
    }

    #[test]
    fn positive_angle_turns_clockwise() {
        // A dark dot right of the centre on a white picture moves down when turned clockwise.
        let (w, h) = (101u32, 101u32);
        let mut d = vec![255u8; (w * h * 3) as usize];
        for y in 48..53 {
            for x in 78..83 {
                let i = ((y * w + x) * 3) as usize;
                d[i..i + 3].fill(0);
            }
        }
        let p = Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, d).unwrap();
        let o = straighten(&p, 30.0).unwrap();
        let (ow, oh) = o.dimensions();
        let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
        for y in 0..oh {
            for x in 0..ow {
                if o.data()[((y * ow + x) * 3) as usize] < 100 {
                    (sx, sy, n) = (sx + f64::from(x), sy + f64::from(y), n + 1.0);
                }
            }
        }
        assert!(n > 0.0, "dot still inside");
        let (cx, cy) = (sx / n - f64::from(ow) / 2.0, sy / n - f64::from(oh) / 2.0);
        let angle = cy.atan2(cx).to_degrees();
        assert!((angle - 30.0).abs() < 4.0, "dot at {angle}° (expected 30° below the right)");
    }

    #[test]
    fn sixteen_bit_and_alpha_survive() {
        let d: Vec<u16> = (0..40 * 30 * 4).map(|i| (i * 31 % 65535) as u16).collect();
        let p = Image::from_u16(40, 30, ChannelLayout::Rgba, &d).unwrap();
        let o = straighten(&p, 7.0).unwrap();
        assert_eq!((o.sample_type(), o.layout()), (SampleType::U16, ChannelLayout::Rgba));
        let c = canvas(&p, 60, 40, 4, [255, 0, 0]).unwrap();
        assert_eq!((c.sample_type(), c.layout(), c.dimensions()), (SampleType::U16, ChannelLayout::Rgba, (60, 40)));
    }

    #[test]
    fn canvas_adds_a_border_or_trims_by_anchor() {
        let p = img(10, 8);
        let b = canvas(&p, 14, 12, 4, [255, 255, 255]).unwrap();
        assert_eq!(b.dimensions(), (14, 12));
        let px = |i: &Image, x: u32, y: u32| {
            let w = i.dimensions().0;
            i.data()[((y * w + x) * 3) as usize..((y * w + x) * 3 + 3) as usize].to_vec()
        };
        assert_eq!(px(&b, 0, 0), vec![255, 255, 255], "border colour");
        assert_eq!(px(&b, 2, 2), px(&p, 0, 0), "picture centred");
        let t = canvas(&p, 6, 8, 2, [0, 0, 0]).unwrap(); // anchor top right: keep the right part
        assert_eq!(px(&t, 0, 0), px(&p, 4, 0));
        assert!(canvas(&p, 0, 5, 4, [0; 3]).is_err() && canvas(&p, 5, 5, 9, [0; 3]).is_err());
    }
}
