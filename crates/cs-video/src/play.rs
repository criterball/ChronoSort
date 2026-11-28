//! Playing a video: the pictures and the sound of an MP4 / MOV decoded on worker threads, ahead of time, into
//! small queues the player takes from. Nothing here touches a screen or a speaker — the app shows the frames
//! and sends the sound to the audio device (the sound is the clock, like FilmCraft's player).
//!
//! Both streams start at a time (for seeking): the picture decoder starts at the key frame before it and
//! drops what comes earlier; the sound decoder starts at the packet that holds it and trims the rest.

use craft_media::isobmff::{self, TrackKind};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

use crate::{Error, err};
use craft_video::frame::{ColorFamily, Samples, VideoFrame};

/// What a video is, before playing it.
#[derive(Clone, Debug)]
pub struct VideoInfo {
    /// Upright picture size (phone videos turned).
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub has_audio: bool,
}

pub fn info(path: &Path) -> Result<VideoInfo, Error> {
    match info_rust(path) {
        Ok(i) => Ok(i),
        Err(e) => {
            #[cfg(windows)]
            if let Some(i) = crate::mf::info(path) {
                return Ok(i);
            }
            Err(e)
        }
    }
}

fn info_rust(path: &Path) -> Result<VideoInfo, Error> {
    let file = std::fs::File::open(path).map_err(|x| err("cannot open", x))?;
    let mp4 = isobmff::open(&file).map_err(|x| err("not a readable MP4/MOV", x))?;
    let ti = mp4.track_of_kind(TrackKind::Video).ok_or_else(|| Error("no video in this file".into()))?;
    let t = &mp4.tracks[ti];
    let turned = matches!(t.display_rotation(), Some(1 | 3));
    let (w, h) = if turned { (t.height, t.width) } else { (t.width, t.height) };
    let duration = t.duration as f64 / f64::from(t.timescale.max(1));
    Ok(VideoInfo { width: w, height: h, duration, has_audio: mp4.track_of_kind(TrackKind::Audio).is_some() })
}

/// One decoded picture, ready to show: RGBA, upright, at most the requested size.
pub struct Frame {
    /// Presentation time in seconds from the start.
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Sound, interleaved stereo `f32` at the track's own rate.
pub struct Sound {
    pub time: f64,
    pub rate: u32,
    pub stereo: Vec<f32>,
}

/// A stream of decoded items from a worker thread; dropping it stops the thread.
pub struct Stream<T> {
    rx: Receiver<T>,
    stop: Arc<AtomicBool>,
    ended: bool,
}

impl<T> Stream<T> {
    /// The next item if one is ready (never waits).
    pub fn try_next(&mut self) -> Option<T> {
        match self.rx.try_recv() {
            Ok(v) => Some(v),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.ended = true;
                None
            }
        }
    }

    /// The thread finished (the end of the file, or an error) and everything was taken.
    pub fn ended(&self) -> bool {
        self.ended
    }
}

impl<T> Drop for Stream<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn spawn<T: Send + 'static>(
    depth: usize,
    work: impl FnOnce(&SyncSender<T>, &AtomicBool) + Send + 'static,
) -> Stream<T> {
    let (tx, rx) = sync_channel(depth);
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    std::thread::Builder::new()
        .name("video-decode".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&tx, &s2)));
        })
        .expect("start a decoder thread");
    Stream { rx, stop, ended: false }
}

/// Use Windows' own video engine (graphics card) when it can play the file; `CHRONOSORT_NO_MF=1` turns it off.
fn use_mf() -> bool {
    cfg!(windows) && std::env::var_os("CHRONOSORT_NO_MF").is_none()
}

/// The pictures from `from` seconds on, at most `max_side` pixels on the longest side.
pub fn pictures(path: &Path, from: f64, max_side: u32) -> Stream<Frame> {
    #[cfg(windows)]
    if use_mf() && crate::mf::can_play(path) {
        let p = path.to_path_buf();
        return spawn(4, move |tx, stop| crate::mf::pictures(&p, from, max_side, tx, stop));
    }
    pictures_rust(path, from, max_side)
}

