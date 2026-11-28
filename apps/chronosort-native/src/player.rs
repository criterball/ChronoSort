//! Playing a video in the viewer: pictures and sound from `cs_video::play` (Windows' own video engine, or our
//! decoders), the sound to the speakers through cpal, and the sound as the clock the pictures follow (like
//! FilmCraft's player). Without sound (a silent video, no sound device, test mode) a wall clock is used.
//! Seeking restarts both decoders at the new time.

use cs_video::play::{Frame, Sound, Stream, VideoInfo};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Seconds of sound kept ready for the speakers.
const AHEAD: f64 = 0.6;

pub(crate) struct AudioOut {
    _stream: cpal::Stream,
    ring: Arc<Mutex<VecDeque<f32>>>,
    /// Sample frames played since the last start / seek.
    played: Arc<AtomicU64>,
    volume: Arc<AtomicU32>,
    rate: u32,
    playing: bool,
}

impl AudioOut {
    fn open() -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let dev = host.default_output_device().ok_or("No sound device on this PC.")?;
        let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
        let channels = cfg.channels() as usize;
        let rate = cfg.sample_rate().0;
        let ring: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let played = Arc::new(AtomicU64::new(0));
        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let (r2, p2, v2) = (ring.clone(), played.clone(), volume.clone());
        let config: cpal::StreamConfig = cfg.clone().into();
        let stream = dev
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let vol = f32::from_bits(v2.load(Ordering::Relaxed));
                    let mut ring = r2.lock().unwrap_or_else(|e| e.into_inner());
                    let mut n = 0u64;
                    for frame in data.chunks_mut(channels) {
                        let (l, r) = if ring.len() >= 2 {
                            n += 1;
                            (ring.pop_front().unwrap_or(0.0), ring.pop_front().unwrap_or(0.0))
                        } else {
                            (0.0, 0.0)
                        };
                        for (c, s) in frame.iter_mut().enumerate() {
                            *s = match c {
                                0 => l * vol,
                                1 => r * vol,
                                _ => 0.0,
                            };
                        }
                        if channels == 1 {
                            frame[0] = (l + r) * 0.5 * vol;
                        }
                    }
                    p2.fetch_add(n, Ordering::Relaxed);
                },
                |e| eprintln!("sound: {e}"),
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { _stream: stream, ring, played, volume, rate, playing: true })
    }

    fn pause(&mut self, on: bool) {
        use cpal::traits::StreamTrait;
        self.playing = !on;
        if on {
            let _ = self._stream.pause();
        } else {
            let _ = self._stream.play();
        }
    }

    fn clear(&self) {
        self.ring.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.played.store(0, Ordering::Relaxed);
    }

    fn queued_secs(&self) -> f64 {
        self.ring.lock().unwrap_or_else(|e| e.into_inner()).len() as f64 / 2.0 / f64::from(self.rate)
    }

    /// Add sound, resampled (linear) from its own rate to the device's.
    fn push(&self, s: &Sound) {
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        if s.rate == self.rate {
            ring.extend(s.stereo.iter().copied());
            return;
        }
        let n = s.stereo.len() / 2;
        let out_n = (n as u64 * u64::from(self.rate) / u64::from(s.rate)) as usize;
        let step = f64::from(s.rate) / f64::from(self.rate);
        for i in 0..out_n {
            let x = i as f64 * step;
            let k = (x as usize).min(n.saturating_sub(1));
            let k2 = (k + 1).min(n.saturating_sub(1));
            let t = (x - k as f64) as f32;
            for c in 0..2 {
                let (a, b) = (s.stereo[k * 2 + c], s.stereo[k2 * 2 + c]);
                ring.push_back(a + (b - a) * t);
            }
        }
    }
}

pub struct Player {
    pub path: PathBuf,
    pub info: Option<VideoInfo>,
    max_side: u32,
    video: Stream<Frame>,
    sound: Option<Stream<Sound>>,
    out: Option<AudioOut>,
    /// No sound device (test mode, or none on this PC): play by the wall clock.
    silent: bool,
    pub note: Option<String>,
    pending: Option<Frame>,
    pub tex: Option<egui::TextureHandle>,
    /// Time the clock started from (the seek point, or the first sound's time).
    base: f64,
    sound_started: bool,
    started: Instant,
    /// Wall-clock mode: seconds already played before the last pause.
    paused_at: Option<f64>,
    pub paused: bool,
    pub ended: bool,
    pub volume: f32,
    pub muted: bool,
    /// Time of the picture on screen.
    pub shown: f64,
    /// Show the next picture even if not due yet (after opening or seeking, also while paused).
    need_frame: bool,
}

