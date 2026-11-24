//! "Add a place": GPS + place names for many photos at once, offline.
//! The work is done by cs-geo (ExifTool, verified backups, undo).

use crate::{AppState, Progress, Res, err};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, State};

/// The place list, loaded once from ExifTool's built-in database (~3 s).
static PLACES: OnceLock<cs_geo::Places> = OnceLock::new();
static CANCEL: AtomicBool = AtomicBool::new(false);

/// ExifTool comes with ChronoSort: `exiftool/exiftool.exe` next to the program.
pub fn exiftool() -> Res<PathBuf> {
    // Tests run from target/…/deps: use the project's own copy.
    #[cfg(test)]
    {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../tools/exiftool").join(if cfg!(windows) {
            "exiftool.exe"
        } else {
            "exiftool"
        });
        if p.is_file() {
            return Ok(p);
        }
    }
    let exe = std::env::current_exe().map_err(err)?;
    let dir = exe.parent().ok_or("no program folder")?;
    let name = if cfg!(windows) { "exiftool.exe" } else { "exiftool" };
    let p = dir.join("exiftool").join(name);
    if p.is_file() {
        Ok(p)
    } else {
        Err("ExifTool is missing from ChronoSort's folder (exiftool\\exiftool.exe). Please reinstall or copy ChronoSort again.".into())
    }
}

fn places() -> Res<&'static cs_geo::Places> {
    if let Some(p) = PLACES.get() {
        return Ok(p);
    }
    let mut cmd = std::process::Command::new(exiftool()?);
    cmd.args(["-api", "geolocation", "-listgeo"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(err)?;
    let list = cs_geo::parse_listgeo(&String::from_utf8_lossy(&out.stdout));
    if list.is_empty() {
        return Err("The place list could not be read from ExifTool.".into());
    }
    Ok(PLACES.get_or_init(|| cs_geo::Places::new(list)))
}

#[derive(Serialize)]
pub struct FoundFile {
    rel: String,
    kind: cs_model::MediaKind,
    /// thumb:// key
    key: String,
    has_place: bool,
}

#[derive(Serialize)]
pub struct FolderOut {
    folder: String,
    files: Vec<FoundFile>,
    runs: Vec<cs_geo::Manifest>,
}

/// Look at a folder: its photos and videos, and which already have a location.
#[tauri::command]
pub async fn places_scan(app: tauri::AppHandle, state: State<'_, AppState>, folder: String) -> Res<FolderOut> {
    let root = PathBuf::from(folder.trim());
    if !root.is_dir() {
        return Err("That folder cannot be found.".into());
    }
    let app2 = app.clone();
    let root2 = root.clone();
    let (files, keys) = tauri::async_runtime::spawn_blocking(move || -> Res<_> {
        let scan = cs_scan::scan(&root2).map_err(err)?;
        let media: Vec<_> = scan
            .files
            .into_iter()
            .filter(|f| matches!(f.kind, cs_model::MediaKind::Image | cs_model::MediaKind::Video))
            .collect();
        let total = media.len();
        let _ = app2.emit("places-progress", Progress { done: 0, total, bytes: 0 });
        // One ExifTool pass over the folder: which files already have GPS or a place name.
        let mut et = cs_geo::exiftool::ExifTool::start(&exiftool()?).map_err(err)?;
        let lines = et
            .run(&[
                "-charset".into(),
                "filename=utf8".into(),
                "-j".into(),
                "-n".into(),
                "-r".into(),
                "-i".into(),
                ".chronosort".into(),
                "-GPSLatitude".into(),
                "-GPSCoordinates".into(),
                "-XMP-photoshop:Country".into(),
                root2.display().to_string(),
            ])
            .map_err(err)?;
        // The JSON array follows the summary lines ("3 directories scanned"):
        // from the first line starting with "[" to the last "]" (it ends "}]").
        let json: String = match lines.iter().position(|l| l.starts_with('[')) {
            Some(a) => {
                let text = lines[a..].join("\n");
                text.rfind(']').map_or_else(|| "[]".to_string(), |e| text[..=e].to_string())
            }
            None => "[]".into(),
        };
        let tagged: std::collections::HashSet<String> = serde_json::from_str::<Vec<serde_json::Value>>(&json)
            .unwrap_or_default()
            .into_iter()
            .filter(|v| {
                v.get("GPSLatitude").is_some() || v.get("GPSCoordinates").is_some() || v.get("Country").is_some()
            })
            .filter_map(|v| v["SourceFile"].as_str().map(same_path))
            .collect();
        let mut files = vec![];
        let mut keys = vec![];
        for f in media {
            let key = blake3::hash(f.abs.to_string_lossy().as_bytes()).to_hex().to_string();
            let abs = same_path(&f.abs.to_string_lossy());
            keys.push((key.clone(), f.abs.clone(), f.format));
            files.push(FoundFile { rel: f.rel.clone(), kind: f.kind, key, has_place: tagged.contains(&abs) });
        }
        let _ = app2.emit("places-progress", Progress { done: total, total, bytes: 0 });
        Ok((files, keys))
    })
    .await
    .map_err(err)??;
    // Thumbnails for these files (keyed by path; the picture itself does not change when a place is added).
    {
        let mut s = state.session.lock().map_err(err)?;
        for (k, abs, fmt) in keys {
            s.by_hash.insert(k, (abs, fmt));
        }
    }
    Ok(FolderOut { folder: root.display().to_string(), files, runs: cs_geo::runs(&root) })
}

/// A path in one comparable form: forward slashes, no doubled slashes
/// (a folder typed as `D:\Photos\` gives ExifTool paths like `D:/Photos//2009/a.jpg`), lower case.
fn same_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars().map(|c| if c == '\\' { '/' } else { c }) {
        if !(c == '/' && out.ends_with('/')) {
            out.push(c);
        }
    }
    out.to_lowercase()
}

