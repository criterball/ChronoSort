//! Background image loading: thumbnails (through the shared cache) and full pictures for the viewer.
//!
//! The newest request is served first (a stack), so whatever is on screen right now loads first
//! while the user scrolls. Requests of a folder that is no longer shown are dropped (`generation`).
//! Decoding happens on worker threads; the UI thread only uploads finished pixels.

use crate::browse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

/// What to load.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub path: PathBuf,
    /// Longest side in pixels.
    pub size: u32,
    /// `true` = decode the original file (viewer), `false` = cached thumbnail.
    pub full: bool,
}

pub struct Done {
    pub key: Key,
    pub result: Result<Picture, String>,
}

pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// Size of the original picture when known (viewer info).
    pub original: Option<(u32, u32)>,
    /// Histogram (viewer previews only, Ctrl+H).
    pub hist: Option<Box<crate::histo::Histo>>,
}

struct Job {
    key: Key,
    generation: u64,
}

struct Queue {
    jobs: Vec<Job>,
    generation: u64,
}

pub struct Loader {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    done_rx: Receiver<Done>,
    pending: HashMap<Key, ()>,
}

impl Loader {
    pub fn new(cache_dir: PathBuf, ctx: egui::Context) -> Self {
        let queue = Arc::new((Mutex::new(Queue { jobs: Vec::new(), generation: 0 }), Condvar::new()));
        let (tx, rx) = channel();
        let cache = Arc::new(cs_thumb::Cache::new(cache_dir));
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, 6);
        for i in 0..threads {
            let (q, tx, cache, ctx) = (queue.clone(), tx.clone(), cache.clone(), ctx.clone());
            std::thread::Builder::new()
                .name(format!("loader-{i}"))
                .spawn(move || worker(&q, &tx, &cache, &ctx))
                .expect("start a loader thread");
        }
        Self { queue, done_rx: rx, pending: HashMap::new() }
    }

    /// Ask for a picture (once; repeated asks while it loads are ignored).
    pub fn request(&mut self, key: Key) {
        if self.pending.contains_key(&key) {
            return;
        }
        self.pending.insert(key.clone(), ());
        let (m, cv) = &*self.queue;
        let mut q = m.lock().unwrap_or_else(|e| e.into_inner());
        let generation = q.generation;
        q.jobs.push(Job { key, generation });
        cv.notify_one();
    }

    /// Forget everything not yet started (a new folder is shown).
    pub fn new_generation(&mut self) {
        let (m, _) = &*self.queue;
        let mut q = m.lock().unwrap_or_else(|e| e.into_inner());
        q.generation += 1;
        q.jobs.clear();
        self.pending.clear();
    }

    pub fn finished(&mut self) -> Vec<Done> {
        let out: Vec<Done> = self.done_rx.try_iter().collect();
        for d in &out {
            self.pending.remove(&d.key);
        }
        out
    }
}

fn worker(q: &(Mutex<Queue>, Condvar), tx: &Sender<Done>, cache: &cs_thumb::Cache, ctx: &egui::Context) {
    let (m, cv) = q;
    loop {
        let job = {
            let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                // Newest first; jobs of an old folder are dropped.
                match g.jobs.pop() {
                    Some(j) if j.generation == g.generation => break j,
                    Some(_) => continue,
                    None => g = cv.wait(g).unwrap_or_else(|e| e.into_inner()),
                }
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(&job.key, cache)))
            .unwrap_or_else(|_| Err("This file could not be read (it may be damaged).".into()));
        if tx.send(Done { key: job.key, result }).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn load(key: &Key, cache: &cs_thumb::Cache) -> Result<Picture, String> {
    let fmt = browse::real_format(&key.path);
    let kind = fmt.kind();
    if kind != cs_model::MediaKind::Image && kind != cs_model::MediaKind::Video {
        return Err("No preview for this kind of file.".into());
    }
    if key.full {
        let original = image_size(&key.path);
        let img = cs_thumb::load(&key.path, fmt, key.size).map_err(|e| e.to_string())?;
        return Ok(to_picture(img, original));
    }
    let meta = std::fs::metadata(&key.path).map_err(|e| e.to_string())?;
    let file =
        cache.get_sized(&browse::cache_key(&key.path, &meta), &key.path, fmt, key.size).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
    let mut pic = to_picture(img, None);
    if key.size == cs_thumb::PREVIEW_SIZE {
        pic.hist = Some(Box::new(crate::histo::Histo::of_rgba(&pic.rgba)));
    }
    Ok(pic)
}

fn to_picture(img: image::DynamicImage, original: Option<(u32, u32)>) -> Picture {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Picture { width: w as usize, height: h as usize, rgba: rgba.into_raw(), original, hist: None }
}

/// Size of the original from its header only (no decode).
pub fn image_size(p: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(p).ok()?.with_guessed_format().ok()?.into_dimensions().ok()
}
