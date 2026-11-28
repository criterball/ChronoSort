//! `cs-video`: one still picture (a "poster frame") from an MP4 / MOV video,
//! decoded in pure Rust with photocraft's craft-media (container) and
//! craft-video (H.264, HEVC incl. 10-bit, VP9, AV1). No FFmpeg needed.
//!
//! The frame is the key frame at or just before about one second in (a tenth
//! of the way into shorter clips), so usually only one packet is decoded. It
//! is turned upright using the track's display matrix (phone videos) and made
//! no bigger than `max_side`.
//!
//! Safety: this parses untrusted files. The app runs it in a short-lived child
//! process with a time limit (see cs-thumb), and here every size is capped:
//! picture size, packet size and how many packets are decoded.

#[cfg(windows)]
mod mf;
pub mod play;

use craft_media::ByteSource;
use craft_media::isobmff::{self, TrackKind};
use craft_video::TrackDecoder;
use craft_video::frame::{ColorFamily, Plane, Samples, VideoFrame};
use image::RgbImage;
use std::path::Path;

/// Biggest picture accepted (8K is 33 MP).
const MAX_PIXELS: u64 = 80_000_000;
/// Biggest single packet read.
const MAX_PACKET: u32 = 64 << 20;
/// Packets decoded at most while waiting for the first picture.
const MAX_PACKETS: usize = 60;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub(crate) fn err(what: &str, x: impl std::fmt::Display) -> Error {
    Error(format!("{what}: {x}"))
}

/// The poster frame of `path`, upright, longest side at most `max_side`.
pub fn poster(path: &Path, max_side: u32) -> Result<RgbImage, Error> {
    let file = std::fs::File::open(path).map_err(|x| err("cannot open", x))?;
    poster_from(&file, max_side)
}

/// The same for a video held in memory.
pub fn poster_bytes(bytes: &[u8], max_side: u32) -> Result<RgbImage, Error> {
    poster_from(bytes, max_side)
}

fn poster_from<S: ByteSource + ?Sized>(file: &S, max_side: u32) -> Result<RgbImage, Error> {
    let mp4 = isobmff::open(file).map_err(|x| err("not a readable MP4/MOV", x))?;
    let ti = mp4.track_of_kind(TrackKind::Video).ok_or_else(|| Error("no video in this file".into()))?;
    let track = &mp4.tracks[ti];
    if track.samples.is_empty() {
        return Err(Error("the video track is empty".into()));
    }
    if u64::from(track.width) * u64::from(track.height) > MAX_PIXELS {
        return Err(Error(format!("picture too big ({}×{})", track.width, track.height)));
    }
    // About 1 s in, or a tenth of a short clip; then the key frame at or before it.
    let ts = i64::from(track.timescale.max(1));
    let dur = i64::try_from(track.duration).unwrap_or(i64::MAX);
    let target = ts.min(dur / 10);
    let at = track.sample_at_presentation_time(target).unwrap_or(0);
    let start = track.sync_sample_before(at);

    let params = track.codec_params();
    let mut dec = TrackDecoder::with_threads(&params, 1).map_err(|x| err("this kind of video is not supported", x))?;
    let mut frame: Option<VideoFrame> = None;
    for (i, s) in track.samples.iter().enumerate().skip(start).take(MAX_PACKETS) {
        if s.size > MAX_PACKET {
            return Err(Error("a video packet is too big".into()));
        }
        let packet = mp4.read_sample(file, ti, i).map_err(|x| err("cannot read the video", x))?;
        // A damaged packet is skipped; the decoder carries on with the next one.
        let mut got = dec.decode(&packet, s.pts).unwrap_or_default();
        if got.is_empty() && i == start {
            // A key frame alone: ask for it now instead of waiting for reordering.
            got = dec.flush().unwrap_or_default();
        }
        if let Some(f) = got.into_iter().next() {
            frame = Some(f);
            break;
        }
    }
    if frame.is_none() {
        frame = dec.flush().ok().and_then(|v| v.into_iter().next());
    }
    let frame = frame.ok_or_else(|| Error("no picture could be decoded".into()))?;
    let img = to_rgb(&frame, max_side)?;
    // Display matrix in quarter turns clockwise (1 = iPhone portrait).
    Ok(match track.display_rotation() {
        Some(1) => image::imageops::rotate90(&img),
        Some(2) => image::imageops::rotate180(&img),
        Some(3) => image::imageops::rotate270(&img),
        _ => img,
    })
}

/// One sample of a plane on the 8-bit scale; out of range reads as 0. Deeper
/// video is the 8-bit value times 2^(depth-8) (BT.709 / BT.2020: 10-bit black
/// is 64, white 940), so it is a division, not a stretch of 0..1023 to 0..255.
fn sample(p: &Plane, x: u32, y: u32) -> f64 {
    let i = y as usize * p.stride() + x as usize;
    let depth = i32::from(p.bit_depth().clamp(1, 16));
    let v = match p.samples() {
        Samples::U8(v) => f64::from(v.get(i).copied().unwrap_or(0)),
        Samples::U16(v) => f64::from(v.get(i).copied().unwrap_or(0)),
    };
    if depth >= 8 { v / f64::from(1u32 << (depth - 8)) } else { v * 255.0 / f64::from((1u32 << depth) - 1) }
}

