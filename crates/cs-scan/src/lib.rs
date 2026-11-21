//! `cs-scan`: walk a source tree, identify files by content, group files that
//! must travel together (RAW+JPEG, Live Photo, sidecars).
//!
//! Safety: never follows symlinks or junctions (a link could point outside the
//! source, or create loops), opens files read-only, reads only a small header.

use cs_model::MediaKind;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Container format detected from the first bytes of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Jpeg,
    Png,
    Gif,
    Bmp,
    Webp,
    /// TIFF and TIFF-based RAW (NEF, CR2, DNG, ARW, ORF, PEF …).
    Tiff,
    Heif,
    Avif,
    /// Canon CR3 (ISO-BMFF based RAW).
    Cr3,
    Mp4,
    Mov,
    ThreeGp,
    Avi,
    Mkv,
    Wmv,
    MpegPs,
    MpegTs,
    Xmp,
    Json,
    Unknown,
}

impl Format {
    pub fn name(self) -> &'static str {
        use Format::*;
        match self {
            Jpeg => "jpeg",
            Png => "png",
            Gif => "gif",
            Bmp => "bmp",
            Webp => "webp",
            Tiff => "tiff",
            Heif => "heif",
            Avif => "avif",
            Cr3 => "cr3",
            Mp4 => "mp4",
            Mov => "mov",
            ThreeGp => "3gp",
            Avi => "avi",
            Mkv => "mkv",
            Wmv => "wmv",
            MpegPs => "mpeg-ps",
            MpegTs => "mpeg-ts",
            Xmp => "xmp",
            Json => "json",
            Unknown => "unknown",
        }
    }

    /// Inverse of [`Format::name`].
    pub fn from_name(n: &str) -> Format {
        use Format::*;
        [Jpeg, Png, Gif, Bmp, Webp, Tiff, Heif, Avif, Cr3, Mp4, Mov, ThreeGp, Avi, Mkv, Wmv, MpegPs, MpegTs, Xmp, Json]
            .into_iter()
            .find(|f| f.name() == n)
            .unwrap_or(Unknown)
    }

    pub fn kind(self) -> MediaKind {
        use Format::*;
        match self {
            Jpeg | Png | Gif | Bmp | Webp | Tiff | Heif | Avif | Cr3 => MediaKind::Image,
            Mp4 | Mov | ThreeGp | Avi | Mkv | Wmv | MpegPs | MpegTs => MediaKind::Video,
            Xmp | Json => MediaKind::Sidecar,
            Unknown => MediaKind::Unknown,
        }
    }

    fn is_raw(self, ext: &str) -> bool {
        self == Format::Cr3 || (self == Format::Tiff && RAW_EXTS.contains(&ext))
    }
}

const RAW_EXTS: &[&str] =
    &["nef", "nrw", "cr2", "dng", "arw", "srf", "sr2", "orf", "pef", "rw2", "raf", "3fr", "erf", "kdc", "mrw", "srw"];

