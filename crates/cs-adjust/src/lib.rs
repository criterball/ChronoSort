//! `cs-adjust`: colour and light adjustments for a photo, like the Basic,
//! Tone Curve, Detail and Effects panels of a raw developer, for already
//! rendered pictures (JPEG, PNG, TIFF, HEIC, WebP).
//!
//! Built on photocraft's develop pipeline pieces (craft-libs, MIT OR Apache-2.0):
//! * contrast / whites / blacks: `craft_develop::tone::ToneMap::display`, the
//!   tone map for display-referred sources (identity at neutral settings);
//! * the region tone curve (shadows / darks / lights / highlights):
//!   `craft_develop::curve::ParametricCurve`; levels and the point curve:
//!   `craft_color_transform::curve::{MonotoneCurve, Lut1}`;
//! * sharpening: `craft_develop::finish::{sharpen_term, sharpen_limit}`;
//! * colour: OkLab / OkLCh (`craft_color_transform::oklab`) and
//!   `craft_develop::finish::gamut_map`;
//! * slider ranges and defaults: `craft_develop_model::CONTROLS`.
//!
//! ChronoSort's own parts: white balance and exposure in linear light,
//! highlights / shadows / clarity / texture as edge-aware local tone (guided
//! filter in log luminance, so no halos), dehaze, noise reduction, colour
//! balance, looks (black & white, sepia, negative) and Auto.
//!
//! Spatial sizes are fractions of the picture's long side, so a preview looks
//! like the saved full-size picture. Pixel-scale steps (sharpening, noise
//! reduction) are in pixels, as in every raw developer.

pub mod effects;
pub mod filter;
pub mod retouch;

use craft_color_transform::curve::{Lut1, MonotoneCurve};
use craft_color_transform::oklab;
use craft_develop::curve::ParametricCurve;
use craft_develop::finish::{gamut_map, sharpen_limit, sharpen_term};
use craft_develop::tone::ToneMap;
use cs_image::{ChannelLayout, Image, SampleType};
use filter::{Plane, gauss, guided};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// A finished look applied last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Look {
    #[default]
    None,
    Grayscale,
    Sepia,
    Negative,
}

/// Every adjustment. All zero (and levels 0 / 1 / gamma 1) = the photo unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Adjust {
    // Light (craft ids light.*)
    pub exposure: f64,
    pub contrast: f64,
    pub highlights: f64,
    pub shadows: f64,
    pub whites: f64,
    pub blacks: f64,
    // Colour: relative white balance for rendered photos (-100..100), vibrance, saturation.
    pub temp: f64,
    pub tint: f64,
    pub vibrance: f64,
    pub saturation: f64,
    // Presence
    pub texture: f64,
    pub clarity: f64,
    pub dehaze: f64,
    // Levels (encoded 0..1; gamma > 1 brightens the midtones, as in Photoshop)
    pub in_black: f64,
    pub in_white: f64,
    pub gamma: f64,
    pub out_black: f64,
    pub out_white: f64,
    // Region tone curve (craft ids curve.*) and an optional point curve (0..1 pairs).
    pub curve_shadows: f64,
    pub curve_darks: f64,
    pub curve_lights: f64,
    pub curve_highlights: f64,
    pub points: Vec<[f64; 2]>,
    // Colour balance: [cyan-red, magenta-green, yellow-blue] per tonal range, -100..100.
    pub balance_shadows: [f64; 3],
    pub balance_midtones: [f64; 3],
    pub balance_highlights: [f64; 3],
    // Detail (craft ids detail.*)
    pub sharpen_amount: f64,
    pub sharpen_radius: f64,
    pub sharpen_detail: f64,
    pub sharpen_masking: f64,
    pub nr_luminance: f64,
    pub nr_color: f64,
    /// Soft blur 0..100.
    pub blur: f64,
    pub look: Look,
}

impl Default for Adjust {
    fn default() -> Self {
        Adjust {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            temp: 0.0,
            tint: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            in_black: 0.0,
            in_white: 1.0,
            gamma: 1.0,
            out_black: 0.0,
            out_white: 1.0,
            curve_shadows: 0.0,
            curve_darks: 0.0,
            curve_lights: 0.0,
            curve_highlights: 0.0,
            points: vec![],
            balance_shadows: [0.0; 3],
            balance_midtones: [0.0; 3],
            balance_highlights: [0.0; 3],
            sharpen_amount: 0.0,
            sharpen_radius: 1.0,
            sharpen_detail: 25.0,
            sharpen_masking: 0.0,
            nr_luminance: 0.0,
            nr_color: 0.0,
            blur: 0.0,
            look: Look::None,
        }
    }
}

