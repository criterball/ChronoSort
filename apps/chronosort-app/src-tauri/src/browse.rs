//! The File browser: drives, folder listings and thumbnails for any listed file.
//!
//! Safety rules (same spirit as `thumb://`):
//! * Listing never reads file contents (names, sizes and dates only), so it is
//!   fast even for 10 000 files and never touches a damaged file.
//! * The web UI never sends a path to `fthumb://`. It sends an id handed out by
//!   the latest listings; only those files can be previewed.
//! * Nothing here changes a file. Housekeeping lives in `fileops.rs`.

use crate::{AppState, Res, err};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use tauri::State;

/// More registered files than this and the oldest listings are forgotten.
const MAX_IDS: usize = 400_000;

/// id -> file, for the files of recent listings.
#[derive(Default)]
pub struct Registry {
    inner: Mutex<Reg>,
}

#[derive(Default)]
struct Reg {
    next: u64,
    by_id: HashMap<u64, PathBuf>,
    by_path: HashMap<PathBuf, u64>,
}

impl Registry {
    /// Stable id for a path (the same path keeps its id while remembered).
    pub fn id_for(&self, p: &Path) -> u64 {
        let mut r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&id) = r.by_path.get(p) {
            return id;
        }
        if r.by_id.len() >= MAX_IDS {
            r.by_id.clear();
            r.by_path.clear();
        }
        r.next += 1;
        let id = r.next;
        r.by_id.insert(id, p.to_path_buf());
        r.by_path.insert(p.to_path_buf(), id);
        id
    }

    pub fn path(&self, id: u64) -> Option<PathBuf> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).by_id.get(&id).cloned()
    }
}

/// What kind of file this looks like, from its extension only (fast).
/// The thumbnail code later checks the real content.
pub fn kind_of(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "jpe" | "jfif" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "heif" | "hif"
        | "avif" => "image",
        "cr2" | "cr3" | "crw" | "nef" | "nrw" | "dng" | "arw" | "srf" | "sr2" | "orf" | "pef" | "rw2" | "raf"
        | "3fr" | "erf" | "kdc" | "mrw" | "srw" | "x3f" | "rwl" | "iiq" => "raw",
        "mp4" | "m4v" | "mov" | "qt" | "3gp" | "3g2" | "avi" | "mkv" | "wmv" | "asf" | "mpg" | "mpeg" | "vob"
        | "mts" | "m2ts" | "ts" | "webm" | "flv" | "mod" | "tod" => "video",
        "xmp" | "aae" | "thm" | "json" => "sidecar",
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "oga" | "opus" | "wma" | "aif" | "aiff" | "amr" => "audio",
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "xls" | "xlsx" | "ods" | "csv" | "ppt" | "pptx"
        | "odp" | "htm" | "html" | "epub" => "document",
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "cab" | "iso" => "archive",
        "exe" | "msi" | "bat" | "cmd" | "ps1" | "com" | "scr" | "lnk" | "url" | "app" | "dmg" | "sh" | "appimage"
        | "deb" | "rpm" => "program",
        _ => "other",
    }
}

#[derive(Serialize)]
pub struct Entry {
    pub id: u64,
    pub name: String,
    pub dir: bool,
    /// "folder" | "image" | "raw" | "video" | "sidecar" | "audio" | "document" | "archive" | "program" | "other"
    pub kind: &'static str,
    /// Lower-case extension without the dot ("" if none).
    pub ext: String,
    pub size: u64,
    /// Last modified, Unix seconds (0 if unknown).
    pub modified: i64,
    pub hidden: bool,
}

#[derive(Serialize)]
pub struct Listing {
    /// The folder, as the user should see it (no `\\?\` prefix).
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<Entry>,
    /// Entries left out because they are hidden or system files.
    pub hidden_count: usize,
    /// Entries that could not be read (permissions, vanished while listing).
    pub unreadable: usize,
}

#[derive(Serialize)]
pub struct Roots {
    pub volumes: Vec<cs_platform::Volume>,
    pub known: Vec<cs_platform::KnownFolder>,
    pub sep: char,
}

