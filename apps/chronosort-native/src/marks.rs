//! Stars and the tag mark of the shown folder's files (A5). Stars are read from the photos on a worker thread
//! (header only, in parts, so a big folder fills in as it goes); the tag list is ChronoSort's own
//! `ChronoSortData\tagged.json` (`cs_app::rating`). Writing stars is a browser / viewer job (with Undo).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};

/// Which files the browser shows (for this session only, like the Tauri app).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StarFilter {
    #[default]
    Any,
    /// At least this many stars (5 = five only).
    AtLeast(u8),
    /// Photos without stars yet.
    NoStars,
    Tagged,
}

impl StarFilter {
    pub fn label(self) -> String {
        match self {
            StarFilter::Any => "Any stars".into(),
            StarFilter::AtLeast(5) => "★★★★★ only".into(),
            StarFilter::AtLeast(n) => format!("{} or more", "★".repeat(n as usize)),
            StarFilter::NoStars => "No stars yet".into(),
            StarFilter::Tagged => "Tagged".into(),
        }
    }
    pub const ALL: [StarFilter; 8] = [
        StarFilter::Any,
        StarFilter::AtLeast(1),
        StarFilter::AtLeast(2),
        StarFilter::AtLeast(3),
        StarFilter::AtLeast(4),
        StarFilter::AtLeast(5),
        StarFilter::NoStars,
        StarFilter::Tagged,
    ];
}

type Batch = (u64, Vec<(PathBuf, Option<u8>)>);

pub struct Marks {
    /// Stars of the files read so far (`None` = cannot have stars, e.g. a video).
    stars: HashMap<PathBuf, Option<u8>>,
    tagged: BTreeSet<String>,
    data: PathBuf,
    folder: Option<PathBuf>,
    generation: Arc<AtomicU64>,
    rx: Option<Receiver<Batch>>,
    pub loading: bool,
}

impl Marks {
    pub fn new(data: &Path) -> Self {
        Self {
            stars: HashMap::new(),
            tagged: cs_app::rating::tagged_set(data).unwrap_or_default(),
            data: data.to_path_buf(),
            folder: None,
            generation: Arc::new(AtomicU64::new(0)),
            rx: None,
            loading: false,
        }
    }

    /// Read the stars of these files (the folder's photos). Earlier values of the same folder stay shown
    /// until the new ones arrive, so a refresh does not flicker.
    pub fn load(&mut self, folder: &Path, files: Vec<PathBuf>, ctx: &egui::Context) {
        if self.folder.as_deref() != Some(folder) {
            self.stars.clear();
            self.folder = Some(folder.to_path_buf());
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.loading = !files.is_empty();
        let (current, ctx) = (self.generation.clone(), ctx.clone());
        std::thread::spawn(move || {
            for part in files.chunks(200) {
                if current.load(Ordering::SeqCst) != generation {
                    return; // another folder by now
                }
                let v: Vec<(PathBuf, Option<u8>)> =
                    part.iter().map(|p| (p.clone(), cs_app::rating::stars_of(p))).collect();
                if tx.send((generation, v)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
    }

    /// Take what the reader found; `true` when anything changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let now = self.generation.load(Ordering::SeqCst);
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok((g, v)) if g == now => {
                    for (p, s) in v {
                        if self.stars.insert(p, s) != Some(s) {
                            changed = true;
                        }
                    }
                }
                Ok(_) => {}
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    self.loading = false;
                    break;
                }
            }
        }
        changed
    }

    /// Stars of a file: `Some(0..=5)` for a photo, `None` for other files or while not read yet.
    pub fn stars(&self, p: &Path) -> Option<u8> {
        self.stars.get(p).copied().flatten()
    }

    pub fn set_stars(&mut self, p: &Path, n: u8) {
        self.stars.insert(p.to_path_buf(), Some(n));
    }

    pub fn tagged(&self, p: &Path) -> bool {
        cs_app::rating::is_tagged(&self.tagged, p)
    }

    /// Tag or untag files (ChronoSort's own list; the files are not touched).
    pub fn set_tagged(&mut self, files: &[PathBuf], on: bool) -> Result<(), String> {
        cs_app::rating::set_tagged(&self.data, files, on)?;
        self.tagged = cs_app::rating::tagged_set(&self.data)?;
        Ok(())
    }

    /// Does this entry pass the filter? Folders always do.
    pub fn pass(&self, f: StarFilter, p: &Path, folder: bool) -> bool {
        if folder {
            return true;
        }
        match f {
            StarFilter::Any => true,
            StarFilter::Tagged => self.tagged(p),
            StarFilter::NoStars => self.stars.get(p) == Some(&Some(0)),
            StarFilter::AtLeast(n) => self.stars(p).unwrap_or(0) >= n,
        }
    }
}

/// "★★★" for 3.
pub fn star_text(n: u8) -> String {
    "★".repeat(n as usize)
}

/// "★★★☆☆" for 3.
pub fn star_text5(n: u8) -> String {
    format!("{}{}", "★".repeat(n as usize), "☆".repeat(5usize.saturating_sub(n as usize)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters() {
        let d = std::env::temp_dir().join(format!("csn_marks_{}", std::process::id()));
        let mut m = Marks::new(&d);
        let (a, b, v) = (PathBuf::from("a.jpg"), PathBuf::from("b.jpg"), PathBuf::from("v.mp4"));
        m.stars.insert(a.clone(), Some(4));
        m.stars.insert(b.clone(), Some(0));
        m.stars.insert(v.clone(), None);
        assert!(m.pass(StarFilter::AtLeast(3), &a, false));
        assert!(!m.pass(StarFilter::AtLeast(3), &b, false));
        assert!(m.pass(StarFilter::NoStars, &b, false));
        assert!(!m.pass(StarFilter::NoStars, &v, false), "a video has no stars, but is not 'no stars yet'");
        assert!(m.pass(StarFilter::Tagged, Path::new("folder"), true));
        assert_eq!(star_text5(2), "★★☆☆☆");
        let _ = std::fs::remove_dir_all(&d);
    }
}
