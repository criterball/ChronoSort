//! Portable mode: run ChronoSort from a USB stick on different PCs.
//!
//! A `portable.txt` next to ChronoSort.exe makes it keep everything in
//! `ChronoSortData` beside the program (see `data_dir`). `make_portable` puts
//! a ready-to-use copy on a stick: the program files, the marker, and, if
//! wanted, the AI models / engine and the people the user has named.

use crate::{Res, data_dir, err};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::Emitter;

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Files that make up the program (copied when present next to ChronoSort.exe).
const PROGRAM_FILES: &[&str] = &["ChronoSort.exe", "WebView2Loader.dll", "chronosort-cli.exe", "README.txt", "LICENSE"];

const MARKER_TEXT: &str = "This file switches on ChronoSort's portable mode.\r\n\
Everything ChronoSort keeps (AI models, the people you named, settings) is stored in the\r\n\
ChronoSortData folder next to ChronoSort.exe, so it travels with this folder.\r\n\
Delete this file to make ChronoSort use the PC's own folders again.\r\n";

#[derive(Serialize)]
pub struct PortableInfo {
    /// Running in portable mode right now.
    portable: bool,
    data_dir: String,
    /// Sizes of what can be taken along.
    models_bytes: u64,
    people: usize,
}

fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

#[tauri::command]
pub async fn portable_info(app: tauri::AppHandle) -> Res<PortableInfo> {
    tauri::async_runtime::spawn_blocking(move || {
        let d = data_dir(&app)?;
        let people = cs_db::Db::open(&d.join("people.db")).ok().and_then(|db| db.people().ok()).map_or(0, |p| p.len());
        Ok(PortableInfo {
            portable: cs_ai::portable_root().is_some(),
            data_dir: d.display().to_string(),
            models_bytes: dir_size(&d.join("models")) + dir_size(&d.join("tools")),
            people,
        })
    })
    .await
    .map_err(err)?
}

#[derive(Serialize, Clone)]
struct CopyProgress {
    done: u64,
    total: u64,
    file: String,
}

#[derive(Serialize)]
pub struct PortableReport {
    folder: String,
    files: usize,
    bytes: u64,
    notes: Vec<String>,
}