fn pictures_rust(path: &Path, from: f64, max_side: u32) -> Stream<Frame> {
    let path: PathBuf = path.to_path_buf();
    spawn(4, move |tx, stop| {
        let Ok(file) = std::fs::File::open(&path) else { return };
        let Ok(mp4) = isobmff::open(&file) else { return };
        let Some(ti) = mp4.track_of_kind(TrackKind::Video) else { return };
        let track = &mp4.tracks[ti];
        let ts = f64::from(track.timescale.max(1));
        let rotation = track.display_rotation();
        let at = track.sample_at_presentation_time((from * ts) as i64).unwrap_or(0);
        let start = track.sync_sample_before(at);
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 8);
        let Ok(mut dec) = craft_video::TrackDecoder::with_threads(&track.codec_params(), threads) else { return };
        let secs = |pts: Option<i64>| pts.map_or(0.0, |p| ((p - track.edit_offset) as f64 / ts).max(0.0));
        let send = |f: VideoFrame| -> bool {
            let t = secs(f.pts);
            if t + 0.001 < from {
                return true; // before the seek point: decoded, not shown
            }
            match to_rgba(&f, max_side, rotation) {
                Some((w, h, rgba)) => tx.send(Frame { time: t, width: w, height: h, rgba }).is_ok(),
                None => true,
            }
        };
        for (i, s) in track.samples.iter().enumerate().skip(start) {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let Ok(packet) = mp4.read_sample(&file, ti, i) else { continue };
            for f in dec.decode(&packet, s.pts).unwrap_or_default() {
                if !send(f) {
                    return;
                }
            }
        }
        for f in dec.flush().unwrap_or_default() {
            if !send(f) {
                return;
            }
        }
    })
}

/// The sound from `from` seconds on (none when the file has no sound or its codec is not supported).
pub fn sound(path: &Path, from: f64) -> Option<Stream<Sound>> {
    #[cfg(windows)]
    if use_mf() && crate::mf::has_sound(path) {
        let p = path.to_path_buf();
        return Some(spawn(16, move |tx, stop| crate::mf::sound(&p, from, tx, stop)));
    }
    sound_rust(path, from)
}

fn sound_rust(path: &Path, from: f64) -> Option<Stream<Sound>> {
    let path: PathBuf = path.to_path_buf();
    // Check first that there is a sound track we can decode, so the caller knows whether to use a clock.
    {
        let file = std::fs::File::open(&path).ok()?;
        let mp4 = isobmff::open(&file).ok()?;
        let ai = mp4.track_of_kind(TrackKind::Audio)?;
        let t = &mp4.tracks[ai];
        let first: Vec<Vec<u8>> = (0..t.samples.len().min(craft_audio::LOOKAHEAD_PACKETS))
            .filter_map(|i| mp4.read_sample(&file, ai, i).ok())
            .collect();
        let refs: Vec<&[u8]> = first.iter().map(Vec::as_slice).collect();
        craft_audio::TrackDecoder::new(&t.codec_params(), &refs).ok()?;
    }
    Some(spawn(16, move |tx, stop| {
        let Ok(file) = std::fs::File::open(&path) else { return };
        let Ok(mp4) = isobmff::open(&file) else { return };
        let Some(ai) = mp4.track_of_kind(TrackKind::Audio) else { return };
        let track = &mp4.tracks[ai];
        let ts = f64::from(track.timescale.max(1));
        let first: Vec<Vec<u8>> = (0..track.samples.len().min(craft_audio::LOOKAHEAD_PACKETS))
            .filter_map(|i| mp4.read_sample(&file, ai, i).ok())
            .collect();
        let refs: Vec<&[u8]> = first.iter().map(Vec::as_slice).collect();
        let Ok(mut dec) = craft_audio::TrackDecoder::new(&track.codec_params(), &refs) else { return };
        // Start a little early (decoders need a packet or two to settle), then trim to `from`.
        let at = track.sample_at_presentation_time((from * ts) as i64).unwrap_or(0);
        let start = at.saturating_sub(dec.preroll().max(2));
        for i in start..track.samples.len() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let Ok(packet) = mp4.read_sample(&file, ai, i) else { continue };
            let Ok(buf) = dec.decode(&packet) else { continue };
            let rate = buf.sample_rate.max(1);
            let n = buf.channels.first().map_or(0, Vec::len);
            if n == 0 {
                continue;
            }
            let t = ((track.samples[i].pts - track.edit_offset) as f64 / ts).max(0.0);
            // Skip what lies before the seek point.
            let skip = if t < from { (((from - t) * f64::from(rate)) as usize).min(n) } else { 0 };
            if skip >= n {
                continue;
            }
            let (l, r) = match buf.channels.len() {
                1 => (&buf.channels[0], &buf.channels[0]),
                _ => (&buf.channels[0], &buf.channels[1]),
            };
            let mut stereo = Vec::with_capacity((n - skip) * 2);
            for k in skip..n {
                stereo.push(l[k]);
                stereo.push(r[k]);
            }
            let time = t + skip as f64 / f64::from(rate);
            if tx.send(Sound { time, rate, stereo }).is_err() {
                return;
            }
        }
    }))
}