impl Adjust {
    pub fn is_neutral(&self) -> bool {
        let d = Adjust::default();
        // Point curves that are the identity count as no curve.
        let points_neutral = self.points.is_empty() || MonotoneCurve::new(&self.pairs()).is_identity();
        Adjust { points: vec![], ..self.clone() } == Adjust { points: vec![], ..d } && points_neutral
    }

    fn pairs(&self) -> Vec<(f64, f64)> {
        self.points.iter().map(|p| (p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0))).collect()
    }

    /// Every number clamped to its slider range (settings from the UI are not trusted).
    pub fn clamped(&self) -> Adjust {
        let c = |v: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
        let b = |a: [f64; 3]| a.map(|v| c(v, -100.0, 100.0));
        let mut a = self.clone();
        a.exposure = c(a.exposure, -5.0, 5.0);
        for v in [
            &mut a.contrast,
            &mut a.highlights,
            &mut a.shadows,
            &mut a.whites,
            &mut a.blacks,
            &mut a.temp,
            &mut a.tint,
            &mut a.vibrance,
            &mut a.saturation,
            &mut a.texture,
            &mut a.clarity,
            &mut a.dehaze,
            &mut a.curve_shadows,
            &mut a.curve_darks,
            &mut a.curve_lights,
            &mut a.curve_highlights,
        ] {
            *v = c(*v, -100.0, 100.0);
        }
        a.in_black = c(a.in_black, 0.0, 0.95);
        a.in_white = c(a.in_white, a.in_black + 0.04, 1.0).max(a.in_black + 0.04);
        a.gamma = if a.gamma.is_finite() { a.gamma.clamp(0.1, 9.99) } else { 1.0 };
        a.out_black = c(a.out_black, 0.0, 1.0);
        a.out_white = c(a.out_white, 0.0, 1.0);
        a.points.truncate(16);
        (a.balance_shadows, a.balance_midtones, a.balance_highlights) =
            (b(a.balance_shadows), b(a.balance_midtones), b(a.balance_highlights));
        a.sharpen_amount = c(a.sharpen_amount, 0.0, 150.0);
        a.sharpen_radius = c(a.sharpen_radius, 0.5, 3.0).max(0.5);
        a.sharpen_detail = c(a.sharpen_detail, 0.0, 100.0);
        a.sharpen_masking = c(a.sharpen_masking, 0.0, 100.0);
        a.nr_luminance = c(a.nr_luminance, 0.0, 100.0);
        a.nr_color = c(a.nr_color, 0.0, 100.0);
        a.blur = c(a.blur, 0.0, 100.0);
        a
    }
}

/// One slider for the UI.
#[derive(Clone, Debug, Serialize)]
pub struct Control {
    /// Field name in [`Adjust`] (camelCase).
    pub key: &'static str,
    pub label: &'static str,
    pub section: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub step: f64,
}

/// The sliders, in panel order. Ranges, defaults and steps come from craft's
/// `CONTROLS` wherever the control exists there.
pub fn controls() -> Vec<Control> {
    let spec =
        |key: &'static str, craft: &str, label: &'static str, section: &'static str, own: (f64, f64, f64, f64)| {
            let (min, max, default, step) =
                craft_develop_model::controls::find(craft).map(|s| (s.min, s.max, s.default, s.step)).unwrap_or(own);
            Control { key, label, section, min, max, default, step }
        };
    let o = (-100.0, 100.0, 0.0, 1.0);
    vec![
        spec("temp", "", "Temperature", "Colour", o),
        spec("tint", "", "Tint", "Colour", o),
        spec("vibrance", "color.vibrance", "Vibrance", "Colour", o),
        spec("saturation", "color.saturation", "Saturation", "Colour", o),
        spec("exposure", "light.exposure", "Exposure", "Light", (-5.0, 5.0, 0.0, 0.01)),
        spec("contrast", "light.contrast", "Contrast", "Light", o),
        spec("highlights", "light.highlights", "Highlights", "Light", o),
        spec("shadows", "light.shadows", "Shadows", "Light", o),
        spec("whites", "light.whites", "Whites", "Light", o),
        spec("blacks", "light.blacks", "Blacks", "Light", o),
        spec("texture", "effects.texture", "Texture", "Presence", o),
        spec("clarity", "effects.clarity", "Clarity", "Presence", o),
        spec("dehaze", "effects.dehaze", "Dehaze", "Presence", o),
        spec("curveHighlights", "curve.highlights", "Highlights", "Curve", o),
        spec("curveLights", "curve.lights", "Lights", "Curve", o),
        spec("curveDarks", "curve.darks", "Darks", "Curve", o),
        spec("curveShadows", "curve.shadows", "Shadows", "Curve", o),
        spec("sharpenAmount", "detail.sharpenAmount", "Sharpen", "Detail", (0.0, 150.0, 0.0, 1.0)),
        spec("sharpenRadius", "detail.sharpenRadius", "Radius", "Detail", (0.5, 3.0, 1.0, 0.1)),
        spec("sharpenDetail", "detail.sharpenDetail", "Detail", "Detail", (0.0, 100.0, 25.0, 1.0)),
        spec("sharpenMasking", "detail.sharpenMasking", "Masking", "Detail", (0.0, 100.0, 0.0, 1.0)),
        spec("nrLuminance", "detail.nrLuminance", "Noise reduction", "Detail", (0.0, 100.0, 0.0, 1.0)),
        spec("nrColor", "detail.nrColor", "Colour noise", "Detail", (0.0, 100.0, 0.0, 1.0)),
        spec("blur", "", "Soft blur", "Detail", (0.0, 100.0, 0.0, 1.0)),
    ]
}