/// Identify a file from its first bytes. `ext` (lower-case) is only used to
/// tell sidecar text formats apart, never to decide image vs video.
pub fn sniff(head: &[u8], ext: &str) -> Format {
    let h = head;
    let at = |o: usize, s: &[u8]| h.len() >= o + s.len() && &h[o..o + s.len()] == s;
    if at(0, &[0xFF, 0xD8, 0xFF]) {
        return Format::Jpeg;
    }
    if at(0, b"\x89PNG\r\n\x1a\n") {
        return Format::Png;
    }
    if at(0, b"GIF87a") || at(0, b"GIF89a") {
        return Format::Gif;
    }
    if at(0, b"II*\0") || at(0, b"MM\0*") || at(0, b"IIRO") || at(0, b"IIU\0") {
        return Format::Tiff; // IIRO = Olympus ORF, IIU = Panasonic RW2
    }
    if at(0, b"FUJIFILMCCD-RAW") {
        return Format::Tiff;
    }
    if at(0, b"RIFF") && at(8, b"WEBP") {
        return Format::Webp;
    }
    if at(0, b"RIFF") && at(8, b"AVI ") {
        return Format::Avi;
    }
    if at(0, b"BM") && h.len() >= 26 && ext == "bmp" {
        return Format::Bmp;
    }
    if at(0, &[0x1A, 0x45, 0xDF, 0xA3]) {
        return Format::Mkv;
    }
    if at(0, &[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11]) {
        return Format::Wmv;
    }
    if at(0, &[0, 0, 1, 0xBA]) {
        return Format::MpegPs;
    }
    // MPEG-TS: sync byte every 188 bytes; AVCHD .mts/.m2ts has a 4-byte prefix (192).
    if h.len() >= 377 && h[0] == 0x47 && h[188] == 0x47 && h[376] == 0x47 {
        return Format::MpegTs;
    }
    if h.len() >= 389 && h[4] == 0x47 && h[196] == 0x47 && h[388] == 0x47 {
        return Format::MpegTs;
    }
    if at(4, b"ftyp") {
        let brand = &h[8..12.min(h.len())];
        return match brand {
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => Format::Heif,
            b"avif" | b"avis" => Format::Avif,
            b"crx " => Format::Cr3,
            b"qt  " => Format::Mov,
            b"3gp4" | b"3gp5" | b"3gp6" | b"3g2a" => Format::ThreeGp,
            _ => Format::Mp4,
        };
    }
    // Old QuickTime files without ftyp start directly with moov/mdat/wide/free.
    if at(4, b"moov") || at(4, b"mdat") || at(4, b"wide") || at(4, b"free") {
        return Format::Mov;
    }
    let text = trim_bom(h);
    match ext {
        "xmp" if text.starts_with(b"<") => Format::Xmp,
        "json" if text.starts_with(b"{") => Format::Json,
        _ => Format::Unknown,
    }
}

fn trim_bom(b: &[u8]) -> &[u8] {
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    let n = b.iter().take_while(|c| c.is_ascii_whitespace()).count();
    &b[n..]
}

/// OS clutter that must never be copied into the sorted tree.
pub fn is_junk(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    matches!(l.as_str(), "thumbs.db" | "ehthumbs.db" | "ehthumbs_vista.db" | "desktop.ini" | ".ds_store" | "icon\r")
        || l.starts_with("._") // macOS AppleDouble resource forks
        || l == ".picasa.ini"
        || l == "picasa.ini"
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub abs: PathBuf,
    /// Relative to the source root, `/`-separated (stable across OSes).
    pub rel: String,
    pub name: String,
    pub size: u64,
    pub format: Format,
    pub kind: MediaKind,
    /// True when the extension disagrees with the content (e.g. PNG named .jpg).
    pub ext_mismatch: bool,
    pub is_raw: bool,
    /// Group key; files with the same key travel together.
    pub group: String,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: Vec<FileEntry>,
    /// Paths that could not be read, with the reason. Never silently dropped.
    pub errors: Vec<(PathBuf, String)>,
    /// Symlinks/junctions that were not followed.
    pub skipped_links: Vec<PathBuf>,
}

const HEAD_LEN: usize = 512;

fn ext_matches(format: Format, ext: &str) -> bool {
    use Format::*;
    let ok: &[&str] = match format {
        Jpeg => &["jpg", "jpeg", "jpe", "jfif", "thm"],
        Png => &["png"],
        Gif => &["gif"],
        Bmp => &["bmp", "dib"],
        Webp => &["webp"],
        Tiff => &[
            "tif", "tiff", "nef", "nrw", "cr2", "dng", "arw", "srf", "sr2", "orf", "pef", "rw2", "raf", "3fr", "erf",
            "kdc", "mrw", "srw",
        ],
        Heif => &["heic", "heif", "hif"],
        Avif => &["avif"],
        Cr3 => &["cr3"],
        Mp4 => &["mp4", "m4v", "mov", "insv", "lrv"],
        Mov => &["mov", "qt", "mp4"],
        ThreeGp => &["3gp", "3g2"],
        Avi => &["avi"],
        Mkv => &["mkv", "webm"],
        Wmv => &["wmv", "asf"],
        MpegPs => &["mpg", "mpeg", "vob", "mod", "tod"],
        MpegTs => &["mts", "m2ts", "ts"],
        Xmp => &["xmp"],
        Json => &["json"],
        Unknown => return true,
    };
    ok.contains(&ext)
}