#[tauri::command]
pub fn browse_roots() -> Roots {
    Roots { volumes: cs_platform::volumes(), known: cs_platform::known_folders(), sep: std::path::MAIN_SEPARATOR }
}

fn modified_secs(m: &std::fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(windows)]
fn is_hidden(name: &str, m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    m.file_attributes() & (HIDDEN | SYSTEM) != 0 || name.starts_with('.')
}
#[cfg(not(windows))]
fn is_hidden(name: &str, _m: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

fn shown(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or_else(|| s.into_owned())
}

/// Read one folder. Folders first, then files, both by name (natural order is
/// done in the UI, which also sorts by date/size/type on request).
pub fn list(reg: &Registry, folder: &Path, show_hidden: bool) -> Res<Listing> {
    let folder = std::path::absolute(folder).map_err(err)?;
    let rd = std::fs::read_dir(&folder).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "This folder does not exist (any more).".to_string(),
        std::io::ErrorKind::PermissionDenied => {
            "Windows does not allow opening this folder (access denied).".to_string()
        }
        _ => format!("Could not open this folder: {e}"),
    })?;
    let (mut entries, mut hidden_count, mut unreadable) = (Vec::new(), 0usize, 0usize);
    for de in rd {
        let Ok(de) = de else {
            unreadable += 1;
            continue;
        };
        let name = de.file_name().to_string_lossy().into_owned();
        let path = de.path();
        // DirEntry metadata does not follow links; follow them so a linked folder opens like a folder.
        // "Hidden" comes from the link itself: Windows' compatibility junctions
        // (Application Data, My Documents …) are hidden/system links to visible folders.
        let (meta, hidden) = match de.metadata() {
            Ok(m) => {
                let hidden = is_hidden(&name, &m);
                (if m.file_type().is_symlink() { std::fs::metadata(&path).unwrap_or(m) } else { m }, hidden)
            }
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        if hidden && !show_hidden {
            hidden_count += 1;
            continue;
        }
        let dir = meta.is_dir();
        let ext = if dir {
            String::new()
        } else {
            path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
        };
        let kind = if dir { "folder" } else { kind_of(&ext) };
        entries.push(Entry {
            id: reg.id_for(&path),
            name,
            dir,
            kind,
            ext,
            size: if dir { 0 } else { meta.len() },
            modified: modified_secs(&meta),
            hidden,
        });
    }
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(Listing { path: shown(&folder), parent: folder.parent().map(shown), entries, hidden_count, unreadable })
}

#[tauri::command]
pub async fn list_dir(state: State<'_, AppState>, path: String, show_hidden: bool) -> Res<Listing> {
    // Network folders can be slow: keep the UI thread free.
    let reg = &state.browse;
    let p = PathBuf::from(path.trim());
    if p.as_os_str().is_empty() {
        return Err("Type a folder, for example C:\\Users".into());
    }
    list(reg, &p, show_hidden)
}

/// Sub-folders only (for the folder tree). Hidden folders stay out unless asked.
#[tauri::command]
pub async fn list_subdirs(state: State<'_, AppState>, path: String, show_hidden: bool) -> Res<Vec<Entry>> {
    let l = list(&state.browse, Path::new(&path), show_hidden)?;
    Ok(l.entries.into_iter().filter(|e| e.dir).collect())
}

/// Up to 4 photos inside a folder, for the folder tile. Never reads deeper
/// than the folder itself, and gives up after 2 000 entries.
#[tauri::command]
pub async fn folder_samples(state: State<'_, AppState>, id: u64) -> Res<Vec<u64>> {
    let dir = state.browse.path(id).ok_or("unknown folder")?;
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut pics: Vec<PathBuf> = rd
        .take(2000)
        .flatten()
        .filter(|d| d.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|d| d.path())
        .filter(|p| {
            let e = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            matches!(kind_of(&e), "image" | "raw")
        })
        .collect();
    pics.sort();
    Ok(pics.into_iter().take(4).map(|p| state.browse.id_for(&p)).collect())
}

