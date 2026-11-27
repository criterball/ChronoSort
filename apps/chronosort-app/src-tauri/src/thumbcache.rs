//! Thumbnail cache tools (Settings → Thumbnails, and right-click a folder):
//! where the cache is and how big, empty it, keep it under a size limit (the
//! thumbnails not looked at for the longest time go first), remove an old cache
//! left in the PC's own folders, and make thumbnails for a folder in advance so
//! browsing it later is instant. Thumbnails are only small copies; deleting
//! them never touches a photo.

use crate::{AppState, Res, err};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tauri::{Emitter, Manager, State};

static CANCEL: AtomicBool = AtomicBool::new(false);
static BUSY: AtomicBool = AtomicBool::new(false);

/// `ChronoSortData\cache\thumbs` next to the program, if that folder can be written.
pub fn beside_program() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join(cs_ai::PORTABLE_DIR).join("cache");
    std::fs::create_dir_all(&dir).ok()?;
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    std::fs::write(&probe, b"ok").ok()?;
    let _ = std::fs::remove_file(&probe);
    Some(dir)
}

/// Is this a file the cache made? (`<2 hex>/<hex…>.jpg` or a leftover `.tmp…`.)
fn is_cache_file(dir: &Path, p: &Path) -> bool {
    let Ok(rel) = p.strip_prefix(dir) else { return false };
    let parts: Vec<_> = rel.components().collect();
    if parts.len() != 2 {
        return false;
    }
    let sub = parts[0].as_os_str().to_string_lossy();
    let name = parts[1].as_os_str().to_string_lossy();
    sub.len() == 2
        && sub.bytes().all(|b| b.is_ascii_hexdigit())
        && name.len() > 16
        && name.bytes().take(16).all(|b| b.is_ascii_hexdigit())
        && (name.ends_with(".jpg") || name.contains(".tmp"))
}

/// Every cache file: (path, size, last used).
fn list(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for sub in rd.flatten() {
        if !sub.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(sub.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if !is_cache_file(dir, &p) {
                continue;
            }
            if let Ok(m) = f.metadata()
                && m.is_file()
            {
                out.push((p, m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
            }
        }
    }
    out
}

/// Remove the given files and any sub-folders left empty.
fn remove(dir: &Path, files: &[(PathBuf, u64, std::time::SystemTime)]) -> (usize, u64) {
    let (mut n, mut bytes) = (0, 0);
    for (p, len, _) in files {
        if std::fs::remove_file(p).is_ok() {
            n += 1;
            bytes += len;
        }
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for sub in rd.flatten() {
            let _ = std::fs::remove_dir(sub.path()); // only succeeds when empty
        }
    }
    (n, bytes)
}

/// The PC's own cache folder from older ChronoSort versions, if it is not the one in use.
fn old_dir(app: &tauri::AppHandle, current: &Path) -> Option<PathBuf> {
    let d = app.path().app_cache_dir().ok()?.join("thumbs");
    let same = match (std::fs::canonicalize(&d), std::fs::canonicalize(current)) {
        (Ok(a), Ok(b)) => a == b,
        _ => d == current,
    };
    (!same && d.is_dir()).then_some(d)
}

#[derive(Serialize)]
pub struct CacheInfo {
    pub dir: String,
    pub files: usize,
    pub bytes: u64,
    /// True when the cache is next to the program (travels with it).
    pub beside_program: bool,
    pub old_dir: Option<String>,
    pub old_bytes: u64,
}

#[tauri::command]
pub async fn thumb_cache_info(app: tauri::AppHandle) -> Res<CacheInfo> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let dir = state.thumbs.dir().to_path_buf();
        let files = list(&dir);
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
        let beside = exe_dir.is_some_and(|e| dir.starts_with(e));
        let old = old_dir(&app, &dir);
        let old_bytes = old.as_ref().map_or(0, |d| list(d).iter().map(|f| f.1).sum());
        Ok(CacheInfo {
            dir: dir.display().to_string(),
            files: files.len(),
            bytes: files.iter().map(|f| f.1).sum(),
            beside_program: beside,
            old_dir: old.filter(|_| old_bytes > 0).map(|d| d.display().to_string()),
            old_bytes,
        })
    })
    .await
    .map_err(err)?
}

#[derive(Serialize)]
pub struct Removed {
    pub files: usize,
    pub bytes: u64,
}

/// Empty the thumbnail cache (they are made again when needed).
#[tauri::command]
pub async fn thumb_cache_clear(app: tauri::AppHandle) -> Res<Removed> {
    if BUSY.load(Ordering::SeqCst) {
        return Err("Thumbnails are being made right now; stop that first.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let dir = app.state::<AppState>().thumbs.dir().to_path_buf();
        let (files, bytes) = remove(&dir, &list(&dir));
        Ok(Removed { files, bytes })
    })
    .await
    .map_err(err)?
}

/// Keep the cache under `max_mb` megabytes: the thumbnails not used for the longest time go first.
#[tauri::command]
pub async fn thumb_cache_trim(app: tauri::AppHandle, max_mb: u64) -> Res<Removed> {
    if BUSY.load(Ordering::SeqCst) {
        return Ok(Removed { files: 0, bytes: 0 });
    }
    tauri::async_runtime::spawn_blocking(move || {
        let dir = app.state::<AppState>().thumbs.dir().to_path_buf();
        let (files, bytes) = remove(&dir, &over_limit(list(&dir), max_mb.max(50) * 1024 * 1024));
        Ok(Removed { files, bytes })
    })
    .await
    .map_err(err)?
}