impl Player {
    pub fn open(path: &Path, max_side: u32, silent: bool) -> Self {
        let info = cs_video::play::info(path).ok();
        let mut note = None;
        let has_audio = info.as_ref().is_some_and(|i| i.has_audio);
        let out = if silent || !has_audio {
            None
        } else {
            match AudioOut::open() {
                Ok(o) => Some(o),
                Err(e) => {
                    note = Some(format!("Playing without sound: {e}"));
                    None
                }
            }
        };
        let silent = out.is_none();
        let mut p = Self {
            path: path.to_path_buf(),
            info,
            max_side,
            video: cs_video::play::pictures(path, 0.0, max_side),
            sound: None,
            out,
            silent,
            note,
            pending: None,
            tex: None,
            base: 0.0,
            sound_started: false,
            started: Instant::now(),
            paused_at: None,
            paused: false,
            ended: false,
            volume: 1.0,
            muted: false,
            shown: 0.0,
            need_frame: true,
        };
        p.seek(0.0);
        p
    }

    pub fn duration(&self) -> f64 {
        self.info.as_ref().map_or(0.0, |i| i.duration)
    }

    /// The playing time.
    pub fn clock(&self) -> f64 {
        if let Some(t) = self.paused_at {
            return t;
        }
        match &self.out {
            Some(o) if self.sound_started => self.base + o.played.load(Ordering::Relaxed) as f64 / f64::from(o.rate),
            Some(_) => self.base,
            None => self.base + self.started.elapsed().as_secs_f64(),
        }
    }

    pub fn seek(&mut self, t: f64) {
        let t = t.clamp(0.0, (self.duration() - 0.05).max(0.0));
        self.video = cs_video::play::pictures(&self.path, t, self.max_side);
        self.sound = if self.silent { None } else { cs_video::play::sound(&self.path, t) };
        if let Some(o) = &self.out {
            o.clear();
        }
        self.pending = None;
        self.base = t;
        self.sound_started = false;
        self.started = Instant::now();
        self.ended = false;
        if self.paused {
            self.paused_at = Some(t);
        }
        self.shown = t;
        self.need_frame = true;
    }

    pub fn toggle(&mut self) {
        if self.ended {
            self.paused = false;
            self.paused_at = None;
            self.seek(0.0);
            return;
        }
        self.set_paused(!self.paused);
    }

    pub fn set_paused(&mut self, on: bool) {
        if on == self.paused {
            return;
        }
        if on {
            self.paused_at = Some(self.clock());
        } else if let Some(t) = self.paused_at.take() {
            // Restart the wall clock from where it stopped (the sound clock just goes on).
            self.base = if self.out.is_some() && self.sound_started {
                t - self.out.as_ref().map_or(0.0, |o| o.played.load(Ordering::Relaxed) as f64 / f64::from(o.rate))
            } else {
                t
            };
            self.started = Instant::now();
        }
        self.paused = on;
        if let Some(o) = &mut self.out {
            o.pause(on);
        }
    }

