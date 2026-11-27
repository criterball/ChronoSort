//! Finishing effects: a frame (border, mat, instant photo, rounded corners,
//! drop shadow, vignette) and a watermark (a text or logo layer).
//!
//! They are applied last — after the colour and light adjustments — so a white
//! frame stays white whatever the look. Sizes are percentages of the picture's
//! shorter side (watermark width: of the picture's width), so the 1600 px
//! preview looks like the saved full-size picture.
//!
//! Order: watermark on the picture, then the frame around (or over the edges of) it.

use crate::retouch::{Buf, buf, image_of};
use cs_image::Image;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frame {
    #[default]
    None,
    /// Plain border in `color` all around.
    Border,
    /// Mat (passe-partout): border in `color` with a thin `accent` line at the picture's edge.
    Matte,
    /// Instant-photo look: a narrow border on three sides, a wide one below.
    Instant,
    /// Rounded corners; the cut corners are filled with `color`. Same size.
    Rounded,
    /// Soft drop shadow (`accent`) on a `color` background.
    Shadow,
    /// Edges fade towards `color` (dark or light vignette). Same size.
    Vignette,
}

/// Where and how big the watermark layer is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Placement {
    pub on: bool,
    /// 0..=8, row by row from the top left (4 = centre).
    pub anchor: u8,
    /// Width of the watermark, % of the picture's width (2..=90).
    pub width: f32,
    /// Distance from the picture's edge, % of its shorter side (0..=20).
    pub margin: f32,
    /// 0..=100.
    pub opacity: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Placement { on: false, anchor: 8, width: 25.0, margin: 3.0, opacity: 70.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Effects {
    pub frame: Frame,
    /// Frame size, % of the shorter side (0.5..=30). Vignette: its strength.
    pub size: f32,
    pub color: [u8; 3],
    pub accent: [u8; 3],
    pub watermark: Placement,
}

impl Default for Effects {
    fn default() -> Self {
        Effects {
            frame: Frame::None,
            size: 5.0,
            color: [255, 255, 255],
            accent: [0, 0, 0],
            watermark: Placement::default(),
        }
    }
}

impl Effects {
    /// True when nothing would change (the watermark only counts when a layer is set).
    pub fn is_none(&self, has_layer: bool) -> bool {
        self.frame == Frame::None && !(self.watermark.on && has_layer)
    }

    /// The same settings with every number in its allowed range.
    pub fn clamped(mut self) -> Self {
        let fix = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.size = fix(self.size, 0.5, 30.0, 5.0);
        let w = &mut self.watermark;
        w.anchor = w.anchor.min(8);
        w.width = fix(w.width, 2.0, 90.0, 25.0);
        w.margin = fix(w.margin, 0.0, 20.0, 3.0);
        w.opacity = fix(w.opacity, 0.0, 100.0, 70.0);
        self
    }
}

/// A watermark picture: straight RGBA, 0..1, at most [`Layer::MAX_SIDE`] wide or high.
#[derive(Clone, Debug)]
pub struct Layer {
    pub w: usize,
    pub h: usize,
    /// Premultiplied RGBA.
    v: Vec<f32>,
}

impl Layer {
    pub const MAX_SIDE: u32 = 2400;

    /// From PNG bytes (a text watermark drawn by the app).
    pub fn from_png(png: &[u8]) -> Result<Layer, String> {
        let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .map_err(|e| format!("The watermark could not be read: {e}"))?;
        Self::from_dynamic(img)
    }

    /// From any picture ChronoSort can open (a logo). Pictures without
    /// transparency are used as they are.
    pub fn from_image(img: &Image) -> Result<Layer, String> {
        let b = buf(img);
        let mut rgba = image::Rgba32FImage::new(b.w as u32, b.h as u32);
        for (p, s) in rgba.pixels_mut().zip(b.v.chunks(b.ch)) {
            p.0 = [s[0], s[1], s[2], if b.ch == 4 { s[3] } else { 1.0 }];
        }
        Self::from_dynamic(image::DynamicImage::ImageRgba32F(rgba))
    }

    fn from_dynamic(mut img: image::DynamicImage) -> Result<Layer, String> {
        if img.width() == 0 || img.height() == 0 {
            return Err("The watermark is empty.".into());
        }
        if img.width().max(img.height()) > Self::MAX_SIDE {
            img = img.resize(Self::MAX_SIDE, Self::MAX_SIDE, image::imageops::FilterType::Triangle);
        }
        let rgba = img.to_rgba32f();
        let (w, h) = (rgba.width() as usize, rgba.height() as usize);
        let mut v = rgba.into_raw();
        for p in v.chunks_mut(4) {
            let a = p[3].clamp(0.0, 1.0);
            p[3] = a;
            for c in &mut p[..3] {
                *c = c.clamp(0.0, 1.0) * a;
            }
        }
        if v.chunks(4).all(|p| p[3] <= 0.0) {
            return Err("The watermark is fully transparent.".into());
        }
        Ok(Layer { w, h, v })
    }

    /// Premultiplied RGBA at (x, y) in layer pixels, bilinear, transparent outside.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let mut out = [0.0; 4];
        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                let (sx, sy) = (x0 as isize + dx, y0 as isize + dy);
                if sx < 0 || sy < 0 || sx as usize >= self.w || sy as usize >= self.h {
                    continue;
                }
                let i = (sy as usize * self.w + sx as usize) * 4;
                let wgt = wx * wy;
                for c in 0..4 {
                    out[c] += self.v[i + c] * wgt;
                }
            }
        }
        out
    }
}