/// The oldest-used files to remove so the rest fits in `max` bytes.
fn over_limit(
    mut files: Vec<(PathBuf, u64, std::time::SystemTime)>,
    max: u64,
) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max {
        return Vec::new();
    }
    files.sort_by_key(|f| f.2);
    let mut out = Vec::new();
    for f in files {
        if total <= max {
            break;
        }
        total -= f.1;
        out.push(f);
    }
    out
}

/// Remove the cache an older ChronoSort left in the PC's own folders.
#[tauri::command]
pub async fn thumb_cache_clear_old(app: tauri::AppHandle) -> Res<Removed> {
    tauri::async_runtime::spawn_blocking(move || {
        let current = app.state::<AppState>().thumbs.dir().to_path_buf();
        let Some(old) = old_dir(&app, &current) else { return Ok(Removed { files: 0, bytes: 0 }) };
        let (files, bytes) = remove(&old, &list(&old));
        let _ = std::fs::remove_dir(&old);
        Ok(Removed { files, bytes })
    })
    .await
    .map_err(err)?
}

#[derive(Clone, Serialize)]
struct Progress {
    done: usize,
    total: usize,
    name: String,
}

#[derive(Serialize, Default)]
pub struct Prescan {
    pub photos: usize,
    pub made: usize,
    pub failed: usize,
    pub cancelled: bool,
}

#[tauri::command]
pub fn thumb_cache_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Make the thumbnails of every photo and video in `path` now (sub-folders too if
/// `recursive`), in the given sizes (longest side in pixels).
#[tauri::command]
pub async fn thumb_cache_prescan(
    app: tauri::AppHandle,
    path: String,
    recursive: bool,
    sizes: Vec<u32>,
) -> Res<Prescan> {
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("Thumbnails are already being made.".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let r = tauri::async_runtime::spawn_blocking(move || prescan(&app, Path::new(&path), recursive, &sizes)).await;
    BUSY.store(false, Ordering::SeqCst);
    r.map_err(err)?
}

fn prescan(app: &tauri::AppHandle, root: &Path, recursive: bool, sizes: &[u32]) -> Res<Prescan> {
    let state: State<'_, AppState> = app.state::<AppState>();
    let sizes: Vec<u32> = sizes.iter().copied().filter(|s| (64..=4096).contains(s)).collect();
    if sizes.is_empty() {
        return Err("No thumbnail size chosen.".into());
    }
    let _ = app.emit("cache-progress", Progress { done: 0, total: 0, name: "Looking for photos…".into() });
    let scan = cs_scan::scan(root).map_err(|e| format!("This folder cannot be read: {e}"))?;
    let files: Vec<cs_scan::FileEntry> = scan
        .files
        .into_iter()
        .filter(|f| matches!(f.kind, cs_model::MediaKind::Image | cs_model::MediaKind::Video))
        .filter(|f| recursive || !f.rel.contains('/'))
        .filter(|f| !f.rel.split('/').any(|part| part.starts_with('.')))
        .collect();
    let total = files.len();
    let done = AtomicUsize::new(0);
    let made = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    use rayon::prelude::*;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4))
        .build()
        .map_err(err)?;
    pool.install(|| {
        files.par_iter().for_each(|f| {
            if CANCEL.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(meta) = std::fs::metadata(&f.abs) {
                let key = crate::browse::cache_key(&f.abs, &meta);
                let fmt = crate::browse::real_format(&f.abs);
                let mut ok = true;
                for &s in &sizes {
                    ok &= state.thumbs.get_sized(&key, &f.abs, fmt, s).is_ok();
                }
                if ok { &made } else { &failed }.fetch_add(1, Ordering::SeqCst);
            } else {
                failed.fetch_add(1, Ordering::SeqCst);
            }
            let n = done.fetch_add(1, Ordering::SeqCst) + 1;
            if n.is_multiple_of(5) || n == total {
                let _ = app.emit("cache-progress", Progress { done: n, total, name: f.name.clone() });
            }
        });
    });
    Ok(Prescan {
        photos: total,
        made: made.load(Ordering::SeqCst),
        failed: failed.load(Ordering::SeqCst),
        cancelled: CANCEL.load(Ordering::SeqCst),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn only_cache_files_are_touched() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path();
        std::fs::create_dir_all(dir.join("ab")).unwrap();
        std::fs::create_dir_all(dir.join("photos")).unwrap();
        std::fs::write(dir.join("ab/abcdef0123456789abcdef0123456789.jpg"), b"x").unwrap();
        std::fs::write(dir.join("ab/abcdef0123456789abcdef0123456789_640.jpg"), b"xy").unwrap();
        std::fs::write(dir.join("ab/notes.txt"), b"keep").unwrap();
        std::fs::write(dir.join("photos/IMG_0001.jpg"), b"keep").unwrap();
        std::fs::write(dir.join("readme.jpg"), b"keep").unwrap();
        let l = list(dir);
        assert_eq!(l.len(), 2);
        assert_eq!(remove(dir, &l), (2, 3));
        assert!(dir.join("ab/notes.txt").exists());
        assert!(dir.join("photos/IMG_0001.jpg").exists());
        assert!(dir.join("readme.jpg").exists());
    }

    #[test]
    fn trim_removes_the_least_recently_used_first() {
        let t = |s| UNIX_EPOCH + Duration::from_secs(s);
        let f = |n: &str, len, s| (PathBuf::from(n), len, t(s));
        let files = vec![f("new", 40, 300), f("old", 40, 100), f("mid", 40, 200)];
        let gone: Vec<_> = over_limit(files.clone(), 80).into_iter().map(|x| x.0).collect();
        assert_eq!(gone, [PathBuf::from("old")]);
        let gone: Vec<_> = over_limit(files.clone(), 40).into_iter().map(|x| x.0).collect();
        assert_eq!(gone, [PathBuf::from("old"), PathBuf::from("mid")]);
        assert!(over_limit(files, 120).is_empty());
    }
}