    pub fn set_volume(&mut self, v: f32, muted: bool) {
        self.volume = v.clamp(0.0, 1.0);
        self.muted = muted;
        if let Some(o) = &self.out {
            let v = if muted { 0.0 } else { self.volume };
            o.volume.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    /// Every frame: keep the speakers fed, show the picture that is due. Returns true while playing.
    pub fn update(&mut self, ctx: &egui::Context) -> bool {
        // Sound: keep about AHEAD seconds queued.
        if let (Some(o), Some(s)) = (&self.out, &mut self.sound) {
            while o.queued_secs() < AHEAD {
                let Some(chunk) = s.try_next() else { break };
                if !self.sound_started {
                    self.sound_started = true;
                    self.base = chunk.time;
                    o.played.store(0, Ordering::Relaxed);
                }
                o.push(&chunk);
            }
            // A sound track that never delivers (damaged): fall back to the wall clock after a second.
            if !self.sound_started && self.started.elapsed().as_secs_f64() > 1.0 && !self.paused {
                self.out = None;
                self.silent = true;
                self.started = Instant::now();
            }
        }
        // Pictures: show the newest one that is due.
        let now = self.clock();
        let mut due: Option<Frame> = None;
        loop {
            if self.pending.is_none() {
                self.pending = self.video.try_next();
            }
            match &self.pending {
                Some(f) if (f.time <= now + 0.01 && !self.paused) || self.need_frame => {
                    due = self.pending.take();
                    if self.need_frame {
                        self.need_frame = false;
                        break;
                    }
                }
                _ => break,
            }
        }
        if let Some(f) = due {
            self.shown = f.time;
            let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
            match &mut self.tex {
                Some(t) if t.size() == [f.width as usize, f.height as usize] => {
                    t.set(img, egui::TextureOptions::LINEAR)
                }
                _ => self.tex = Some(ctx.load_texture("video-frame", img, egui::TextureOptions::LINEAR)),
            }
        }
        let sound_done =
            self.sound.as_ref().is_none_or(|s| s.ended()) && self.out.as_ref().is_none_or(|o| o.queued_secs() < 0.01);
        if self.video.ended() && self.pending.is_none() && sound_done && !self.ended && now >= self.shown {
            self.ended = true;
            self.paused_at = Some(self.shown.max(now.min(self.duration())));
            self.paused = true;
        }
        let playing = !self.paused && !self.ended;
        if playing {
            ctx.request_repaint();
        }
        playing
    }
}

/// "1:05" or "1:02:03".
pub fn clock_text(t: f64) -> String {
    let s = t.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Background music for the slideshow: the songs one after another (round and round), through the same
/// sound output as videos. Silent (but keeping its place) in test mode or without a sound device.
pub struct Music {
    out: Option<AudioOut>,
    tracks: Vec<PathBuf>,
    index: usize,
    stream: Option<Stream<Sound>>,
    pub on: bool,
    paused: bool,
    volume: f32,
    /// Softer while a video with its own sound plays (0.2), else 1.
    duck: f32,
    /// Fading out at the end: the current factor (1 → 0).
    fade: Option<f32>,
}

impl Music {
    pub fn new(tracks: Vec<PathBuf>, volume: f32, silent: bool) -> Self {
        let out = if silent || tracks.is_empty() { None } else { AudioOut::open().ok() };
        let mut m =
            Self { out, tracks, index: 0, stream: None, on: true, paused: false, volume, duck: 1.0, fade: None };
        m.start_track();
        m
    }

    /// The song playing now.
    pub fn track_name(&self) -> Option<String> {
        self.tracks.get(self.index).and_then(|p| p.file_stem()).map(|n| n.to_string_lossy().into_owned())
    }

    fn start_track(&mut self) {
        self.stream = self.tracks.get(self.index).and_then(|p| cs_video::play::sound(p, 0.0));
    }

    fn apply_volume(&self) {
        if let Some(o) = &self.out {
            let v = if self.on && !self.paused { self.volume * self.duck * self.fade.unwrap_or(1.0) } else { 0.0 };
            o.volume.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn set(&mut self, on: bool, paused: bool, duck: bool) {
        self.on = on;
        if paused != self.paused {
            self.paused = paused;
            if let Some(o) = &mut self.out {
                o.pause(paused);
            }
        }
        self.duck = if duck { 0.2 } else { 1.0 };
        self.apply_volume();
    }

    /// Fade out over about two seconds (the end of the show).
    pub fn fade_out(&mut self) {
        if self.fade.is_none() {
            self.fade = Some(1.0);
        }
    }

    pub fn restart(&mut self) {
        self.fade = None;
        self.index = 0;
        if let Some(o) = &self.out {
            o.clear();
        }
        self.start_track();
        self.apply_volume();
    }

    /// Every frame: keep the speakers fed; the next song when one ends.
    pub fn update(&mut self, dt: f32) {
        if let Some(f) = &mut self.fade {
            *f = (*f - dt / 2.0).max(0.0);
        }
        self.apply_volume();
        let Some(o) = &self.out else { return };
        if self.paused {
            return;
        }
        let mut next = false;
        if let Some(s) = &mut self.stream {
            while o.queued_secs() < AHEAD {
                match s.try_next() {
                    Some(chunk) => o.push(&chunk),
                    None => {
                        next = s.ended();
                        break;
                    }
                }
            }
        } else {
            next = true;
        }
        if next && !self.tracks.is_empty() && o.queued_secs() < 0.05 {
            self.index = (self.index + 1) % self.tracks.len();
            self.start_track();
        }
    }
}
