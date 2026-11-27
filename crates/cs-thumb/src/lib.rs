//! `cs-thumb`: small JPEG previews for the review grid, cached by content hash.
//!
//! Decoding untrusted images is a classic attack surface. We use the pure-Rust
//! decoders from the `image` crate with hard limits on dimensions and memory,
//! so a "decompression bomb" (tiny file, huge declared size) is refused.

use cs_scan::Format;

mod external;
pub use external::{ffmpeg_available, set_ffmpeg, video_helper_main};
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use std::io::{self, BufReader, Cursor};
use std::path::{Path, PathBuf};

/// Longest side of a thumbnail, in pixels.
pub const DEFAULT_SIZE: u32 = 320;
/// Large preview (e.g. to compare duplicates or look closely at a scan).
pub const PREVIEW_SIZE: u32 = 1280;
const MAX_DIM: u32 = 40_000;
const MAX_ALLOC: u64 = 768 << 20;

#[derive(Debug)]
pub enum ThumbError {
    /// Format we cannot preview yet (video, HEIC without EXIF thumbnail …).
    Unsupported(&'static str),
    Decode(String),
    Io(io::Error),
}

impl std::fmt::Display for ThumbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(w) => write!(f, "no preview available for {w}"),
            Self::Decode(e) => write!(f, "could not decode image: {e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for ThumbError {}
impl From<io::Error> for ThumbError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<image::ImageError> for ThumbError {
    fn from(e: image::ImageError) -> Self {
        Self::Decode(e.to_string())
    }
}

fn limits() -> Limits {
    let mut l = Limits::default();
    l.max_image_width = Some(MAX_DIM);
    l.max_image_height = Some(MAX_DIM);
    l.max_alloc = Some(MAX_ALLOC);
    l
}

/// Apply EXIF orientation (1 = normal … 8).
fn orient(img: DynamicImage, o: Option<u16>) -> DynamicImage {
    match o {
        Some(2) => img.fliph(),
        Some(3) => img.rotate180(),
        Some(4) => img.flipv(),
        Some(5) => img.rotate90().fliph(),
        Some(6) => img.rotate90(),
        Some(7) => img.rotate270().fliph(),
        Some(8) => img.rotate270(),
        _ => img,
    }
}

pub(crate) fn decode_limited<R: io::BufRead + io::Seek>(r: ImageReader<R>) -> Result<DynamicImage, ThumbError> {
    let mut r = r.with_guessed_format()?;
    r.limits(limits());
    let mut dec = r.into_decoder()?;
    let (w, h) = dec.dimensions();
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
        return Err(ThumbError::Decode(format!("refusing image of {w}x{h} pixels")));
    }
    dec.set_limits(limits())?;
    Ok(DynamicImage::from_decoder(dec)?)
}

fn encode_jpeg(img: &DynamicImage) -> Result<Vec<u8>, ThumbError> {
    let mut out = Vec::new();
    let rgb = img.to_rgb8();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82).encode_image(&rgb)?;
    Ok(out)
}

/// Build a JPEG thumbnail (longest side `size`) for an image file.
pub fn make(path: &Path, format: Format, size: u32) -> Result<Vec<u8>, ThumbError> {
    encode_jpeg(&load(path, format, size)?)
}

/// Decode an image safely (size limits, embedded preview for RAW/HEIC),
/// shrink it so the longest side is at most `max_side`, and turn it upright
/// (EXIF orientation). Used for thumbnails and for face detection.
pub fn load(path: &Path, format: Format, max_side: u32) -> Result<DynamicImage, ThumbError> {
    let (orientation, embedded) = cs_meta::orientation_and_thumbnail(path, format);
    let img = match format {
        Format::Jpeg | Format::Png | Format::Gif | Format::Bmp | Format::Webp => {
            decode_limited(ImageReader::new(BufReader::new(std::fs::File::open(path)?)))?
        }
        // RAW (TIFF-based) and CR3: the camera's big embedded preview, else the
        // small EXIF thumbnail.
        Format::Tiff | Format::Cr3
            if let Some(j) = cs_meta::raw_preview(path).filter(|j| j.len() > embedded.as_ref().map_or(0, Vec::len)) =>
        {
            decode_limited(ImageReader::new(Cursor::new(j)))?
        }
        // HEIC: photocraft's pure-Rust decoder (cs-image): full picture, already upright, no FFmpeg needed.
        Format::Heif if let Ok(img) = cs_image::open(path).and_then(|p| cs_image::preview_rgb8(&p.image, max_side)) => {
            return Ok(DynamicImage::ImageRgb8(img));
        }
        // HEIC it could not read / AVIF: FFmpeg when it is set up (full picture), else the embedded thumbnail.
        Format::Heif | Format::Avif if external::ffmpeg_available() => match external::preview(path, max_side, false) {
            Ok(img) => return Ok(img), // FFmpeg already turned it upright
            Err(e) => match embedded {
                Some(j) => decode_limited(ImageReader::new(Cursor::new(j)))?,
                None => return Err(e),
            },
        },
        // RAW without a big preview, or HEIC without FFmpeg: the embedded thumbnail if present.
        Format::Tiff | Format::Heif | Format::Avif | Format::Cr3 => match embedded {
            Some(j) => decode_limited(ImageReader::new(Cursor::new(j)))?,
            None => return Err(ThumbError::Unsupported("this RAW/HEIC file (no embedded preview)")),
        },
        f if f.kind() == cs_model::MediaKind::Video => return external::video(path, max_side),
        _ => return Err(ThumbError::Unsupported("this file type")),
    };
    let img = if img.width().max(img.height()) > max_side { img.thumbnail(max_side, max_side) } else { img };
    Ok(orient(img, orientation))
}