/// Full size of a picture in pixels, from its header only (no decoding), so the
/// viewer can show zoom relative to the real picture, not its preview.
/// Stored size: the UI swaps it when the preview is upright the other way (EXIF turn).
/// `None` for formats the header reader does not know (e.g. HEIC, RAW).
pub fn image_size_of(p: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(p).ok()?.with_guessed_format().ok()?.into_dimensions().ok()
}

#[derive(Serialize)]
pub struct ImageSize {
    pub w: u32,
    pub h: u32,
    /// `fthumb://<id>_o` can serve the original (the webview draws this format).
    pub original: bool,
}

#[tauri::command]
pub async fn image_size(state: State<'_, AppState>, id: u64) -> Res<Option<ImageSize>> {
    let p = state.browse.path(id).ok_or("unknown file")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (w, h) = image_size_of(&p)?;
        let small_enough = std::fs::metadata(&p).is_ok_and(|m| m.len() <= MAX_ORIGINAL);
        Some(ImageSize { w, h, original: small_enough && native_type(real_format(&p)).is_some() })
    })
    .await
    .map_err(err)
}

/// Show a listed file or folder in Explorer / Finder / Files.
#[tauri::command]
pub fn reveal_item(state: State<'_, AppState>, id: u64) -> Res<()> {
    let p = state.browse.path(id).ok_or("unknown item")?;
    if p.is_dir() { cs_platform::open_in_file_manager(&p) } else { cs_platform::show_file_in_file_manager(&p) }
        .map_err(err)
}

/// Thumbnail sizes the UI may ask for: small/medium share the 320 px cache,
/// large uses 640 px, `p` is the viewer preview.
fn size_for(code: &str) -> Option<u32> {
    match code {
        "s" | "m" => Some(cs_thumb::DEFAULT_SIZE),
        "l" => Some(640),
        "p" => Some(cs_thumb::PREVIEW_SIZE),
        _ => None,
    }
}

/// Cache key for a file by location + size + modified time: a changed file
/// gets a new thumbnail, an unchanged one is never decoded twice.
fn cache_key(p: &Path, m: &std::fs::Metadata) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"fthumb1\0");
    h.update(p.as_os_str().as_encoded_bytes());
    h.update(&m.len().to_le_bytes());
    let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    h.update(&t.to_le_bytes());
    h.finalize().to_hex()[..32].to_string()
}

pub(crate) fn real_format(p: &Path) -> cs_scan::Format {
    use std::io::Read;
    let mut head = [0u8; 512];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    cs_scan::sniff(&head[..n], &ext)
}

/// Content type of a picture the webview can draw itself (sniffed, not by name).
fn native_type(fmt: cs_scan::Format) -> Option<&'static str> {
    use cs_scan::Format::*;
    Some(match fmt {
        Jpeg => "image/jpeg",
        Png => "image/png",
        Gif => "image/gif",
        Bmp => "image/bmp",
        Webp => "image/webp",
        _ => return None,
    })
}

/// Largest original the viewer loads at full size (bigger ones stay at preview size).
const MAX_ORIGINAL: u64 = 256 * 1024 * 1024;

/// The viewer's preview of a photo or video (JPEG bytes from the thumbnail cache).
pub(crate) fn preview_bytes(state: &AppState, id: u64) -> Res<Vec<u8>> {
    let file = state.browse.path(id).ok_or("unknown file")?;
    let meta = std::fs::metadata(&file).map_err(err)?;
    let fmt = real_format(&file);
    if fmt.kind() != cs_model::MediaKind::Image && fmt.kind() != cs_model::MediaKind::Video {
        return Err("not a photo or video".into());
    }
    let size = size_for("p").ok_or("bad size")?;
    let p = state.thumbs.get_sized(&cache_key(&file, &meta), &file, fmt, size).map_err(err)?;
    std::fs::read(p).map_err(err)
}