/// The picture's size after the effects (frames that add a border make it bigger).
pub fn output_size(w: u32, h: u32, fx: &Effects) -> (u32, u32) {
    let fx = fx.clamped();
    let d = ((w.min(h) as f32 * fx.size / 100.0).round() as u32).max(1);
    match fx.frame {
        Frame::Border | Frame::Matte | Frame::Shadow => (w + 2 * d, h + 2 * d),
        Frame::Instant => (w + 2 * d, h + d + (d as f32 * 3.5).round() as u32),
        Frame::None | Frame::Rounded | Frame::Vignette => (w, h),
    }
}

/// Apply the effects. `layer` is the watermark picture, if one is set.
pub fn apply(img: &Image, fx: &Effects, layer: Option<&Layer>) -> Result<Image, String> {
    let fx = fx.clamped();
    if fx.is_none(layer.is_some()) {
        return Ok(img.clone());
    }
    let mut b = buf(img);
    if fx.watermark.on
        && let Some(l) = layer
    {
        watermark(&mut b, l, &fx.watermark);
    }
    let b = frame(b, &fx);
    image_of(img, &b)
}

fn watermark(b: &mut Buf, l: &Layer, p: &Placement) {
    let (w, h) = (b.w as f32, b.h as f32);
    let tw = (w * p.width / 100.0).max(1.0);
    let s = tw / l.w as f32; // picture px per layer px
    let th = l.h as f32 * s;
    let m = w.min(h) * p.margin / 100.0;
    let pos = |a: u8, size: f32, room: f32| match a {
        0 => m,
        1 => (room - size) / 2.0,
        _ => room - size - m,
    };
    let (x0, y0) = (pos(p.anchor % 3, tw, w), pos(p.anchor / 3, th, h));
    let op = p.opacity / 100.0;
    let (bw, ch) = (b.w, b.ch);
    let rows = (y0.floor().max(0.0) as usize)..((y0 + th).ceil().min(h) as usize);
    let cols = (x0.floor().max(0.0) as usize)..((x0 + tw).ceil().min(w) as usize);
    if rows.is_empty() || cols.is_empty() {
        return;
    }
    let start = rows.start;
    b.v.par_chunks_mut(bw * ch).skip(start).take(rows.len()).enumerate().for_each(|(i, row)| {
        let py = (start + i) as f32 + 0.5;
        let ly = (py - y0) / s;
        for x in cols.clone() {
            let lx = (x as f32 + 0.5 - x0) / s;
            let [r, g, bl, a] = l.sample(lx, ly);
            let a = a * op;
            if a <= 0.0 {
                continue;
            }
            let px = &mut row[x * ch..(x + 1) * ch];
            for (c, v) in [r, g, bl].into_iter().enumerate() {
                px[c] = v * op + px[c] * (1.0 - a);
            }
            if ch == 4 {
                px[3] = a + px[3] * (1.0 - a);
            }
        }
    });
}

fn rgb(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| f32::from(v) / 255.0)
}

