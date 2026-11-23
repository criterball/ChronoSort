//! `cs-image`: open, edit and save photos for ChronoSort's viewer.
//!
//! Decoding and encoding use photocraft's image codecs (`craft-image-codecs`
//! from storytold/craft-libs, MIT OR Apache-2.0, pinned to one reviewed commit).
//! They keep EXIF, XMP and ICC on save and decode HEIC in pure Rust.
//!
//! Edits work on an upright copy in memory: rotate / flip (any bit depth, via
//! the codec's orientation code), crop, and resize (8- or 16-bit). Nothing here
//! writes to the original file: [`save`] writes a new file that must not exist yet.
//!
//! For JPEG, a rotate/flip-only edit can be done **losslessly** by changing the
//! EXIF orientation tag instead of re-encoding; [`orientation_after`] gives the
//! tag value for that (the app writes it with ExifTool).

use craft_image_codecs as cc;
pub use craft_image_codecs::{ChannelLayout, Format, Image, SampleType};
use std::path::Path;

/// Largest picture we agree to open for editing (pixels), to stay within memory.
pub const MAX_PIXELS: u64 = 200_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    RotateLeft,
    RotateRight,
    Rotate180,
    FlipHorizontal,
    FlipVertical,
    /// Rectangle in the current (upright, already edited) picture.
    Crop {
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
    /// New size in pixels (the caller keeps the aspect ratio if wanted).
    Resize {
        w: u32,
        h: u32,
    },
}

impl Edit {
    /// The EXIF orientation value that performs this turn, if it is one.
    fn as_orientation(self) -> Option<u16> {
        match self {
            Edit::FlipHorizontal => Some(2),
            Edit::Rotate180 => Some(3),
            Edit::FlipVertical => Some(4),
            Edit::RotateRight => Some(6),
            Edit::RotateLeft => Some(8),
            _ => None,
        }
    }
}

/// A photo opened for editing: upright pixels + its metadata.
pub struct Photo {
    pub image: Image,
    /// Format of the file it came from.
    pub format: Format,
    /// EXIF orientation the file had (1 = upright).
    pub orientation: u16,
}

pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Detect the format from the bytes (never trusts the extension).
pub fn detect(bytes: &[u8]) -> Option<Format> {
    cc::detect(bytes)
}

/// Which formats ChronoSort can save edits as.
pub fn save_formats() -> [(Format, &'static str, &'static str); 4] {
    [
        (Format::Jpeg, "JPEG", "jpg"),
        (Format::Png, "PNG", "png"),
        (Format::Tiff, "TIFF", "tif"),
        (Format::WebP, "WebP", "webp"),
    ]
}

/// Open a photo and turn it upright (its EXIF orientation applied and reset to 1).
pub fn open(path: &Path) -> Result<Photo, String> {
    let bytes = std::fs::read(path).map_err(err)?;
    open_bytes(&bytes)
}

pub fn open_bytes(bytes: &[u8]) -> Result<Photo, String> {
    let format = detect(bytes).ok_or("This file type cannot be edited (unknown picture format).")?;
    if !matches!(
        format,
        Format::Jpeg | Format::Png | Format::Tiff | Format::WebP | Format::Heif | Format::Bmp | Format::Gif
    ) {
        return Err(format!("{format:?} pictures cannot be edited in ChronoSort yet."));
    }
    if let Ok((w, h)) = cc::probe_size(bytes)
        && u64::from(w) * u64::from(h) > MAX_PIXELS
    {
        return Err(format!("This picture is too large to edit ({w} × {h})."));
    }
    let img = cc::decode(bytes).map_err(|e| format!("Could not read the picture: {e}"))?;
    let orientation = img.meta.exif.as_deref().map(cc::exif_orientation).unwrap_or(1);
    let image = img.oriented(orientation).map_err(err)?;
    Ok(Photo { image, format, orientation })
}

/// Apply one edit to the upright picture.
pub fn apply(img: Image, e: Edit) -> Result<Image, String> {
    if let Some(o) = e.as_orientation() {
        return img.oriented(o).map_err(err);
    }
    match e {
        Edit::Crop { x, y, w, h } => crop(&img, x, y, w, h),
        Edit::Resize { w, h } => resize(&img, w, h),
        _ => unreachable!(),
    }
}

fn crop(img: &Image, x: u32, y: u32, w: u32, h: u32) -> Result<Image, String> {
    let (iw, ih) = img.dimensions();
    if w == 0 || h == 0 || x.saturating_add(w) > iw || y.saturating_add(h) > ih {
        return Err(format!("The crop {w}×{h} at {x},{y} does not fit the picture ({iw}×{ih})."));
    }
    let bpp = img.layout().channels() * img.sample_type().bytes();
    let row = iw as usize * bpp;
    let mut out = Vec::with_capacity(w as usize * h as usize * bpp);
    for r in y..y + h {
        let start = r as usize * row + x as usize * bpp;
        out.extend_from_slice(&img.data()[start..start + w as usize * bpp]);
    }
    rebuild(img, w, h, img.layout(), img.sample_type(), out)
}