/// Kr, Kb of the colour matrix (ITU-T H.273 code points); unsignalled = BT.709
/// for HD and bigger, BT.601 below.
pub(crate) fn matrix(code: u8, height: u32) -> (f64, f64) {
    match code {
        1 => (0.2126, 0.0722),
        5 | 6 => (0.299, 0.114),
        9 | 10 => (0.2627, 0.0593),
        _ if height >= 720 => (0.2126, 0.0722),
        _ => (0.299, 0.114),
    }
}

/// Y'CbCr (0..=255 scale) to R'G'B'.
fn ycbcr_to_rgb(y: f64, cb: f64, cr: f64, full: bool, (kr, kb): (f64, f64)) -> [u8; 3] {
    let (y, cb, cr) = if full {
        (y, cb - 128.0, cr - 128.0)
    } else {
        ((y - 16.0) * 255.0 / 219.0, (cb - 128.0) * 255.0 / 224.0, (cr - 128.0) * 255.0 / 224.0)
    };
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    let g = (y - kr * r - kb * b) / (1.0 - kr - kb);
    [r, g, b].map(|v| v.round().clamp(0.0, 255.0) as u8)
}

/// Convert to RGB while shrinking (nearest sample per output pixel: fine for
/// a thumbnail and cheap even for 8K).
fn to_rgb(f: &VideoFrame, max_side: u32) -> Result<RgbImage, Error> {
    let (w, h) = (f.width(), f.height());
    let planes = f.planes();
    if w == 0 || h == 0 || planes.is_empty() {
        return Err(Error("empty picture".into()));
    }
    // Sample aspect ratio (anamorphic video): stretch the width.
    let (sn, sd) = f.sar.filter(|&(n, d)| n > 0 && d > 0).unwrap_or((1, 1));
    let disp_w = (u64::from(w) * u64::from(sn) / u64::from(sd)).max(1) as f64;
    let scale = (f64::from(max_side.max(1)) / disp_w.max(f64::from(h))).min(1.0);
    let ow = ((disp_w * scale).round() as u32).max(1);
    let oh = ((f64::from(h) * scale).round() as u32).max(1);
    let family = f.family();
    if family != ColorFamily::Gray && planes.len() < 3 {
        return Err(Error("unknown picture layout".into()));
    }
    let sub = f.subsampling();
    let m = matrix(f.color.matrix, h);
    let mut out = RgbImage::new(ow, oh);
    for oy in 0..oh {
        let y = (((f64::from(oy) + 0.5) * f64::from(h) / f64::from(oh)) as u32).min(h - 1);
        for ox in 0..ow {
            let x = (((f64::from(ox) + 0.5) * f64::from(w) / f64::from(ow)) as u32).min(w - 1);
            let px = match family {
                ColorFamily::Gray => [sample(&planes[0], x, y).round().clamp(0.0, 255.0) as u8; 3],
                ColorFamily::Rgb => [0, 1, 2].map(|i| sample(&planes[i], x, y).round().clamp(0.0, 255.0) as u8),
                ColorFamily::YCbCr => {
                    let cx = (x >> sub.x).min(planes[1].width().saturating_sub(1));
                    let cy = (y >> sub.y).min(planes[1].height().saturating_sub(1));
                    ycbcr_to_rgb(
                        sample(&planes[0], x, y),
                        sample(&planes[1], cx, cy),
                        sample(&planes[2], cx, cy),
                        f.color.full_range,
                        m,
                    )
                }
            };
            out.put_pixel(ox, oy, image::Rgb(px));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_conversion() {
        let bt709 = matrix(1, 1080);
        assert_eq!(ycbcr_to_rgb(16.0, 128.0, 128.0, false, bt709), [0, 0, 0], "limited-range black");
        assert_eq!(ycbcr_to_rgb(235.0, 128.0, 128.0, false, bt709), [255, 255, 255], "limited-range white");
        assert_eq!(ycbcr_to_rgb(255.0, 128.0, 128.0, true, bt709), [255, 255, 255], "full-range white");
        // BT.709 limited red (Y 63, Cb 102, Cr 240).
        let [r, g, b] = ycbcr_to_rgb(63.0, 102.0, 240.0, false, bt709);
        assert!(r > 250 && g < 5 && b < 5, "{r} {g} {b}");
        assert_eq!(matrix(2, 480), (0.299, 0.114), "unsignalled SD = BT.601");
    }

    #[test]
    fn not_a_video_is_a_plain_error() {
        let d = std::env::temp_dir().join(format!("cs-video-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("fake.mp4");
        std::fs::write(&f, b"\0\0\0\x18ftypisom\0\0\0\0isomrubbish rubbish rubbish").unwrap();
        assert!(poster(&f, 320).is_err());
        std::fs::write(&f, b"").unwrap();
        assert!(poster(&f, 320).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