/// Lower-case extension of the file name.
fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Name with sidecar and media extensions removed, used for grouping.
/// `IMG_1234.jpg.json`, `IMG_1234.jpg.supplemental-metadata.json`,
/// `IMG_1234.xmp`, `IMG_1234.JPG.xmp` and `IMG_1234.NEF` all give `IMG_1234`.
///
/// Google Takeout quirks: a copy's number moves behind the extension
/// (`IMG_1234(1).jpg` has `IMG_1234.jpg(1).json`) and the
/// `.supplemental-metadata` part may be cut short (`IMG_1234.jpg.suppl.json`).
pub fn group_stem(name: &str) -> String {
    let mut s = name.to_string();
    let mut after_json = false;
    for _ in 0..4 {
        let p = Path::new(&s);
        let (Some(stem), Some(ext)) = (p.file_stem(), p.extension()) else { break };
        let ext = ext.to_string_lossy().to_ascii_lowercase();
        let stem = stem.to_string_lossy().into_owned();
        // "jpg(1)" -> media extension + copy number, which belongs to the stem.
        if after_json
            && let Some((e, n)) = ext.split_once('(')
            && is_media_ext(e)
            && n.strip_suffix(')').is_some_and(|d| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()))
        {
            s = format!("{stem}({n}");
            break;
        }
        let strip = matches!(ext.as_str(), "json" | "xmp" | "aae" | "thm")
            || ext.starts_with("supplemental")
            || (after_json && ext.len() >= 2 && "supplemental-metadata".starts_with(ext.as_str()))
            || is_media_ext(&ext);
        if !strip {
            break;
        }
        after_json = ext == "json";
        s = stem;
    }
    s
}

/// Takeout cuts long sidecar names to 51 characters ("…_long_name.j.json").
/// For such a JSON, the one media file in the same folder whose name starts
/// with the cut-off part.
fn truncated_takeout_partner(json_name: &str, media_names: &[(usize, String)]) -> Option<usize> {
    let lower = json_name.to_lowercase();
    let base = lower.strip_suffix(".json")?;
    // Also drop a cut-off ".supplemental-metadata" piece.
    let base = match base.rsplit_once('.') {
        Some((b, tail)) if tail.len() >= 2 && "supplemental-metadata".starts_with(tail) => b,
        _ => base,
    };
    if json_name.chars().count() < 40 {
        return None; // short names are never cut, so no guessing
    }
    let mut hits = media_names.iter().filter(|(_, n)| n.starts_with(base) && n.len() > base.len());
    let first = hits.next()?;
    hits.next().is_none().then_some(first.0)
}

fn is_media_ext(ext: &str) -> bool {
    const M: &[&str] = &[
        "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "tif", "tiff", "mov", "mp4", "m4v", "avi", "3gp", "dng",
        "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf", "pef",
    ];
    M.contains(&ext)
}

