//! `cs-app`: ChronoSort's application logic without any user interface: the edit session, photo information
//! (ExifTool), lossless crop and removing information, wallpaper, file housekeeping with Undo. The native UI
//! (`apps/chronosort-native`) calls it from worker threads. Moved here from the Tauri app's `src-tauri`
//! (2026-11-27); the logic is the same, only the Tauri glue is gone.

pub mod batch;
pub mod create;
pub mod edit;
pub mod exiftool;
pub mod fileops;
pub mod meta;
pub mod print;
pub mod privacy;
pub mod rating;
pub mod wallpaper;

use std::path::Path;

pub type Res<T> = Result<T, String>;

pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn now_unix() -> i64 {
    cs_model::civil::system_to_unix(std::time::SystemTime::now())
}

/// Progress of a job over many files.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub bytes: u64,
}

/// The real format from the first bytes (a renamed file is still recognised).
pub fn real_format(p: &Path) -> cs_scan::Format {
    use std::io::Read;
    let mut head = [0u8; 512];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    cs_scan::sniff(&head[..n], &ext)
}

/// Open any picture, RAW files through their embedded preview when the codecs cannot read them.
pub fn open_any(p: &Path) -> Res<cs_image::Photo> {
    match cs_image::open(p) {
        Ok(ph) => Ok(ph),
        Err(e) => {
            if real_format(p).kind() == cs_model::MediaKind::Image
                && let Some(jpg) = cs_meta::raw_preview(p)
            {
                return cs_image::open_bytes(&jpg).map_err(|_| e);
            }
            Err(e)
        }
    }
}