/// New image with the same metadata/ICC as `like`.
fn rebuild(
    like: &Image,
    w: u32,
    h: u32,
    layout: ChannelLayout,
    sample: SampleType,
    data: Vec<u8>,
) -> Result<Image, String> {
    Ok(Image::from_raw(w, h, layout, sample, data)
        .map_err(err)?
        .with_icc(like.icc.clone())
        .with_meta(like.meta.clone()))
}

fn resize(img: &Image, w: u32, h: u32) -> Result<Image, String> {
    use image::imageops::FilterType;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("That size is not possible.".into());
    }
    let (iw, ih) = img.dimensions();
    let alpha =
        matches!(img.layout(), ChannelLayout::GrayA | ChannelLayout::Rgba | ChannelLayout::CmykA | ChannelLayout::LabA);
    let wide = img.sample_type() != SampleType::U8;
    let layout = if alpha { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let sample = if wide { SampleType::U16 } else { SampleType::U8 };
    let src = img.converted(layout, sample);
    let filter = FilterType::Lanczos3;
    let data: Vec<u8> = match (alpha, wide) {
        (false, false) => {
            let b = image::RgbImage::from_raw(iw, ih, src.data().to_vec()).ok_or("bad buffer")?;
            image::imageops::resize(&b, w, h, filter).into_raw()
        }
        (true, false) => {
            let b = image::RgbaImage::from_raw(iw, ih, src.data().to_vec()).ok_or("bad buffer")?;
            image::imageops::resize(&b, w, h, filter).into_raw()
        }
        (a, true) => {
            // 16-bit samples are stored native-endian by the codec crate.
            let v: Vec<u16> = src.data().as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes(*c)).collect();
            let out: Vec<u16> = if a {
                let b = image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(iw, ih, v).ok_or("bad buffer")?;
                image::imageops::resize(&b, w, h, filter).into_raw()
            } else {
                let b = image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(iw, ih, v).ok_or("bad buffer")?;
                image::imageops::resize(&b, w, h, filter).into_raw()
            };
            out.iter().flat_map(|s| s.to_ne_bytes()).collect()
        }
    };
    rebuild(img, w, h, layout, sample, data)
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct SaveOptions {
    /// JPEG / WebP quality 1–100.
    pub quality: u8,
}
impl Default for SaveOptions {
    fn default() -> Self {
        Self { quality: 92 }
    }
}

/// What will be lost when saving `img` as `format` (e.g. "16-bit reduced to 8-bit").
pub fn warnings(img: &Image, format: Format) -> Vec<String> {
    cc::fidelity_warnings(img, format).into_iter().map(|w| w.to_string()).collect()
}

/// Encode and write to `dst`, which must not exist (never overwrites).
pub fn save(img: &Image, format: Format, opts: SaveOptions, dst: &Path) -> Result<(), String> {
    let q = opts.quality.clamp(1, 100);
    let o = cc::EncodeOptions { jpeg_quality: q, webp_quality: q, webp_lossless: false, ..Default::default() };
    let bytes = cc::encode(img, format, &o).map_err(|e| format!("Could not save as {format:?}: {e}"))?;
    use std::io::Write;
    let mut f = std::fs::File::options().write(true).create_new(true).open(dst).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            "A file with that name already exists.".to_string()
        } else {
            e.to_string()
        }
    })?;
    f.write_all(&bytes).map_err(err)?;
    f.sync_all().map_err(err)
}

/// Upright RGB, 8-bit, for showing an edit preview (longest side at most `max_side`).
pub fn preview_rgb8(img: &Image, max_side: u32) -> Result<image::RgbImage, String> {
    let (w, h) = img.dimensions();
    let rgb = img.converted(ChannelLayout::Rgb, SampleType::U8);
    let b = image::RgbImage::from_raw(w, h, rgb.data().to_vec()).ok_or("bad buffer")?;
    if w.max(h) <= max_side {
        return Ok(b);
    }
    let s = f64::from(max_side) / f64::from(w.max(h));
    let (nw, nh) = (((f64::from(w) * s).round() as u32).max(1), ((f64::from(h) * s).round() as u32).max(1));
    Ok(image::imageops::thumbnail(&b, nw, nh))
}