/// Walk `root` and identify every regular file.
pub fn scan(root: &Path) -> io::Result<ScanResult> {
    if !std::fs::metadata(root)?.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "source is not a folder"));
    }
    let mut out = ScanResult::default();
    // ChronoSort's own state folder (journal, reports) is never treated as photos.
    let walk = walkdir::WalkDir::new(root).follow_links(false).sort_by_file_name().into_iter();
    for item in walk.filter_entry(|e| e.depth() == 0 || e.file_name() != ".chronosort") {
        let ent = match item {
            Ok(e) => e,
            Err(e) => {
                let p = e.path().map(Path::to_path_buf).unwrap_or_default();
                out.errors.push((p, e.to_string()));
                continue;
            }
        };
        let ft = ent.file_type();
        if ft.is_symlink() || is_reparse_point(ent.path()) {
            out.skipped_links.push(ent.path().to_path_buf());
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        match identify(root, ent.path()) {
            Ok(f) => out.files.push(f),
            Err(e) => out.errors.push((ent.path().to_path_buf(), e.to_string())),
        }
    }
    assign_groups(&mut out.files);
    Ok(out)
}

/// Windows junctions are reparse points that walkdir reports as directories.
#[cfg(windows)]
fn is_reparse_point(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    std::fs::symlink_metadata(p).map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0).unwrap_or(false)
}
#[cfg(not(windows))]
fn is_reparse_point(_: &Path) -> bool {
    false
}

fn identify(root: &Path, path: &Path) -> io::Result<FileEntry> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let rel = path
        .strip_prefix(root)
        .map_err(io::Error::other)?
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    let size = std::fs::symlink_metadata(path)?.len();
    let ext = ext_of(&name);
    if is_junk(&name) {
        return Ok(FileEntry {
            abs: path.into(),
            rel,
            name,
            size,
            format: Format::Unknown,
            kind: MediaKind::Junk,
            ext_mismatch: false,
            is_raw: false,
            group: String::new(),
        });
    }
    let mut head = Vec::with_capacity(HEAD_LEN);
    File::open(path)?.take(HEAD_LEN as u64).read_to_end(&mut head)?;
    let format = sniff(&head, &ext);
    Ok(FileEntry {
        abs: path.into(),
        rel,
        name,
        size,
        format,
        kind: format.kind(),
        ext_mismatch: !ext_matches(format, &ext),
        is_raw: format.is_raw(&ext),
        group: String::new(),
    })
}

