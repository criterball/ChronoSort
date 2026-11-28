//! Windows Media Foundation playback: the system's own video engine (part of Windows, nothing installed).
//! It decodes on the graphics card where it can (fast even for 4K), scales and converts the pictures to RGB
//! itself, decodes the sound to float samples, and reads more kinds of files than our own decoders
//! (camcorder MTS / M2TS, AVI, WMV, MPEG …). `play.rs` uses it first and falls back to the pure-Rust
//! decoders when Windows cannot open a file. Each reader lives on one worker thread (COM, multithreaded).

use super::play::{Frame, Sound, VideoInfo};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::{GUID, HSTRING};

const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

/// COM + Media Foundation for the current thread, undone on drop.
struct Session;

impl Session {
    fn start() -> Option<Self> {
        // SAFETY: plain initialisation calls; undone in Drop on the same thread.
        unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return None;
            }
            if MFStartup(MF_VERSION, MFSTARTUP_FULL).is_err() {
                CoUninitialize();
                return None;
            }
        }
        Some(Session)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: matched with the successful calls in `start`.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

/// A source reader for one kind of stream (the others are switched off).
fn reader(path: &Path, stream: u32) -> Option<IMFSourceReader> {
    // SAFETY: COM calls on this thread's initialised apartment; every out-pointer is a live local.
    unsafe {
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 2).ok()?;
        let attrs = attrs?;
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1).ok()?;
        attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1).ok()?;
        let r = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attrs).ok()?;
        r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).ok()?;
        r.SetStreamSelection(stream, true).ok()?;
        Some(r)
    }
}

fn duration(r: &IMFSourceReader) -> f64 {
    // SAFETY: valid reader; the PROPVARIANT is owned and dropped here.
    unsafe {
        r.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            .ok()
            .and_then(|v| u64::try_from(&v).ok())
            .map_or(0.0, |t| t as f64 / 1e7)
    }
}

fn seek(r: &IMFSourceReader, secs: f64) {
    if secs > 0.0 {
        let pos = PROPVARIANT::from((secs * 1e7) as i64);
        // SAFETY: valid reader and PROPVARIANT.
        let _ = unsafe { r.SetCurrentPosition(&GUID::zeroed(), &pos) };
    }
}

/// Picture size and rotation (clockwise quarter turns) of the first video stream.
fn native_video(r: &IMFSourceReader) -> Option<(u32, u32, u8)> {
    // SAFETY: valid reader; attribute reads only.
    unsafe {
        let t = r.GetNativeMediaType(VIDEO, 0).ok()?;
        let size = t.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let rot = t.GetUINT32(&MF_MT_VIDEO_ROTATION).unwrap_or(0);
        Some(((size >> 32) as u32, size as u32, ((rot / 90) % 4) as u8))
    }
}

pub fn info(path: &Path) -> Option<VideoInfo> {
    let _s = Session::start()?;
    let r = reader(path, VIDEO)?;
    let (w, h, rot) = native_video(&r)?;
    let (w, h) = if rot % 2 == 1 { (h, w) } else { (w, h) };
    let has_audio = reader(path, AUDIO).is_some_and(|a| {
        // SAFETY: valid reader.
        unsafe { a.GetNativeMediaType(AUDIO, 0).is_ok() }
    });
    Some(VideoInfo { width: w, height: h, duration: duration(&r), has_audio })
}

/// Can Windows play the pictures of this file? (Checked before a worker is started.)
pub fn can_play(path: &Path) -> bool {
    let Some(_s) = Session::start() else { return false };
    reader(path, VIDEO).is_some_and(|r| native_video(&r).is_some())
}