/// `fthumb://<id>_<s|m|l|p|o>`; `o` = the original file, only for formats the webview draws.
pub fn fthumb_response(app: &tauri::AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let mut ctype = "image/jpeg";
    let result = (|| -> Res<Vec<u8>> {
        let (id, code) = path.trim_start_matches('/').split_once('_').ok_or("bad request")?;
        let file = state.browse.path(id.parse().map_err(err)?).ok_or("unknown file")?;
        let meta = std::fs::metadata(&file).map_err(err)?;
        let fmt = real_format(&file);
        if code == "o" {
            ctype = native_type(fmt).ok_or("not drawn at full size")?;
            if meta.len() > MAX_ORIGINAL {
                return Err("too large".into());
            }
            return std::fs::read(&file).map_err(err);
        }
        let size = size_for(code).ok_or("bad size")?;
        if fmt.kind() != cs_model::MediaKind::Image && fmt.kind() != cs_model::MediaKind::Video {
            return Err("not a photo or video".into());
        }
        let p = state.thumbs.get_sized(&cache_key(&file, &meta), &file, fmt, size).map_err(err)?;
        std::fs::read(p).map_err(err)
    })();
    // Not "immutable": the id stays the same when a file is edited in place.
    let b = tauri::http::Response::builder().header("Cache-Control", "no-cache");
    match result {
        Ok(bytes) => b.header("Content-Type", ctype).status(200).body(bytes),
        Err(e) => b.header("Content-Type", "text/plain").status(404).body(e.into_bytes()),
    }
    .expect("static response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_first_and_hides_dotfiles() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("b_folder")).unwrap();
        std::fs::write(d.path().join("a.JPG"), b"x").unwrap();
        std::fs::write(d.path().join("notes.txt"), b"hello").unwrap();
        std::fs::write(d.path().join(".hidden"), b"").unwrap();
        let reg = Registry::default();
        let l = list(&reg, d.path(), false).unwrap();
        let names: Vec<_> = l.entries.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(names, [("b_folder", "folder"), ("a.JPG", "image"), ("notes.txt", "document")]);
        assert_eq!(l.hidden_count, 1);
        assert_eq!(l.entries[2].size, 5);
        assert_eq!(list(&reg, d.path(), true).unwrap().entries.len(), 4);
        // Ids are stable and resolve back to the file.
        let id = l.entries[1].id;
        assert_eq!(reg.path(id).unwrap(), d.path().join("a.JPG"));
        assert_eq!(reg.id_for(&d.path().join("a.JPG")), id);
    }

    #[test]
    fn missing_folder_is_a_friendly_error() {
        let reg = Registry::default();
        let e = list(&reg, Path::new("Z:/surely/not/here/123"), false).err().unwrap();
        assert!(e.contains("does not exist") || e.contains("Could not open"), "{e}");
    }

    #[test]
    fn ten_thousand_files_list_fast() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..10_000 {
            std::fs::write(d.path().join(format!("IMG_{i:05}.jpg")), b"").unwrap();
        }
        let reg = Registry::default();
        let t = std::time::Instant::now();
        let l = list(&reg, d.path(), false).unwrap();
        let ms = t.elapsed().as_millis();
        assert_eq!(l.entries.len(), 10_000);
        assert!(ms < 1000, "listing took {ms} ms");
    }

    #[test]
    fn cache_key_changes_with_content_time() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.jpg");
        std::fs::write(&f, b"1").unwrap();
        let k1 = cache_key(&f, &std::fs::metadata(&f).unwrap());
        std::fs::write(&f, b"22").unwrap();
        let k2 = cache_key(&f, &std::fs::metadata(&f).unwrap());
        assert_ne!(k1, k2);
        assert_eq!(k1.len(), 32);
    }

    #[test]
    fn full_size_from_header_and_native_formats() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("wide.jpg");
        image::RgbImage::from_pixel(300, 200, image::Rgb([9, 9, 9])).save(&f).unwrap();
        assert_eq!(image_size_of(&f), Some((300, 200)));
        // Wrong extension: sniffed by content, not by name.
        let g = d.path().join("really_jpeg.png");
        std::fs::copy(&f, &g).unwrap();
        assert_eq!(image_size_of(&g), Some((300, 200)));
        assert_eq!(native_type(real_format(&g)), Some("image/jpeg"));
        let junk = d.path().join("junk.jpg");
        std::fs::write(&junk, b"not a picture").unwrap();
        assert_eq!(image_size_of(&junk), None);
        assert_eq!(native_type(cs_scan::Format::Tiff), None);
    }
}