#[inline]
fn to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

#[inline]
fn to_srgb(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Fast linear → sRGB for the pipeline (table, interpolated; exact in the linear toe).
fn enc(v: f32) -> f32 {
    static T: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    const N: usize = 1 << 14;
    let t = T.get_or_init(|| (0..=N).map(|i| to_srgb(i as f32 / N as f32)).collect());
    if v >= 1.0 {
        return to_srgb(v);
    }
    let f = v.max(0.0) * N as f32;
    let i = (f as usize).min(N - 1);
    t[i] + (t[i + 1] - t[i]) * (f - i as f32)
}

/// Fast sRGB → linear for the pipeline (table, interpolated).
fn dec(v: f32) -> f32 {
    static T: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    const N: usize = 1 << 12;
    let t = T.get_or_init(|| (0..=N).map(|i| to_linear(i as f32 / N as f32)).collect());
    if v >= 1.0 {
        return to_linear(v);
    }
    let f = v.max(0.0) * N as f32;
    let i = (f as usize).min(N - 1);
    t[i] + (t[i + 1] - t[i]) * (f - i as f32)
}

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn luma(p: [f32; 3]) -> f32 {
    LUMA[0] * p[0] + LUMA[1] * p[1] + LUMA[2] * p[2]
}

/// Linear RGB pixels plus size.
struct Px {
    w: usize,
    h: usize,
    p: Vec<[f32; 3]>,
}

impl Px {
    fn long(&self) -> f32 {
        self.w.max(self.h) as f32
    }
    fn lum_plane(&self) -> Plane {
        Plane::new(self.w, self.h, self.p.par_iter().map(|p| luma(*p)).collect())
    }
    /// Scale every pixel's colour so its luminance becomes `new[i]` (hue and
    /// saturation kept).
    fn set_lum(&mut self, old: &Plane, new: &[f32]) {
        self.p.par_iter_mut().zip(old.v.par_iter().zip(new.par_iter())).for_each(|(p, (&o, &n))| {
            let k = if o > 1e-6 { n / o } else { 1.0 };
            *p = if o > 1e-6 { p.map(|c| c * k) } else { [n; 3] };
        });
    }
}

fn log_lum(y: &Plane) -> Plane {
    Plane::new(y.w, y.h, y.v.par_iter().map(|v| v.max(1e-5).log2()).collect())
}

/// Apply the adjustments to `img` (any layout / depth); returns a picture of
/// the same size, sample depth, ICC profile and metadata.
pub fn render(img: &Image, adjust: &Adjust) -> Result<Image, String> {
    let a = adjust.clamped();
    if a.is_neutral() {
        return Ok(img.clone());
    }
    let (w, h) = img.dimensions();
    let alpha =
        matches!(img.layout(), ChannelLayout::GrayA | ChannelLayout::Rgba | ChannelLayout::CmykA | ChannelLayout::LabA);
    let layout = if alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    // 8-bit stays 8-bit; everything deeper is worked on and written as 16-bit.
    let wide = img.sample_type() != SampleType::U8;
    let sample = if wide { SampleType::U16 } else { SampleType::U8 };
    let src = img.converted(layout, sample);
    let ch = if alpha { 4 } else { 3 };
    let bytes = src.data();
    let (bps, max) = if wide { (2, 65535usize) } else { (1, 255usize) };
    let decode: Vec<f32> = (0..=max).map(|i| to_linear(i as f32 / max as f32)).collect();
    let sample_at = |b: &[u8], i: usize| -> usize {
        if wide { usize::from(u16::from_ne_bytes([b[2 * i], b[2 * i + 1]])) } else { usize::from(b[i]) }
    };
    let mut px = Px {
        w: w as usize,
        h: h as usize,
        p: bytes.par_chunks_exact(ch * bps).map(|c| [0, 1, 2].map(|k| decode[sample_at(c, k)])).collect(),
    };
    process(&mut px, &a);
    // Linear → sRGB through a fine table (exact in the linear toe, interpolated above).
    const ENC: usize = 1 << 16;
    let enc: Vec<f32> = (0..=ENC).map(|i| to_srgb(i as f32 / ENC as f32)).collect();
    let encode = |v: f32| -> f32 {
        let f = v.clamp(0.0, 1.0) * ENC as f32;
        let i = (f as usize).min(ENC - 1);
        enc[i] + (enc[i + 1] - enc[i]) * (f - i as f32)
    };
    let mut out = vec![0u8; bytes.len()];
    out.par_chunks_exact_mut(ch * bps).zip(px.p.par_iter()).zip(bytes.par_chunks_exact(ch * bps)).for_each(
        |((o, p), s)| {
            for k in 0..3 {
                let v = (encode(p[k]) * max as f32).round() as u16;
                if wide {
                    o[2 * k..2 * k + 2].copy_from_slice(&v.to_ne_bytes());
                } else {
                    o[k] = v as u8;
                }
            }
            if alpha {
                o[3 * bps..4 * bps].copy_from_slice(&s[3 * bps..4 * bps]);
            }
        },
    );
    Ok(Image::from_raw(w, h, layout, sample, out)
        .map_err(|e| e.to_string())?
        .with_icc(img.icc.clone())
        .with_meta(img.meta.clone()))
}

fn process(px: &mut Px, a: &Adjust) {
    let long = px.long();

    // 1. Noise reduction (first, as in a raw developer).
    if a.nr_color > 0.0 {
        // In OkLab: lightness kept, the two colour channels smoothed edge-aware, so
        // colour speckles go but real colour edges stay (no bleeding into neighbours).
        let k = (a.nr_color / 100.0) as f32;
        let lab: Vec<[f32; 3]> = px.p.par_iter().map(|p| oklab::from_linear_srgb(p.map(|v| v.max(0.0)))).collect();
        let r = 2 + (6.0 * k).round() as usize;
        let eps = (0.004 + 0.03 * k).powi(2);
        let ch = |c: usize| guided(&Plane::new(px.w, px.h, lab.iter().map(|l| l[c]).collect()), r, eps);
        let (ca, cb) = (ch(1), ch(2));
        px.p.par_iter_mut().enumerate().for_each(|(i, p)| {
            *p = gamut_map(oklab::to_linear_srgb([lab[i][0], ca.v[i], cb.v[i]]), LUMA).0;
        });
    }
    if a.nr_luminance > 0.0 {
        let k = (a.nr_luminance / 100.0) as f32;
        let y = px.lum_plane();
        let l = log_lum(&y);
        let r = 1 + (3.0 * k).round() as usize;
        let eps = 0.002 + 0.03 * k * k;
        let f = guided(&l, r, eps);
        let ny: Vec<f32> = f.v.par_iter().map(|v| v.exp2()).collect();
        px.set_lum(&y, &ny);
    }

    // 2. White balance (relative, for rendered photos) and exposure, in linear light.
    if a.temp != 0.0 || a.tint != 0.0 || a.exposure != 0.0 {
        let (t, m) = ((a.temp / 100.0) as f32, (a.tint / 100.0) as f32);
        let mut g = [(0.35 * t).exp2(), (-0.25 * m).exp2(), (-0.35 * t).exp2()];
        let y = luma(g);
        let e = (a.exposure as f32).exp2();
        g = g.map(|v| v / y * e);
        px.p.par_iter_mut().for_each(|p| *p = [p[0] * g[0], p[1] * g[1], p[2] * g[2]]);
    }

    // 3. Local tone: highlights, shadows, clarity, texture (edge-aware, in log luminance).
    if a.highlights != 0.0 || a.shadows != 0.0 || a.clarity != 0.0 || a.texture != 0.0 {
        let y = px.lum_plane();
        let l = log_lum(&y);
        let base = guided(&l, ((long * 0.02) as usize).max(2), 0.12);
        let fine = (a.texture != 0.0).then(|| guided(&l, ((long * 0.003) as usize).max(1), 0.02));
        let (hl, sh) = ((a.highlights / 100.0) as f32, (a.shadows / 100.0) as f32);
        let (cl, tx) = ((a.clarity / 100.0) as f32, (a.texture / 100.0) as f32);
        let grey = 0.18f32.log2();
        let ny: Vec<f32> = (0..l.v.len())
            .into_par_iter()
            .map(|i| {
                let (li, bi) = (l.v[i], base.v[i]);
                let ev = bi - grey;
                // Shadows lift / lower the dark regions, highlights the bright ones (up to ~1.6 stops).
                // Midtones (middle grey ± ~0.5 stop) are left mostly alone, as in Lightroom.
                let ws = 1.0 - smooth(-3.0, -0.4, ev);
                let wh = smooth(0.4, 2.4, ev);
                let nb = bi + 1.5 * sh * ws + 1.5 * hl * wh;
                // Clarity: mid-frequency local contrast, strongest in the midtones.
                let mid = 1.0 - smooth(1.0, 3.5, ev.abs());
                let mut nl = nb + (li - bi) * (1.0 + 0.9 * cl * mid);
                if let Some(f) = &fine {
                    nl += 0.8 * tx * (li - f.v[i]);
                }
                nl.exp2()
            })
            .collect();
        px.set_lum(&y, &ny);
    }

    // 4. Dehaze (dark channel prior, soft).
    if a.dehaze != 0.0 {
        dehaze(px, (a.dehaze / 100.0) as f32);
    }

    // 5. Contrast / whites / blacks: craft's display tone map on luminance.
    if a.contrast != 0.0 || a.whites != 0.0 || a.blacks != 0.0 {
        let tm = ToneMap::display(a.contrast, a.whites, a.blacks);
        let y = px.lum_plane();
        let ny: Vec<f32> = y.v.par_iter().map(|&v| tm.apply(v)).collect();
        px.set_lum(&y, &ny);
    }

    // 6. Levels, region curve and point curve: one LUT per channel on encoded values.
    let levels = a.in_black != 0.0 || a.in_white != 1.0 || a.gamma != 1.0 || a.out_black != 0.0 || a.out_white != 1.0;
    let region = a.curve_shadows != 0.0 || a.curve_darks != 0.0 || a.curve_lights != 0.0 || a.curve_highlights != 0.0;
    let point = (!a.points.is_empty()).then(|| MonotoneCurve::new(&a.pairs())).filter(|c| !c.is_identity());
    if levels || region || point.is_some() {
        let pc = region.then(|| {
            let tc = craft_develop_model::ToneCurve {
                shadows: a.curve_shadows,
                darks: a.curve_darks,
                lights: a.curve_lights,
                highlights: a.curve_highlights,
                ..Default::default()
            };
            ParametricCurve::new(&tc)
        });
        let (ib, iw, g, ob, ow) = (a.in_black, a.in_white, a.gamma, a.out_black, a.out_white);
        let lut = Lut1::from_fn(4096, |x| {
            let mut v = f64::from(x);
            if levels {
                let t = ((v - ib) / (iw - ib)).clamp(0.0, 1.0).powf(1.0 / g);
                v = ob + t * (ow - ob);
            }
            if let Some(pc) = &pc {
                v = pc.at(v);
            }
            if let Some(c) = &point {
                v = c.eval(v);
            }
            v.clamp(0.0, 1.0) as f32
        });
        px.p.par_iter_mut().for_each(|p| *p = p.map(|c| dec(lut.eval(enc(c).min(1.0)))));
    }

    // 7. Colour balance (shadows / midtones / highlights), luminosity kept.
    let bal = [a.balance_shadows, a.balance_midtones, a.balance_highlights];
    if bal.iter().flatten().any(|v| *v != 0.0) {
        let o = bal.map(|b| b.map(|v| (v / 100.0) as f32 * 0.12));
        px.p.par_iter_mut().for_each(|p| {
            let e = p.map(enc);
            let l = luma(e);
            let ws = 1.0 - smooth(0.0, 0.5, l);
            let wh = smooth(0.5, 1.0, l);
            let wm = (1.0 - ws - wh).max(0.0);
            let mut n = [0.0f32; 3];
            for c in 0..3 {
                n[c] = e[c] + ws * o[0][c] + wm * o[1][c] + wh * o[2][c];
            }
            let d = l - luma(n);
            *p = n.map(|v| dec((v + d).clamp(0.0, 1.0)));
        });
    }

    // 8. Vibrance and saturation in OkLCh (vibrance spares already-saturated colours and skin).
    if a.vibrance != 0.0 || a.saturation != 0.0 {
        let (vib, sat) = ((a.vibrance / 100.0) as f32, (a.saturation / 100.0) as f32);
        px.p.par_iter_mut().for_each(|p| {
            let lch = oklab::lab_to_lch(oklab::from_linear_srgb(*p));
            let c = lch[1];
            let skin = 1.0 - 0.6 * (-((lch[2] - 0.9) / 0.35).powi(2)).exp();
            let low = (1.0 - (c / 0.25).min(1.0)).powi(2);
            let k = (1.0 + sat) * (1.0 + vib * low * if vib > 0.0 { skin } else { 1.0 });
            let rgb = oklab::to_linear_srgb(oklab::lch_to_lab([lch[0], (c * k).max(0.0), lch[2]]));
            *p = gamut_map(rgb, LUMA).0;
        });
    }

    // 9. Look.
    match a.look {
        Look::None => {}
        Look::Grayscale => px.p.par_iter_mut().for_each(|p| *p = [luma(*p); 3]),
        Look::Sepia => px.p.par_iter_mut().for_each(|p| {
            // Warm toning: paper-white stays white, shadows go brown.
            let l = enc(luma(*p)).clamp(0.0, 1.0);
            *p = [l.powf(0.85), l.powf(1.0), l.powf(1.3)].map(dec);
        }),
        Look::Negative => px.p.par_iter_mut().for_each(|p| *p = p.map(|c| dec(1.0 - enc(c).min(1.0)))),
    }

    // 10. Soft blur.
    if a.blur > 0.0 {
        let s = (a.blur / 100.0) as f32 * long * 0.01;
        let planes: Vec<Plane> =
            (0..3).map(|c| gauss(&Plane::new(px.w, px.h, px.p.iter().map(|p| p[c]).collect()), s)).collect();
        px.p.par_iter_mut().enumerate().for_each(|(i, p)| *p = [planes[0].v[i], planes[1].v[i], planes[2].v[i]]);
    }

    // 11. Sharpening (craft's detail term, in log luminance).
    if a.sharpen_amount > 0.0 {
        let y = px.lum_plane();
        let l = log_lum(&y);
        let b = gauss(&l, a.sharpen_radius as f32);
        let (limit, mask, amt) =
            (sharpen_limit(a.sharpen_detail), (a.sharpen_masking / 100.0) as f32, (a.sharpen_amount / 150.0) as f32);
        let ny: Vec<f32> = (0..l.v.len())
            .into_par_iter()
            .map(|i| (l.v[i] + amt * sharpen_term(l.v[i] - b.v[i], limit, mask)).exp2())
            .collect();
        px.set_lum(&y, &ny);
    }

    // Keep every colour displayable: desaturate (not clip) what left the gamut.
    px.p.par_iter_mut().for_each(|p| *p = gamut_map(*p, LUMA).0);
}

fn dehaze(px: &mut Px, d: f32) {
    let dark = Plane::new(px.w, px.h, px.p.par_iter().map(|p| p[0].min(p[1]).min(p[2])).collect());
    let soft = gauss(&dark, px.long() * 0.01);
    // Haze colour: the average of the brightest 0.2 % of the soft dark channel.
    let mut idx: Vec<usize> = (0..soft.v.len()).collect();
    let n = (soft.v.len() / 500).max(1);
    idx.select_nth_unstable_by(n - 1, |x, y| soft.v[*y].total_cmp(&soft.v[*x]));
    let mut air = [0.0f32; 3];
    for &i in &idx[..n] {
        for c in 0..3 {
            air[c] += px.p[i][c] / n as f32;
        }
    }
    let amax = air[0].max(air[1]).max(air[2]).max(1e-3);
    px.p.par_iter_mut().zip(soft.v.par_iter()).for_each(|(p, &s)| {
        if d > 0.0 {
            let t = (1.0 - 0.95 * d * s / amax).max(0.2);
            *p = [0, 1, 2].map(|c| ((p[c] - air[c]) / t + air[c]).max(0.0));
        } else {
            let k = -d * 0.6;
            *p = [0, 1, 2].map(|c| p[c] + (air[c] - p[c]) * k * smooth(0.0, 1.0, s / amax));
        }
    });
}

/// Auto: settings that give a typical photo full tonal range, a pleasant
/// brightness and neutral greys. Returned as slider values, so the user can
/// see and change them.
pub fn auto(img: &Image) -> Result<Adjust, String> {
    let small = cs_image::preview_rgb8(img, 512)?;
    let px: Vec<[f32; 3]> = small.pixels().map(|p| p.0.map(|v| f32::from(v) / 255.0)).collect();
    if px.is_empty() {
        return Ok(Adjust::default());
    }
    let mut l: Vec<f32> = px.iter().map(|p| luma(*p)).collect();
    l.sort_by(f32::total_cmp);
    let pct = |q: f32| l[((l.len() - 1) as f32 * q) as usize];
    let (lo, hi, med) = (pct(0.003), pct(0.997), pct(0.5));
    let mut a = Adjust::default();
    if hi - lo > 0.2 {
        a.in_black = f64::from(lo.min(0.25));
        a.in_white = f64::from(hi.max(0.7));
    }
    // Midtones: the median at about 46 % after the stretch.
    let m = ((f64::from(med) - a.in_black) / (a.in_white - a.in_black)).clamp(0.02, 0.98);
    a.gamma = (m.ln() / 0.46f64.ln()).clamp(0.6, 1.8);
    // White balance from the surfaces that should be neutral (walls, clouds,
    // paper, grey clothes): the densest cluster of low-chroma midtone colours in
    // OkLab a/b. A colour cast moves that whole cluster, so it is still found on
    // a yellowed print, while a scene that is mostly one strong colour (blue
    // overalls, a green lawn) is not "corrected" towards grey.
    const BINS: usize = 25; // a and b from −0.125 to 0.125, 0.01 per bin
    let bin = |v: f32| ((v + 0.125) / 0.01).floor() as isize;
    let mids: Vec<([f32; 3], [f32; 2])> = px
        .iter()
        .filter(|p| (0.15..0.9).contains(&luma(**p)))
        .map(|p| {
            let lin = p.map(dec);
            let lab = oklab::from_linear_srgb(lin);
            (lin, [lab[1], lab[2]])
        })
        .collect();
    let mut hist = [[0u32; BINS]; BINS];
    for (_, ab) in &mids {
        let (i, j) = (bin(ab[0]), bin(ab[1]));
        if (0..BINS as isize).contains(&i) && (0..BINS as isize).contains(&j) {
            hist[i as usize][j as usize] += 1;
        }
    }
    let mut best = (0u32, 0usize, 0usize);
    for i in 0..BINS {
        for j in 0..BINS {
            let s: u32 = (i.saturating_sub(1)..(i + 2).min(BINS))
                .flat_map(|x| (j.saturating_sub(1)..(j + 2).min(BINS)).map(move |y| (x, y)))
                .map(|(x, y)| hist[x][y])
                .sum();
            if s > best.0 {
                best = (s, i, j);
            }
        }
    }
    let centre = [best.1 as f32 * 0.01 - 0.12, best.2 as f32 * 0.01 - 0.12];
    let mut sum = [0.0f64; 3];
    let mut n = 0.0;
    for (lin, ab) in &mids {
        if (ab[0] - centre[0]).hypot(ab[1] - centre[1]) < 0.025 {
            for c in 0..3 {
                sum[c] += f64::from(lin[c]);
            }
            n += 1.0;
        }
    }
    if n > (px.len() as f64 * 0.01).max(50.0) {
        let [r, g, b] = sum.map(|v| v / n);
        if r > 1e-4 && g > 1e-4 && b > 1e-4 {
            // Gains model of step 2: r ~ 2^(0.35 t), b ~ 2^(-0.35 t), g ~ 2^(-0.25 m), with t, m = slider / 100.
            // Neutral wants r·gr = b·gb  → t = log2(b / r) / 0.7,
            //            and g·gg = (r + b) / 2 → m = −log2(((r + b) / 2) / g) / 0.25. 80 % of the way.
            a.temp = (0.8 * 100.0 * (b / r).log2() / 0.7).clamp(-100.0, 100.0).round();
            a.tint = (-0.8 * 100.0 * (((r + b) / 2.0) / g).log2() / 0.25).clamp(-100.0, 100.0).round();
        }
    }
    a.vibrance = 10.0;
    a.gamma = (a.gamma * 100.0).round() / 100.0;
    a.in_black = (a.in_black * 1000.0).round() / 1000.0;
    a.in_white = (a.in_white * 1000.0).round() / 1000.0;
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Image {
        let mut d = Vec::new();
        for y in 0..h {
            for x in 0..w {
                d.extend(f(x, y));
            }
        }
        Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, d).unwrap()
    }

    fn mean(i: &Image) -> [f32; 3] {
        let v = i.to_normalized();
        let n = (v.len() / 3) as f32;
        let mut s = [0.0; 3];
        for c in v.as_chunks::<3>().0 {
            for k in 0..3 {
                s[k] += c[k] / n;
            }
        }
        s
    }

    fn photo() -> Image {
        img(64, 48, |x, y| [(x * 4) as u8, (y * 5) as u8, ((x + y) * 2) as u8])
    }

    #[test]
    fn neutral_is_exact_identity() {
        let p = photo();
        let out = render(&p, &Adjust::default()).unwrap();
        assert_eq!(out.data(), p.data());
        assert!(Adjust { points: vec![[0.0, 0.0], [1.0, 1.0]], ..Default::default() }.is_neutral());
    }

    #[test]
    fn every_slider_changes_the_picture_and_stays_in_range() {
        let p = photo();
        let base = mean(&p);
        let cases: Vec<(&str, Adjust)> = vec![
            ("exposure", Adjust { exposure: 1.0, ..Default::default() }),
            ("contrast", Adjust { contrast: 60.0, ..Default::default() }),
            ("highlights", Adjust { highlights: -80.0, ..Default::default() }),
            ("shadows", Adjust { shadows: 80.0, ..Default::default() }),
            ("whites", Adjust { whites: 50.0, ..Default::default() }),
            ("blacks", Adjust { blacks: -50.0, ..Default::default() }),
            ("temp", Adjust { temp: 50.0, ..Default::default() }),
            ("tint", Adjust { tint: 50.0, ..Default::default() }),
            ("vibrance", Adjust { vibrance: 80.0, ..Default::default() }),
            ("saturation", Adjust { saturation: -100.0, ..Default::default() }),
            ("clarity", Adjust { clarity: 80.0, ..Default::default() }),
            ("texture", Adjust { texture: 80.0, ..Default::default() }),
            ("dehaze", Adjust { dehaze: 60.0, ..Default::default() }),
            ("levels", Adjust { in_black: 0.1, gamma: 1.5, ..Default::default() }),
            ("curve", Adjust { curve_darks: 50.0, ..Default::default() }),
            ("points", Adjust { points: vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]], ..Default::default() }),
            ("balance", Adjust { balance_midtones: [40.0, 0.0, -40.0], ..Default::default() }),
            ("sharpen", Adjust { sharpen_amount: 120.0, ..Default::default() }),
            ("nr", Adjust { nr_luminance: 80.0, nr_color: 80.0, ..Default::default() }),
            ("blur", Adjust { blur: 50.0, ..Default::default() }),
            ("sepia", Adjust { look: Look::Sepia, ..Default::default() }),
            ("negative", Adjust { look: Look::Negative, ..Default::default() }),
        ];
        for (name, a) in cases {
            let out = render(&p, &a).unwrap();
            assert_eq!(out.dimensions(), p.dimensions(), "{name}");
            assert_ne!(out.data(), p.data(), "{name} changed nothing");
            assert!(out.to_normalized().iter().all(|v| (0.0..=1.0).contains(v)), "{name} out of range");
        }
        // Directions.
        assert!(mean(&render(&p, &Adjust { exposure: 1.0, ..Default::default() }).unwrap())[1] > base[1]);
        let warm = mean(&render(&p, &Adjust { temp: 60.0, ..Default::default() }).unwrap());
        assert!(warm[0] - warm[2] > base[0] - base[2], "warmer = redder than bluer");
        let grey = render(&p, &Adjust { look: Look::Grayscale, ..Default::default() }).unwrap().to_normalized();
        assert!(grey.as_chunks::<3>().0.iter().all(|c| (c[0] - c[1]).abs() < 0.01 && (c[1] - c[2]).abs() < 0.01));
    }

    #[test]
    fn sixteen_bit_and_alpha_are_kept() {
        let d: Vec<u16> = (0..16 * 8 * 4).map(|i| (i * 97 % 65535) as u16).collect();
        let p = Image::from_u16(16, 8, ChannelLayout::Rgba, &d).unwrap();
        let out = render(&p, &Adjust { exposure: 0.5, ..Default::default() }).unwrap();
        assert_eq!((out.sample_type(), out.layout()), (SampleType::U16, ChannelLayout::Rgba));
        let (a, b) = (p.to_normalized(), out.to_normalized());
        assert!(
            a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0).all(|(x, y)| (x[3] - y[3]).abs() < 1e-4),
            "alpha untouched"
        );
    }

    #[test]
    fn auto_stretches_a_flat_photo_and_neutralises_a_cast() {
        // Faded, blue-cast scan: values 90..170, blue high.
        let p = img(64, 64, |x, y| {
            let v = 90 + ((x + y) * 80 / 126) as u8;
            [v.saturating_sub(10), v, v.saturating_add(25)]
        });
        let a = auto(&p).unwrap();
        assert!(a.in_black > 0.2 && a.in_white < 0.8, "{a:?}");
        assert!(a.temp > 10.0, "a blue cast asks for warmer: {}", a.temp);
        let out = render(&p, &a).unwrap();
        let m = mean(&out);
        assert!(
            (m[2] - m[0]).abs() < (mean(&p)[2] - mean(&p)[0]).abs(),
            "cast reduced: {a:?} before {:?} after {m:?}",
            mean(&p)
        );
        let v = out.to_normalized();
        let (lo, hi) = v.iter().fold((1.0f32, 0.0f32), |(l, h), x| (l.min(*x), h.max(*x)));
        assert!(lo < 0.1 && hi > 0.85, "range stretched: {lo}..{hi}");
    }

    #[test]
    fn controls_follow_craft_ranges() {
        let c = controls();
        let sharpen = c.iter().find(|c| c.key == "sharpenAmount").unwrap();
        assert_eq!((sharpen.min, sharpen.max), (0.0, 150.0));
        let detail = c.iter().find(|c| c.key == "sharpenDetail").unwrap();
        assert_eq!(detail.default, 25.0);
        assert!(c.iter().all(|c| c.min < c.max && (c.min..=c.max).contains(&c.default)));
    }

    #[test]
    fn hostile_numbers_are_clamped() {
        let a = Adjust { exposure: f64::NAN, contrast: 1e9, gamma: 0.0, in_white: -3.0, ..Default::default() };
        let c = a.clamped();
        assert_eq!(c.exposure, 0.0);
        assert_eq!(c.contrast, 100.0);
        assert!(c.gamma >= 0.1 && c.in_white > c.in_black);
        render(&photo(), &a).unwrap();
    }
}