/// Mark a thumbnail as used (its modified time), at most once a day, so trimming
/// the cache removes the ones nobody looked at for the longest time.
fn touch_if_old(p: &Path, m: &std::fs::Metadata) {
    const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
    let now = std::time::SystemTime::now();
    if m.modified().ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > DAY)
        && let Ok(f) = std::fs::File::options().write(true).open(p)
    {
        let _ = f.set_modified(now);
    }
}

/// Thumbnails stored as `<dir>/<hash[..2]>/<hash>.jpg`; content-addressed, so
/// renaming or moving a photo never invalidates its thumbnail.
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The folder the thumbnails are kept in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path_for(&self, hash: &str) -> Option<PathBuf> {
        self.path_for_size(hash, DEFAULT_SIZE)
    }

    fn path_for_size(&self, hash: &str, size: u32) -> Option<PathBuf> {
        let ok = hash.len() >= 16 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        let name = if size == DEFAULT_SIZE { format!("{hash}.jpg") } else { format!("{hash}_{size}.jpg") };
        ok.then(|| self.dir.join(&hash[..2]).join(name))
    }

    /// Cached thumbnail, generating it on first use.
    pub fn get(&self, hash: &str, src: &Path, format: Format) -> Result<PathBuf, ThumbError> {
        self.get_sized(hash, src, format, DEFAULT_SIZE)
    }

    /// Like [`Cache::get`] with another longest side (e.g. [`PREVIEW_SIZE`]).
    pub fn get_sized(&self, hash: &str, src: &Path, format: Format, size: u32) -> Result<PathBuf, ThumbError> {
        let p = self.path_for_size(hash, size).ok_or(ThumbError::Unsupported("invalid hash"))?;
        if let Ok(m) = std::fs::metadata(&p) {
            touch_if_old(&p, &m);
            return Ok(p);
        }
        let jpg = make(src, format, size)?;
        std::fs::create_dir_all(p.parent().unwrap())?;
        // Write to a temp name then rename, so a half-written thumbnail is never served.
        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, &jpg)?;
        match std::fs::rename(&tmp, &p) {
            Ok(()) => Ok(p),
            Err(_) if p.exists() => {
                let _ = std::fs::remove_file(&tmp);
                Ok(p) // another thread won the race; same content
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_thumbnails() {
        let d = tempfile::tempdir().unwrap();
        cs_testgen::generate(d.path()).unwrap();
        let src = d.path().join(cs_testgen::SOURCE_DIR);
        let cache = Cache::new(d.path().join("thumbs"));
        let a = cs_core::analyze(&src, &cs_core::DatePolicy::new(0x7fff_ffff)).unwrap();
        let mut made = 0;
        for i in a.items.iter().filter(|i| i.kind == cs_model::MediaKind::Image) {
            let fmt = cs_scan::sniff(&std::fs::read(src.join(&i.rel_path)).unwrap()[..64.min(i.size as usize)], "");
            match cache.get(i.hash.as_deref().unwrap(), &src.join(&i.rel_path), fmt) {
                Ok(p) => {
                    let img = image::open(&p).unwrap();
                    assert!(img.width().max(img.height()) <= DEFAULT_SIZE);
                    made += 1;
                }
                // Truncated / hostile / TIFF-without-preview files may fail, but must not panic.
                Err(e) => eprintln!("{}: {e}", i.rel_path),
            }
        }
        assert!(made >= 20, "only {made} thumbnails");
    }

    #[test]
    fn decompression_bomb_is_refused() {
        // A valid PNG header declaring 100000 x 100000 pixels.
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend(100_000u32.to_be_bytes());
        ihdr.extend(100_000u32.to_be_bytes());
        ihdr.extend([8, 2, 0, 0, 0]);
        png.extend(13u32.to_be_bytes());
        png.extend(&ihdr);
        png.extend(crc32(&ihdr).to_be_bytes());
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("bomb.png");
        std::fs::write(&p, png).unwrap();
        assert!(matches!(make(&p, Format::Png, 320), Err(ThumbError::Decode(_))));
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
}
