//! Folder listings for the File browser. Names, sizes and dates only: listing never reads file
//! contents, so it stays fast for 10 000 files and never touches a damaged file.
//! (Same rules as the Tauri app's `browse.rs`; the two will share one crate once the move is done.)

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Image,
    Raw,
    Video,
    Audio,
    Other,
}

impl Kind {
    pub fn has_thumbnail(self) -> bool {
        matches!(self, Kind::Image | Kind::Raw | Kind::Video)
    }
    pub fn viewable(self) -> bool {
        self.has_thumbnail()
    }
}

/// From the extension only (fast). The thumbnail code later checks the real content.
pub fn kind_of(ext: &str) -> Kind {
    match ext {
        "jpg" | "jpeg" | "jpe" | "jfif" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "heif" | "hif"
        | "avif" => Kind::Image,
        "cr2" | "cr3" | "crw" | "nef" | "nrw" | "dng" | "arw" | "srf" | "sr2" | "orf" | "pef" | "rw2" | "raf"
        | "3fr" | "erf" | "kdc" | "mrw" | "srw" | "x3f" | "rwl" | "iiq" => Kind::Raw,
        "mp4" | "m4v" | "mov" | "qt" | "3gp" | "3g2" | "avi" | "mkv" | "wmv" | "asf" | "mpg" | "mpeg" | "vob"
        | "mts" | "m2ts" | "ts" | "webm" | "flv" | "mod" | "tod" => Kind::Video,
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "oga" | "opus" | "wma" | "aif" | "aiff" | "amr" => Kind::Audio,
        _ => Kind::Other,
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub ext: String,
    pub size: u64,
    /// Unix seconds, 0 if unknown.
    pub modified: i64,
}

pub struct Listing {
    pub path: PathBuf,
    /// What is shown: `all` filtered (stars, tag, name) and sorted.
    pub entries: Vec<Entry>,
    /// Everything in the folder.
    pub all: Vec<Entry>,
    pub hidden_count: usize,
    pub unreadable: usize,
}

#[cfg(windows)]
fn is_hidden(name: &str, m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & (0x2 | 0x4) != 0 || name.starts_with('.')
}
#[cfg(not(windows))]
fn is_hidden(name: &str, _m: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

/// The path as the user should see it (no `\\?\` prefix).
pub fn shown(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or_else(|| s.into_owned())
}

/// Natural order: "IMG_2" before "IMG_10".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Read one folder: folders first, then files, both in natural order.
pub fn list(folder: &Path, show_hidden: bool) -> Result<Listing, String> {
    let folder = std::path::absolute(folder).map_err(|e| e.to_string())?;
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
        let modified =
            meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
        entries.push(Entry {
            kind: if dir { Kind::Folder } else { kind_of(&ext) },
            size: if dir { 0 } else { meta.len() },
            name,
            path,
            ext,
            modified,
        });
    }
    entries.sort_by(|a, b| {
        (b.kind == Kind::Folder).cmp(&(a.kind == Kind::Folder)).then_with(|| natural_cmp(&a.name, &b.name))
    });
    Ok(Listing { path: folder, all: entries.clone(), entries, hidden_count, unreadable })
}

/// Sub-folders only, for the folder tree.
pub fn subfolders(folder: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|de| {
            let name = de.file_name().to_string_lossy().into_owned();
            let m = de.metadata().ok()?;
            if is_hidden(&name, &m) {
                return None;
            }
            let path = de.path();
            let is_dir = if m.file_type().is_symlink() { std::fs::metadata(&path).ok()?.is_dir() } else { m.is_dir() };
            is_dir.then_some((name, path))
        })
        .collect();
    out.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    out
}

/// Thumbnail cache key: location + size + modified time. Byte for byte the Tauri app's key, so
/// both apps share the thumbnails in `ChronoSortData\cache`.
pub fn cache_key(p: &Path, m: &std::fs::Metadata) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"fthumb1\0");
    h.update(p.as_os_str().as_encoded_bytes());
    h.update(&m.len().to_le_bytes());
    let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    h.update(&t.to_le_bytes());
    h.finalize().to_hex()[..32].to_string()
}

/// The real format from the first bytes (a renamed file is still recognised).
pub fn real_format(p: &Path) -> cs_scan::Format {
    use std::io::Read;
    let mut head = [0u8; 512];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    cs_scan::sniff(&head[..n], &ext)
}

pub fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} bytes") } else { format!("{v:.1} {}", U[i]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["IMG_10.jpg", "img_2.jpg", "IMG_1.jpg", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "IMG_1.jpg", "img_2.jpg", "IMG_10.jpg"]);
    }

    #[test]
    fn same_cache_key_as_the_tauri_app() {
        // The Tauri app hashes exactly these bytes; a changed file must change the key.
        let d = std::env::temp_dir().join(format!("csn_key_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("x.jpg");
        std::fs::write(&f, b"1").unwrap();
        let k1 = cache_key(&f, &std::fs::metadata(&f).unwrap());
        std::fs::write(&f, b"22").unwrap();
        let k2 = cache_key(&f, &std::fs::metadata(&f).unwrap());
        assert_ne!(k1, k2);
        assert_eq!(k1.len(), 32);
        let _ = std::fs::remove_dir_all(&d);
    }
}