/// A bigger canvas filled with `color`, the picture copied in at (ox, oy).
fn grow(b: &Buf, nw: usize, nh: usize, ox: usize, oy: usize, color: [f32; 3]) -> Buf {
    let ch = b.ch;
    let mut fill = color.to_vec();
    if ch == 4 {
        fill.push(1.0);
    }
    let mut v = vec![0.0f32; nw * nh * ch];
    v.par_chunks_mut(nw * ch).enumerate().for_each(|(y, row)| {
        for px in row.chunks_mut(ch) {
            px.copy_from_slice(&fill);
        }
        if y >= oy && y < oy + b.h {
            let src = &b.v[(y - oy) * b.w * ch..(y - oy + 1) * b.w * ch];
            row[ox * ch..(ox + b.w) * ch].copy_from_slice(src);
        }
    });
    Buf { w: nw, h: nh, ch, v, layout: b.layout, sample: b.sample }
}

/// Error function (Abramowitz & Stegun 7.1.26, |error| < 1.5e-7).
fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let y = 1.0
        - (((((1.061_405_4 * t - 1.453_152_1) * t) + 1.421_413_7) * t - 0.284_496_74) * t + 0.254_829_6)
            * t
            * (-x * x).exp();
    y.copysign(x)
}

fn frame(b: Buf, fx: &Effects) -> Buf {
    let short = b.w.min(b.h) as f32;
    let px = |pct: f32| ((short * pct / 100.0).round() as usize).max(1);
    let color = rgb(fx.color);
    let accent = rgb(fx.accent);
    let ch = b.ch;
    match fx.frame {
        Frame::None => b,
        Frame::Border => {
            let d = px(fx.size);
            grow(&b, b.w + 2 * d, b.h + 2 * d, d, d, color)
        }
        Frame::Instant => {
            let d = px(fx.size);
            let bottom = (d as f32 * 3.5).round() as usize;
            grow(&b, b.w + 2 * d, b.h + d + bottom, d, d, color)
        }
        Frame::Matte => {
            let d = px(fx.size);
            let line = (d / 14).max(1);
            let mut o = grow(&b, b.w + 2 * d, b.h + 2 * d, d, d, color);
            // The line: a ring `line` px wide just outside the picture.
            let (x0, y0, x1, y1) = (d - line, d - line, d + b.w + line, d + b.h + line);
            let ow = o.w;
            o.v.par_chunks_mut(ow * ch).enumerate().for_each(|(y, row)| {
                if y < y0 || y >= y1 {
                    return;
                }
                let inside_y = y >= d && y < d + b.h;
                for x in x0..x1 {
                    if inside_y && x >= d && x < d + b.w {
                        continue;
                    }
                    row[x * ch..x * ch + 3].copy_from_slice(&accent);
                }
            });
            o
        }
        Frame::Rounded => {
            let mut o = b;
            let r = (short * fx.size / 100.0).clamp(1.0, short / 2.0);
            let (w, h) = (o.w as f32, o.h as f32);
            let ow = o.w;
            o.v.par_chunks_mut(ow * ch).enumerate().for_each(|(y, row)| {
                let py = y as f32 + 0.5;
                let cy = if py < r {
                    r
                } else if py > h - r {
                    h - r
                } else {
                    return;
                };
                for x in 0..ow {
                    let pxf = x as f32 + 0.5;
                    let cx = if pxf < r {
                        r
                    } else if pxf > w - r {
                        w - r
                    } else {
                        continue;
                    };
                    let dist = (pxf - cx).hypot(py - cy);
                    let cover = (r - dist + 0.5).clamp(0.0, 1.0); // 1 inside, anti-aliased edge
                    if cover >= 1.0 {
                        continue;
                    }
                    let p = &mut row[x * ch..(x + 1) * ch];
                    for c in 0..3 {
                        p[c] = p[c] * cover + color[c] * (1.0 - cover);
                    }
                    if ch == 4 {
                        p[3] = 1.0;
                    }
                }
            });
            o
        }
        Frame::Shadow => {
            let d = px(fx.size);
            let off = d as f32 * 0.35;
            let sigma = (d as f32 * 0.45).max(0.8);
            let mut o = grow(&b, b.w + 2 * d, b.h + 2 * d, d, d, color);
            // Shadow rectangle = the picture moved by `off`, blurred (separable erf).
            let (sx0, sy0) = (d as f32 + off, d as f32 + off);
            let (sx1, sy1) = (sx0 + b.w as f32, sy0 + b.h as f32);
            let k = std::f32::consts::SQRT_2 * sigma;
            let edge = |p: f32, a: f32, z: f32| 0.5 * (erf((p - a) / k) - erf((p - z) / k));
            let ow = o.w;
            let ax: Vec<f32> = (0..ow).map(|x| edge(x as f32 + 0.5, sx0, sx1)).collect();
            const STRENGTH: f32 = 0.6;
            o.v.par_chunks_mut(ow * ch).enumerate().for_each(|(y, row)| {
                let ay = edge(y as f32 + 0.5, sy0, sy1);
                let in_y = y >= d && y < d + b.h;
                for x in 0..ow {
                    if in_y && x >= d && x < d + b.w {
                        continue; // the picture itself
                    }
                    let a = ay * ax[x] * STRENGTH;
                    if a <= 0.0 {
                        continue;
                    }
                    let p = &mut row[x * ch..(x + 1) * ch];
                    for c in 0..3 {
                        p[c] = accent[c] * a + p[c] * (1.0 - a);
                    }
                }
            });
            o
        }
        Frame::Vignette => {
            let mut o = b;
            let amount = (fx.size / 30.0).clamp(0.0, 1.0).sqrt();
            let (hw, hh) = (o.w as f32 / 2.0, o.h as f32 / 2.0);
            let ow = o.w;
            o.v.par_chunks_mut(ow * ch).enumerate().for_each(|(y, row)| {
                let dy = (y as f32 + 0.5 - hh) / hh;
                for x in 0..ow {
                    let dx = (x as f32 + 0.5 - hw) / hw;
                    let d = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2; // 0 centre, 1 corner
                    let t = ((d - 0.45) / 0.55).clamp(0.0, 1.0);
                    let a = t * t * (3.0 - 2.0 * t) * amount;
                    if a <= 0.0 {
                        continue;
                    }
                    let p = &mut row[x * ch..(x + 1) * ch];
                    for c in 0..3 {
                        p[c] = color[c] * a + p[c] * (1.0 - a);
                    }
                }
            });
            o
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cs_image::{ChannelLayout, SampleType};

    fn flat(w: u32, h: u32, c: [u8; 3]) -> Image {
        let d: Vec<u8> = (0..w * h).flat_map(|_| c).collect();
        Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, d).unwrap()
    }
    fn px(i: &Image, x: u32, y: u32) -> [u8; 3] {
        let w = i.dimensions().0;
        let o = ((y * w + x) * 3) as usize;
        [i.data()[o], i.data()[o + 1], i.data()[o + 2]]
    }
    fn fx(frame: Frame, size: f32) -> Effects {
        Effects { frame, size, ..Effects::default() }
    }

    #[test]
    fn none_is_a_copy() {
        let p = flat(20, 10, [10, 20, 30]);
        assert!(fx(Frame::None, 5.0).is_none(false));
        assert_eq!(apply(&p, &fx(Frame::None, 5.0), None).unwrap().data(), p.data());
        // A watermark that is switched on but has no picture changes nothing.
        let e = Effects { watermark: Placement { on: true, ..Placement::default() }, ..Effects::default() };
        assert!(e.is_none(false));
    }

    #[test]
    fn border_and_instant_grow_the_canvas() {
        let p = flat(200, 100, [10, 20, 30]);
        let o = apply(&p, &fx(Frame::Border, 10.0), None).unwrap();
        assert_eq!(o.dimensions(), (220, 120));
        assert_eq!(px(&o, 0, 0), [255, 255, 255]);
        assert_eq!(px(&o, 10, 10), [10, 20, 30]);
        assert_eq!(px(&o, 209, 109), [10, 20, 30]);
        assert_eq!(px(&o, 210, 110), [255, 255, 255]);
        let o = apply(&p, &fx(Frame::Instant, 10.0), None).unwrap();
        assert_eq!(o.dimensions(), (220, 100 + 10 + 35));
        for f in
            [Frame::None, Frame::Border, Frame::Matte, Frame::Instant, Frame::Rounded, Frame::Shadow, Frame::Vignette]
        {
            for size in [0.5, 3.3, 10.0, 30.0] {
                let e = fx(f, size);
                assert_eq!(apply(&p, &e, None).unwrap().dimensions(), output_size(200, 100, &e), "{f:?} {size}");
            }
        }
    }

    #[test]
    fn matte_has_a_line_at_the_picture_edge() {
        let p = flat(300, 300, [100, 100, 100]);
        let o = apply(&p, &fx(Frame::Matte, 10.0), None).unwrap(); // border 30, line 2
        assert_eq!(o.dimensions(), (360, 360));
        assert_eq!(px(&o, 5, 5), [255, 255, 255]);
        assert_eq!(px(&o, 29, 100), [0, 0, 0]);
        assert_eq!(px(&o, 28, 100), [0, 0, 0]);
        assert_eq!(px(&o, 27, 100), [255, 255, 255]);
        assert_eq!(px(&o, 30, 100), [100, 100, 100]);
    }

    #[test]
    fn rounded_fills_only_the_corners() {
        let p = flat(100, 100, [0, 0, 0]);
        let o = apply(&p, &fx(Frame::Rounded, 20.0), None).unwrap();
        assert_eq!(o.dimensions(), (100, 100));
        assert_eq!(px(&o, 0, 0), [255, 255, 255]);
        assert_eq!(px(&o, 99, 99), [255, 255, 255]);
        assert_eq!(px(&o, 50, 0), [0, 0, 0]); // straight edge untouched
        assert_eq!(px(&o, 50, 50), [0, 0, 0]);
    }

    #[test]
    fn shadow_is_darker_below_right_than_top_left() {
        let p = flat(100, 100, [200, 0, 0]);
        let o = apply(&p, &fx(Frame::Shadow, 10.0), None).unwrap();
        assert_eq!(o.dimensions(), (120, 120));
        assert_eq!(px(&o, 50, 50), [200, 0, 0]);
        let below = px(&o, 60, 113)[0];
        let above = px(&o, 60, 6)[0];
        assert!(below < above && below < 200, "below {below} above {above}");
        assert_eq!(px(&o, 1, 1), [255, 255, 255]);
    }

    #[test]
    fn vignette_darkens_corners_not_the_middle() {
        let p = flat(100, 80, [200, 200, 200]);
        let mut e = fx(Frame::Vignette, 15.0);
        e.color = [0, 0, 0];
        let o = apply(&p, &e, None).unwrap();
        assert_eq!(px(&o, 50, 40), [200, 200, 200]);
        assert!(px(&o, 0, 0)[0] < 120);
    }

    #[test]
    fn watermark_lands_bottom_right_with_its_opacity() {
        // A 10×10 opaque white layer, 20 % of a 200×100 picture's width, margin 0.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let l = Layer::from_png(&png).unwrap();
        let p = flat(200, 100, [0, 0, 0]);
        let mut e = Effects {
            watermark: Placement { on: true, anchor: 8, width: 20.0, margin: 0.0, opacity: 50.0 },
            ..Effects::default()
        };
        let o = apply(&p, &e, Some(&l)).unwrap();
        assert_eq!(o.dimensions(), (200, 100));
        let v = px(&o, 190, 90)[0];
        assert!((126..=129).contains(&v), "{v}"); // 50 % white over black
        assert_eq!(px(&o, 150, 70), [0, 0, 0]); // outside the 40×40 box
        assert_eq!(px(&o, 10, 10), [0, 0, 0]);
        // The frame comes after the watermark: the mark stays on the picture.
        e.frame = Frame::Border;
        e.size = 10.0;
        let o = apply(&p, &e, Some(&l)).unwrap();
        let v = px(&o, 200, 100)[0];
        assert!((126..=129).contains(&v), "{v}");
    }

    #[test]
    fn layers_refuse_empty_or_invisible_pictures() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 0]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert!(Layer::from_png(&png).is_err());
        assert!(Layer::from_png(b"not a png").is_err());
        // A logo without transparency is used as it is.
        assert!(Layer::from_image(&flat(8, 8, [1, 2, 3])).is_ok());
    }

    #[test]
    fn sixteen_bit_and_alpha_pictures_keep_their_kind() {
        let d: Vec<u8> = (0..16 * 16).flat_map(|_| 30000u16.to_ne_bytes().repeat(4)).collect();
        let p = Image::from_raw(16, 16, ChannelLayout::Rgba, SampleType::U16, d).unwrap();
        let o = apply(&p, &fx(Frame::Border, 10.0), None).unwrap();
        assert_eq!(o.dimensions(), (20, 20)); // 10 % of 16 px rounds to 2
        assert_eq!(o.layout(), ChannelLayout::Rgba);
        assert_eq!(o.sample_type(), SampleType::U16);
    }
}