/// A decoded frame as upright RGBA no bigger than `max_side` (nearest sample per output pixel), in parallel.
fn to_rgba(f: &VideoFrame, max_side: u32, rotation: Option<u8>) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = (f.width(), f.height());
    let planes = f.planes();
    if w == 0 || h == 0 || planes.is_empty() {
        return None;
    }
    let (sn, sd) = f.sar.filter(|&(n, d)| n > 0 && d > 0).unwrap_or((1, 1));
    let disp_w = (u64::from(w) * u64::from(sn) / u64::from(sd)).max(1) as f64;
    let scale = (f64::from(max_side.max(1)) / disp_w.max(f64::from(h))).min(1.0);
    let ow = ((disp_w * scale).round() as u32).max(1);
    let oh = ((f64::from(h) * scale).round() as u32).max(1);
    let family = f.family();
    if family != ColorFamily::Gray && planes.len() < 3 {
        return None;
    }
    let sub = f.subsampling();
    let (kr, kb) = crate::matrix(f.color.matrix, h);
    let full = f.color.full_range;
    // Fixed-point coefficients (×1024) for speed.
    let cr_r = (2.0 * (1.0 - kr) * 1024.0) as i32;
    let cb_b = (2.0 * (1.0 - kb) * 1024.0) as i32;
    let kg = 1.0 - kr - kb;
    let cr_g = (2.0 * (1.0 - kr) * kr / kg * 1024.0) as i32;
    let cb_g = (2.0 * (1.0 - kb) * kb / kg * 1024.0) as i32;
    let value = |p: &craft_video::frame::Plane, x: u32, y: u32| -> i32 {
        let i = y as usize * p.stride() + x as usize;
        match p.samples() {
            Samples::U8(v) => i32::from(v.get(i).copied().unwrap_or(0)),
            Samples::U16(v) => i32::from(v.get(i).copied().unwrap_or(0)) >> (p.bit_depth().clamp(8, 16) - 8),
        }
    };
    let xs: Vec<u32> =
        (0..ow).map(|ox| (((f64::from(ox) + 0.5) * f64::from(w) / f64::from(ow)) as u32).min(w - 1)).collect();
    let mut out = vec![255u8; (ow * oh * 4) as usize];
    out.par_chunks_mut((ow * 4) as usize).enumerate().for_each(|(oy, row)| {
        let y = (((oy as f64 + 0.5) * f64::from(h) / f64::from(oh)) as u32).min(h - 1);
        for (ox, &x) in xs.iter().enumerate() {
            let px = match family {
                ColorFamily::Gray => {
                    let v = value(&planes[0], x, y).clamp(0, 255) as u8;
                    [v, v, v]
                }
                ColorFamily::Rgb => [0, 1, 2].map(|i| value(&planes[i], x, y).clamp(0, 255) as u8),
                ColorFamily::YCbCr => {
                    let cx = (x >> sub.x).min(planes[1].width().saturating_sub(1));
                    let cy = (y >> sub.y).min(planes[1].height().saturating_sub(1));
                    let (mut yy, mut cb, mut cr) =
                        (value(&planes[0], x, y), value(&planes[1], cx, cy) - 128, value(&planes[2], cx, cy) - 128);
                    if !full {
                        yy = (yy - 16) * 1192 / 1024; // 255/219
                        cb = cb * 1166 / 1024; // 255/224
                        cr = cr * 1166 / 1024;
                    }
                    let r = yy + ((cr_r * cr) >> 10);
                    let g = yy - ((cb_g * cb + cr_g * cr) >> 10);
                    let b = yy + ((cb_b * cb) >> 10);
                    [r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8]
                }
            };
            row[ox * 4..ox * 4 + 3].copy_from_slice(&px);
        }
    });
    let img = image::RgbaImage::from_raw(ow, oh, out)?;
    let img = match rotation {
        Some(1) => image::imageops::rotate90(&img),
        Some(2) => image::imageops::rotate180(&img),
        Some(3) => image::imageops::rotate270(&img),
        _ => img,
    };
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}