/// Group key = folder + stem. Within one key, allow at most one ordinary
/// image and one video (plus any RAWs and sidecars); extra images that only
/// happen to share a stem (photo.jpg + photo.png) get their own group.
fn assign_groups(files: &mut [FileEntry]) {
    let dir_of = |f: &FileEntry| f.rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
    let mut keys: Vec<Option<String>> = files
        .iter()
        .map(|f| (f.kind != MediaKind::Junk).then(|| format!("{}/{}", dir_of(f), group_stem(&f.name).to_lowercase())))
        .collect();
    // Takeout JSONs whose (cut-off) name matches no photo: pair by name prefix.
    let media_keys: std::collections::HashSet<&String> = files
        .iter()
        .zip(&keys)
        .filter(|(f, _)| matches!(f.kind, MediaKind::Image | MediaKind::Video))
        .filter_map(|(_, k)| k.as_ref())
        .collect();
    let mut fixes = vec![];
    for (i, f) in files.iter().enumerate() {
        if f.format != Format::Json || keys[i].as_ref().is_none_or(|k| media_keys.contains(k)) {
            continue;
        }
        let dir = dir_of(f);
        let media: Vec<(usize, String)> = files
            .iter()
            .enumerate()
            .filter(|(_, m)| matches!(m.kind, MediaKind::Image | MediaKind::Video) && dir_of(m) == dir)
            .map(|(j, m)| (j, m.name.to_lowercase()))
            .collect();
        if let Some(j) = truncated_takeout_partner(&f.name, &media) {
            fixes.push((i, keys[j].clone()));
        }
    }
    for (i, k) in fixes {
        keys[i] = k;
    }
    let mut by_key: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, k) in keys.into_iter().enumerate() {
        if let Some(k) = k {
            by_key.entry(k).or_default().push(i);
        }
    }
    for (key, idx) in by_key {
        let (mut img, mut vid) = (false, false);
        for i in idx {
            let f = &mut files[i];
            let slot = match (f.kind, f.is_raw) {
                (MediaKind::Image, false) => std::mem::replace(&mut img, true),
                (MediaKind::Video, _) => std::mem::replace(&mut vid, true),
                _ => false,
            };
            f.group = if slot { format!("{key}#{}", f.rel) } else { key.clone() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffing() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE1], "png"), Format::Jpeg);
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n....", "jpg"), Format::Png);
        assert_eq!(sniff(b"II*\0\x08\0\0\0", "nef"), Format::Tiff);
        assert_eq!(sniff(b"\0\0\0\x18ftypqt  \0\0\x02\0", "mov"), Format::Mov);
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0", "heic"), Format::Heif);
        assert_eq!(sniff(b"\0\0\0\x18ftypcrx \0\0\0\0", "cr3"), Format::Cr3);
        assert_eq!(sniff(b"  <x:xmpmeta", "xmp"), Format::Xmp);
        assert_eq!(sniff(b"{\"title\":1}", "json"), Format::Json);
        assert_eq!(sniff(b"{\"title\":1}", "jpg"), Format::Unknown);
        assert_eq!(sniff(b"", "jpg"), Format::Unknown);
    }

    #[test]
    fn stems() {
        assert_eq!(group_stem("IMG_1234.jpg.json"), "IMG_1234");
        assert_eq!(group_stem("IMG_1234.jpg.supplemental-metadata.json"), "IMG_1234");
        assert_eq!(group_stem("photo_x.xmp"), "photo_x");
        assert_eq!(group_stem("DSC_0700.NEF"), "DSC_0700");
        assert_eq!(group_stem("2004-05-06 party.jpg"), "2004-05-06 party");
        assert_eq!(group_stem("archive.v2.final"), "archive.v2.final");
        // Takeout: copy number behind the extension; cut-off "supplemental-metadata".
        assert_eq!(group_stem("IMG_1234.jpg(1).json"), "IMG_1234(1)");
        assert_eq!(group_stem("IMG_1234(1).jpg"), "IMG_1234(1)");
        assert_eq!(group_stem("IMG_1234.jpg.suppl.json"), "IMG_1234");
        assert_eq!(group_stem("IMG_1234.jpg.supplemental-metad.json"), "IMG_1234");
        assert_eq!(group_stem("notes.jp.json"), "notes.jp", "a non-media part is not dropped");
    }

    #[test]
    fn takeout_cut_off_json_names() {
        let media =
            vec![(0, "img_20190314_123456789_hdr_edited_long_name_x.jpg".to_string()), (1, "short.jpg".to_string())];
        assert_eq!(truncated_takeout_partner("IMG_20190314_123456789_HDR_edited_long_name.json", &media), Some(0));
        assert_eq!(truncated_takeout_partner("IMG_20190314_123456789_HDR_edited_long_name_x.j.json", &media), Some(0));
        assert_eq!(truncated_takeout_partner("short.json", &media), None, "short names are never guessed");
        let two = vec![
            (0, "img_20190314_123456789_hdr_edited_long_a.jpg".into()),
            (1, "img_20190314_123456789_hdr_edited_long_b.jpg".into()),
        ];
        assert_eq!(
            truncated_takeout_partner("IMG_20190314_123456789_HDR_edited_long_.json", &two),
            None,
            "ambiguous: no guess"
        );
    }

    #[test]
    fn junk() {
        assert!(is_junk("Thumbs.db") && is_junk(".DS_Store") && is_junk("._IMG_1.JPG") && is_junk("desktop.ini"));
        assert!(!is_junk("thumbs.jpg"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.jpg"), [0xFF, 0xD8, 0xFF]).unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("link")).unwrap();
        let r = scan(d.path()).unwrap();
        assert!(r.files.is_empty());
        assert_eq!(r.skipped_links.len(), 1);
    }
}