#[cfg(test)]
mod tests {
    #[test]
    fn same_path_ignores_slash_style() {
        assert_eq!(super::same_path(r"D:\Photos\/2009/A.jpg"), super::same_path("d:/photos/2009/a.JPG"));
        assert_ne!(super::same_path("d:/photos/a.jpg"), super::same_path("d:/photos/b.jpg"));
    }
}

#[derive(Serialize)]
pub struct SearchOut {
    places: Vec<cs_geo::Place>,
    regions: Vec<cs_geo::Region>,
    countries: Vec<cs_geo::Country>,
}

#[tauri::command]
pub async fn places_search(query: String) -> Res<SearchOut> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = places()?;
        Ok(SearchOut {
            places: p.search(&query, 12).into_iter().cloned().collect(),
            regions: p.regions(&query, 5),
            countries: p.countries(&query, 6),
        })
    })
    .await
    .map_err(err)?
}

/// Add the place to these files (relative paths in `folder`).
#[tauri::command]
pub async fn places_apply(
    app: tauri::AppHandle,
    folder: String,
    rels: Vec<String>,
    target: cs_geo::Target,
) -> Res<cs_geo::Report> {
    let root = PathBuf::from(folder);
    CANCEL.store(false, Ordering::Relaxed);
    tauri::async_runtime::spawn_blocking(move || {
        let jobs: Vec<cs_geo::Job> = rels
            .into_iter()
            .filter(|r| !r.split('/').any(|c| c == ".." || c.is_empty()) && !Path::new(r).is_absolute())
            .map(|rel| {
                let video = matches!(
                    rel.rsplit('.').next().map(str::to_ascii_lowercase).as_deref(),
                    Some("mov" | "mp4" | "m4v" | "3gp")
                );
                cs_geo::Job { rel, video }
            })
            .collect();
        let run = {
            let (y, mo, d, h, mi, s) = cs_model::civil::from_unix(crate::now_unix());
            format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
        };
        let emit = |done, total| {
            let _ = app.emit("places-progress", Progress { done, total, bytes: 0 });
        };
        cs_geo::apply(&exiftool()?, &root, &jobs, &target, &run, &emit, &CANCEL).map_err(err)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn places_cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub async fn places_undo(folder: String, run: String) -> Res<cs_geo::UndoReport> {
    if run.contains(['/', '\\', '.']) {
        return Err("unknown run".into());
    }
    tauri::async_runtime::spawn_blocking(move || cs_geo::undo(Path::new(&folder), &run).map_err(err))
        .await
        .map_err(err)?
}