/// Files to copy: (source, destination relative to the new ChronoSort folder).
fn plan(src_data: &Path, models: bool) -> Res<Vec<(PathBuf, PathBuf)>> {
    let exe_dir = std::env::current_exe().map_err(err)?.parent().ok_or("no program folder")?.to_path_buf();
    let mut out: Vec<(PathBuf, PathBuf)> =
        PROGRAM_FILES.iter().map(|n| (exe_dir.join(n), PathBuf::from(n))).filter(|(s, _)| s.is_file()).collect();
    // ExifTool (for "Add a place") lives in its own folder next to the program.
    let mut stack = vec![exe_dir.join("exiftool")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() => {
                    let rel = p.strip_prefix(&exe_dir).map_err(err)?.to_path_buf();
                    out.push((p, rel));
                }
                _ => {}
            }
        }
    }
    if models {
        for sub in ["models", "tools"] {
            let mut stack = vec![src_data.join(sub)];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    match e.file_type() {
                        Ok(t) if t.is_dir() => stack.push(p),
                        // Unfinished downloads stay behind.
                        Ok(t) if t.is_file() && p.extension().is_none_or(|x| x != "part") => {
                            let rel = p.strip_prefix(src_data).map_err(err)?;
                            out.push((p.clone(), Path::new(cs_ai::PORTABLE_DIR).join(rel)));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Copy one file via `<name>.part`, reporting progress; an identical-size file
/// already there (an earlier copy to the same stick) is skipped.
fn copy_file(src: &Path, dst: &Path, done: &mut u64, report: &dyn Fn(u64, &str)) -> std::io::Result<bool> {
    let len = std::fs::metadata(src)?.len();
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Program files are always replaced (an update); data files of the same size are skipped.
    let is_program = dst.components().any(|c| c.as_os_str() == "exiftool")
        || dst.extension().is_some_and(|x| x == "exe" || x == "dll");
    if !is_program && std::fs::metadata(dst).is_ok_and(|m| m.len() == len) {
        *done += len;
        report(*done, &name);
        return Ok(false);
    }
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    let part = dst.with_file_name(format!("{name}.part"));
    let mut r = std::fs::File::open(src)?;
    let mut w = std::fs::File::create(&part)?;
    let mut buf = vec![0u8; 4 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        *done += n as u64;
        report(*done, &name);
    }
    w.sync_all()?;
    drop(w);
    if std::fs::metadata(&part)?.len() != len {
        let _ = std::fs::remove_file(&part);
        return Err(std::io::Error::other(format!("{name}: the copy is incomplete")));
    }
    // Keep the original's time so the model check ("unchanged since linked") still holds.
    if let Ok(t) = std::fs::metadata(src).and_then(|m| m.modified()) {
        let _ = std::fs::File::options().write(true).open(&part).and_then(|f| f.set_modified(t));
    }
    std::fs::rename(&part, dst)?;
    Ok(true)
}

/// Put a portable ChronoSort into `<target>/ChronoSort`.
#[tauri::command]
pub async fn make_portable(app: tauri::AppHandle, target: String, models: bool, people: bool) -> Res<PortableReport> {
    let target = PathBuf::from(target.trim());
    if !target.is_dir() {
        return Err("That folder cannot be found.".into());
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Res<PortableReport> {
        let app = app2;
        let src_data = data_dir(&app)?;
        let home = target.join("ChronoSort");
        let new_data = home.join(cs_ai::PORTABLE_DIR);
        let exe_dir = std::env::current_exe().map_err(err)?.parent().map(Path::to_path_buf).unwrap_or_default();
        if home.starts_with(&src_data) {
            return Err("Please choose a folder outside ChronoSort's own data folder.".into());
        }
        if same_dir(&home, &exe_dir) {
            return Err("ChronoSort is already running from that folder. Choose another drive or folder.".into());
        }
        let files = plan(&src_data, models)?;
        let total: u64 = files.iter().map(|(s, _)| std::fs::metadata(s).map_or(0, |m| m.len())).sum();
        if let Ok(free) = cs_platform::free_space(&target)
            && free < total + (64 << 20)
        {
            return Err(format!(
                "Not enough space there: {:.1} GB needed, {:.1} GB free. Try without the AI models, or a bigger stick.",
                (total + (64 << 20)) as f64 / 1e9,
                free as f64 / 1e9
            ));
        }
        let mut notes = vec![];
        let mut done = 0u64;
        let mut copied = 0;
        let emit = |done: u64, file: &str| {
            let _ = app.emit("portable-progress", CopyProgress { done, total, file: file.to_string() });
        };
        for (src, rel) in &files {
            copied += usize::from(copy_file(src, &home.join(rel), &mut done, &emit).map_err(err)?);
        }
        std::fs::create_dir_all(&new_data).map_err(err)?;
        std::fs::write(home.join(cs_ai::PORTABLE_MARKER), MARKER_TEXT).map_err(err)?;

        // The model list, pointing at the copies (stored relative, so drive letters do not matter).
        if models {
            let linked = cs_ai::Linked::load(&src_data.join("models.json")).map_err(err)?;
            let mut out = cs_ai::Linked::default();
            let mut skipped_own = false;
            for (pack, roles) in &linked.packs {
                for (role, f) in roles {
                    match f.path.strip_prefix(&src_data) {
                        Ok(rel) if new_data.join(rel).is_file() => {
                            let mut g = f.clone();
                            g.path = new_data.join(rel);
                            out.packs.entry(pack.clone()).or_default().insert(role.clone(), g);
                        }
                        _ => skipped_own = true,
                    }
                }
            }
            out.save(&new_data.join("models.json")).map_err(err)?;
            if skipped_own {
                notes.push("Model files you linked from other folders were not copied; link them again on the stick if needed.".into());
            }
        }

        // People: a consistent copy of the database (never over a list already on the stick).
        if people {
            let src_db = src_data.join("people.db");
            let dst_db = new_data.join("people.db");
            if dst_db.exists() {
                notes.push("The stick already has its own list of people; it was kept as it is.".into());
            } else if src_db.exists() {
                let db = cs_db::Db::open(&src_db).map_err(err)?;
                db.conn().execute("VACUUM INTO ?1", [dst_db.to_string_lossy().as_ref()]).map_err(err)?;
                notes.push("The people you named are on the stick. Keep it safe: it holds their names and face fingerprints.".into());
            }
        }
        Ok(PortableReport { folder: home.display().to_string(), files: copied, bytes: total, notes })
    })
    .await
    .map_err(err)?
}