/// The EXIF orientation a JPEG should get so that it *displays* as if `edits`
/// (turns and flips only) had been applied to how it displays now with
/// orientation `current`. `None` if `edits` contain a crop or resize.
pub fn orientation_after(current: u16, edits: &[Edit]) -> Option<u16> {
    let turns: Vec<u16> = edits.iter().map(|e| e.as_orientation()).collect::<Option<_>>()?;
    // Simulate on a tiny 3×2 picture with unique pixel values.
    let raw = Image::from_raw(3, 2, ChannelLayout::Gray, SampleType::U8, vec![1, 2, 3, 4, 5, 6]).ok()?;
    let mut shown = raw.clone().oriented(current.clamp(1, 8)).ok()?;
    for t in turns {
        shown = shown.oriented(t).ok()?;
    }
    (1..=8u16).find(|&o| {
        raw.clone().oriented(o).ok().is_some_and(|x| x.dimensions() == shown.dimensions() && x.data() == shown.data())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(w: u32, h: u32) -> Image {
        let data: Vec<u8> = (0..w * h * 3).map(|i| (i % 251) as u8).collect();
        Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::U8, data).unwrap()
    }

    #[test]
    fn rotate_swaps_size_and_four_turns_are_identity() {
        let a = sample(40, 30);
        let r = apply(a.clone(), Edit::RotateRight).unwrap();
        assert_eq!(r.dimensions(), (30, 40));
        let mut x = a.clone();
        for _ in 0..4 {
            x = apply(x, Edit::RotateLeft).unwrap();
        }
        assert_eq!(x.data(), a.data());
        let f = apply(apply(a.clone(), Edit::FlipHorizontal).unwrap(), Edit::FlipHorizontal).unwrap();
        assert_eq!(f.data(), a.data());
    }

    #[test]
    fn crop_and_resize() {
        let a = sample(40, 30);
        let c = apply(a.clone(), Edit::Crop { x: 5, y: 4, w: 10, h: 8 }).unwrap();
        assert_eq!(c.dimensions(), (10, 8));
        // First pixel of the crop is pixel (5,4) of the source.
        let i = (4 * 40 + 5) * 3;
        assert_eq!(&c.data()[..3], &a.data()[i..i + 3]);
        assert!(apply(a.clone(), Edit::Crop { x: 35, y: 0, w: 10, h: 5 }).is_err());
        let s = apply(a, Edit::Resize { w: 20, h: 15 }).unwrap();
        assert_eq!(s.dimensions(), (20, 15));
    }

    #[test]
    fn save_roundtrip_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let a = sample(64, 48);
        for (fmt, _, ext) in save_formats() {
            let p = d.path().join(format!("x.{ext}"));
            save(&a, fmt, SaveOptions::default(), &p).unwrap();
            let back = open(&p).unwrap();
            assert_eq!(back.image.dimensions(), (64, 48), "{fmt:?}");
            assert!(save(&a, fmt, SaveOptions::default(), &p).is_err(), "must not overwrite");
        }
    }

    #[test]
    fn lossless_orientation_math() {
        // Upright JPEG turned right once -> tag 6; twice -> 3; right then left -> 1.
        assert_eq!(orientation_after(1, &[Edit::RotateRight]), Some(6));
        assert_eq!(orientation_after(1, &[Edit::RotateRight, Edit::RotateRight]), Some(3));
        assert_eq!(orientation_after(1, &[Edit::RotateRight, Edit::RotateLeft]), Some(1));
        // A photo already tagged 6 (shown turned right) turned left -> shown as stored -> 1.
        assert_eq!(orientation_after(6, &[Edit::RotateLeft]), Some(1));
        assert_eq!(orientation_after(1, &[Edit::Resize { w: 1, h: 1 }]), None);
        // Every combination lands on a valid tag.
        for o in 1..=8 {
            for e in [Edit::RotateLeft, Edit::RotateRight, Edit::Rotate180, Edit::FlipHorizontal, Edit::FlipVertical] {
                assert!(orientation_after(o, &[e]).is_some());
            }
        }
    }
}

#[cfg(test)]
mod preview_tests {
    #[test]
    fn preview_keeps_aspect() {
        let img =
            super::Image::from_raw(400, 100, super::ChannelLayout::Rgb, super::SampleType::U8, vec![7; 400 * 100 * 3])
                .unwrap();
        let p = super::preview_rgb8(&img, 200).unwrap();
        assert_eq!(p.dimensions(), (200, 50));
    }
}

#[cfg(test)]
mod heic_tests {
    /// Real HEIC samples (local test files; skipped when absent).
    #[test]
    fn opens_real_heic() {
        for f in ["example.heic", "image1.heic"] {
            let p = std::path::Path::new("../../../temp/heic_samples").join(f);
            if !p.exists() {
                eprintln!("skip: {} missing", p.display());
                continue;
            }
            let t = std::time::Instant::now();
            let ph = super::open(&p).unwrap_or_else(|e| panic!("{f}: {e}"));
            let (w, h) = ph.image.dimensions();
            eprintln!("{f}: {w}x{h} orientation {} in {:?}", ph.orientation, t.elapsed());
            assert!(w > 0 && h > 0);
            assert_eq!(ph.format, super::Format::Heif);
        }
    }
}