/// The picture worker: RGBA frames, upright, at most `max_side`, from `from` seconds.
pub fn pictures(path: &Path, from: f64, max_side: u32, tx: &SyncSender<Frame>, stop: &AtomicBool) {
    let Some(_s) = Session::start() else { return };
    let Some(r) = reader(path, VIDEO) else { return };
    let Some((nw, nh, rot)) = native_video(&r) else { return };
    // Ask the video processor for RGB32 at the size we show (even numbers, aspect kept).
    let scale = (f64::from(max_side) / f64::from(nw.max(nh))).min(1.0);
    let ow = ((f64::from(nw) * scale) as u32 / 2 * 2).max(2);
    let oh = ((f64::from(nh) * scale) as u32 / 2 * 2).max(2);
    // SAFETY: COM calls with live locals; the buffer is locked only while its bytes are copied.
    unsafe {
        let Ok(t) = MFCreateMediaType() else { return };
        if t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).is_err()
            || t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).is_err()
            || t.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(ow) << 32) | u64::from(oh)).is_err()
            || r.SetCurrentMediaType(VIDEO, None, &t).is_err()
        {
            return;
        }
        let cur = r.GetCurrentMediaType(VIDEO).ok();
        let size = cur
            .as_ref()
            .and_then(|c| c.GetUINT64(&MF_MT_FRAME_SIZE).ok())
            .unwrap_or((u64::from(ow) << 32) | u64::from(oh));
        let (fw, fh) = ((size >> 32) as u32, size as u32);
        let stride =
            cur.as_ref().and_then(|c| c.GetUINT32(&MF_MT_DEFAULT_STRIDE).ok()).map_or(fw as i32 * 4, |s| s as i32);
        seek(&r, from);
        while !stop.load(Ordering::Relaxed) {
            let (mut flags, mut ts) = (0u32, 0i64);
            let mut sample: Option<IMFSample> = None;
            if r.ReadSample(VIDEO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample)).is_err() {
                return;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return;
            }
            let Some(sample) = sample else { continue };
            let time = ts as f64 / 1e7;
            if time + 0.001 < from {
                continue;
            }
            let Ok(buf) = sample.ConvertToContiguousBuffer() else { continue };
            let (mut p, mut len) = (std::ptr::null_mut::<u8>(), 0u32);
            if buf.Lock(&mut p, None, Some(&mut len)).is_err() || p.is_null() {
                continue;
            }
            let bytes = std::slice::from_raw_parts(p, len as usize);
            let mut rgba = vec![255u8; (fw * fh * 4) as usize];
            let row = (fw * 4) as usize;
            for y in 0..fh as usize {
                // A negative stride means bottom-up rows.
                let src_row = if stride < 0 { fh as usize - 1 - y } else { y };
                let s = src_row * stride.unsigned_abs() as usize;
                if s + row > bytes.len() {
                    break;
                }
                for x in 0..fw as usize {
                    let (si, di) = (s + x * 4, y * row + x * 4);
                    rgba[di] = bytes[si + 2];
                    rgba[di + 1] = bytes[si + 1];
                    rgba[di + 2] = bytes[si];
                }
            }
            let _ = buf.Unlock();
            let (w, h, rgba) = turn(fw, fh, rgba, rot);
            if tx.send(Frame { time, width: w, height: h, rgba }).is_err() {
                return;
            }
        }
    }
}

/// Is there sound Windows can decode?
pub fn has_sound(path: &Path) -> bool {
    let Some(_s) = Session::start() else { return false };
    reader(path, AUDIO).is_some_and(|r| {
        // SAFETY: valid reader.
        unsafe { r.GetNativeMediaType(AUDIO, 0).is_ok() }
    })
}

/// The sound worker: stereo float samples from `from` seconds.
pub fn sound(path: &Path, from: f64, tx: &SyncSender<Sound>, stop: &AtomicBool) {
    let Some(_s) = Session::start() else { return };
    let Some(r) = reader(path, AUDIO) else { return };
    // SAFETY: COM calls with live locals; the buffer is locked only while its samples are copied.
    unsafe {
        let Ok(t) = MFCreateMediaType() else { return };
        if t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).is_err()
            || t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float).is_err()
            || r.SetCurrentMediaType(AUDIO, None, &t).is_err()
        {
            return;
        }
        let Ok(cur) = r.GetCurrentMediaType(AUDIO) else { return };
        let channels = cur.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(2).max(1) as usize;
        let rate = cur.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(48_000).max(1);
        seek(&r, from);
        while !stop.load(Ordering::Relaxed) {
            let (mut flags, mut ts) = (0u32, 0i64);
            let mut sample: Option<IMFSample> = None;
            if r.ReadSample(AUDIO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample)).is_err() {
                return;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return;
            }
            let Some(sample) = sample else { continue };
            let Ok(buf) = sample.ConvertToContiguousBuffer() else { continue };
            let (mut p, mut len) = (std::ptr::null_mut::<u8>(), 0u32);
            if buf.Lock(&mut p, None, Some(&mut len)).is_err() || p.is_null() {
                continue;
            }
            let n = len as usize / 4 / channels;
            let f = std::slice::from_raw_parts(p.cast::<f32>(), n * channels);
            let mut time = ts as f64 / 1e7;
            let mut skip = 0;
            if time < from {
                skip = (((from - time) * f64::from(rate)) as usize).min(n);
                time = from;
            }
            let mut stereo = Vec::with_capacity((n - skip) * 2);
            for k in skip..n {
                let l = f[k * channels];
                let rr = if channels > 1 { f[k * channels + 1] } else { l };
                stereo.push(l);
                stereo.push(rr);
            }
            let _ = buf.Unlock();
            if !stereo.is_empty() && tx.send(Sound { time, rate, stereo }).is_err() {
                return;
            }
        }
    }
}

/// Turn RGBA pixels clockwise by quarter turns.
fn turn(w: u32, h: u32, rgba: Vec<u8>, quarters: u8) -> (u32, u32, Vec<u8>) {
    if quarters.is_multiple_of(4) {
        return (w, h, rgba);
    }
    let Some(img) = image::RgbaImage::from_raw(w, h, rgba) else { return (w, h, Vec::new()) };
    let img = match quarters % 4 {
        1 => image::imageops::rotate90(&img),
        2 => image::imageops::rotate180(&img),
        _ => image::imageops::rotate270(&img),
    };
    let (w, h) = img.dimensions();
    (w, h, img.into_raw())
}
