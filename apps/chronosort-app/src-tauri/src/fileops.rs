//! Housekeeping in the File browser: new folder, rename, copy / move, delete
//! (Recycle Bin or permanent), and Undo.
//!
//! Safety rules:
//! * Nothing is ever overwritten. A name clash gets " (2)", " (3)" …
//! * Copies are verified byte for byte (BLAKE3) with dates kept (`cs_exec::copy_verified`).
//! * A move to another drive is a verified copy first; the original is removed
//!   only after every file of that item was verified.
//! * "Recycle Bin" is only used where the drive really has one
//!   ([`cs_platform::recycle_bin`]): on Windows, recycling on a drive whose bin is
//!   switched off deletes for good, so the UI offers only "Delete permanently" there.
//! * Drive roots, system folders and ChronoSort's own folders are refused.
//! * Everything except a permanent delete can be undone (newest first).

use crate::{AppState, Res, err};
use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;
use tauri::{Emitter, State};

static CANCEL: AtomicBool = AtomicBool::new(false);
static HISTORY: Mutex<Vec<Op>> = Mutex::new(Vec::new());
/// Only one housekeeping job at a time.
static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub struct Op {
    pub label: String,
    pub undo: Undo,
}

#[derive(Debug)]
pub enum Undo {
    NewFolder(PathBuf),
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    /// Files we created (path, size, modified) and folders we created; undo
    /// removes only files that are still unchanged, then the empty folders.
    Copied {
        files: Vec<(PathBuf, u64, Option<SystemTime>)>,
        dirs: Vec<PathBuf>,
    },
    /// Top-level items: original place -> new place.
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Sent to the Recycle Bin at this time.
    Trashed {
        items: Vec<PathBuf>,
        at: i64,
    },
    /// An edited picture took the original's place; the original waits in `backup`.
    Replaced {
        edited: PathBuf,
        original: PathBuf,
        backup: PathBuf,
    },
    /// Metadata written into `file`; `backup` is a verified copy from before,
    /// `after` the file's hash right after the change.
    MetaEdited {
        file: PathBuf,
        backup: PathBuf,
        after: String,
    },
    /// The same for many files at once (e.g. star ratings): (file, backup, after).
    MetaEditedMany(Vec<(PathBuf, PathBuf, String)>),
}

#[derive(Debug, Default, Serialize)]
pub struct OpReport {
    /// Top-level items done.
    pub done: usize,
    pub failed: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub stopped: bool,
    /// New paths (to select them afterwards).
    pub created: Vec<String>,
}

#[derive(Clone, Serialize)]
struct Progress {
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    current: String,
}

// ---------------------------------------------------------------- names

const BAD_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A file or folder name that works on Windows, macOS and Linux.
pub fn check_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Please type a name.".into());
    }
    if n == "." || n == ".." {
        return Err("That name is not allowed.".into());
    }
    if let Some(c) = n.chars().find(|c| BAD_CHARS.contains(c) || c.is_control()) {
        return Err(format!(
            "A name cannot contain {}",
            if c.is_control() { "control characters".to_string() } else { format!("the character  {c}") }
        ));
    }
    if n.ends_with('.') {
        return Err("A name cannot end with a full stop.".into());
    }
    let stem = n.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(format!("“{stem}” is a name Windows reserves for devices."));
    }
    if n.len() > 240 {
        return Err("That name is too long.".into());
    }
    Ok(n.to_string())
}

/// `dir/name`, or `dir/stem (2).ext` … if that name is taken.
pub fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let p = Path::new(name);
    let (stem, ext) = match (p.file_stem(), p.extension()) {
        (Some(s), Some(e)) if !name.starts_with('.') => {
            (s.to_string_lossy().into_owned(), format!(".{}", e.to_string_lossy()))
        }
        _ => (name.to_string(), String::new()),
    };
    for i in 2.. {
        let c = dir.join(format!("{stem} ({i}){ext}"));
        if fs::symlink_metadata(&c).is_err() {
            return c;
        }
    }
    unreachable!()
}

// ---------------------------------------------------------------- guards

fn is_within(p: &Path, dir: &Path) -> bool {
    let (Ok(a), Ok(b)) = (std::path::absolute(p), std::path::absolute(dir)) else { return false };
    if cfg!(windows) {
        let (a, b) = (a.to_string_lossy().to_lowercase(), b.to_string_lossy().to_lowercase());
        let b = b.trim_end_matches('\\');
        a == b || a.starts_with(&format!("{b}\\"))
    } else {
        a.starts_with(b)
    }
}

/// Places ChronoSort will never change.
fn protected_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(windows) {
        for k in ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData"] {
            if let Some(p) = std::env::var_os(k) {
                v.push(PathBuf::from(p));
            }
        }
    } else {
        for p in [
            "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/sbin", "/sys", "/usr", "/var", "/System",
            "/Library", "/private",
        ] {
            v.push(PathBuf::from(p));
        }
    }
    if let Some(d) = cs_ai::app_data_dir() {
        v.push(d);
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(d);
    }
    v
}

/// Refuse to change drive roots, system folders and ChronoSort's own folders.
pub fn guard(p: &Path) -> Result<(), String> {
    let abs = std::path::absolute(p).map_err(err)?;
    if abs.parent().is_none() || abs.file_name().is_none() {
        return Err(format!("“{}” is a whole drive; ChronoSort does not change drives as a whole.", abs.display()));
    }
    if let Some(d) = protected_dirs().into_iter().find(|d| is_within(&abs, d)) {
        return Err(format!(
            "“{}” is inside a protected folder ({}). ChronoSort does not change system or program folders.",
            abs.display(),
            d.display()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------- copy / move

fn is_cross_device(e: &io::Error) -> bool {
    // Windows ERROR_NOT_SAME_DEVICE = 17, Unix EXDEV = 18.
    e.kind() == io::ErrorKind::CrossesDevices
        || (cfg!(windows) && e.raw_os_error() == Some(17))
        || (cfg!(unix) && e.raw_os_error() == Some(18))
}

/// Every file below `p` (or `p` itself) with its size, for progress totals.
fn inventory(p: &Path, out: &mut Vec<(PathBuf, u64)>) -> io::Result<()> {
    let m = fs::symlink_metadata(p)?;
    if m.is_dir() {
        for e in fs::read_dir(p)? {
            inventory(&e?.path(), out)?;
        }
    } else {
        out.push((p.to_path_buf(), m.len()));
    }
    Ok(())
}

struct Job<'a> {
    cancel: &'a AtomicBool,
    report: &'a mut dyn FnMut(&Progress),
    prog: Progress,
}

impl Job<'_> {
    fn tick(&mut self, file: &Path, bytes: u64) {
        self.prog.files_done += 1;
        self.prog.bytes_done += bytes;
        self.prog.current = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (self.report)(&self.prog);
    }
}

/// Copy `src` (file or folder) to `dst` (must not exist), verifying every file.
/// Created files/folders are appended to `files` / `dirs` (also on failure, so they can be cleaned up).
fn copy_tree(
    src: &Path,
    dst: &Path,
    job: &mut Job,
    files: &mut Vec<(PathBuf, u64, Option<SystemTime>)>,
    dirs: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    if job.cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
    }
    let m = fs::symlink_metadata(src)?;
    if m.file_type().is_symlink() {
        return Err(io::Error::other(format!("“{}” is a link; links are not copied", src.display())));
    }
    if m.is_dir() {
        fs::create_dir(dst)?;
        dirs.push(dst.to_path_buf());
        let mut kids: Vec<PathBuf> = fs::read_dir(src)?.map(|e| e.map(|e| e.path())).collect::<io::Result<_>>()?;
        kids.sort();
        for k in kids {
            let name = k.file_name().ok_or_else(|| io::Error::other("bad name"))?;
            copy_tree(&k, &dst.join(name), job, files, dirs, warnings)?;
        }
        if let Ok(t) = cs_platform::read_times(src)
            && let Ok(f) = fs::File::open(dst)
        {
            let _ = cs_platform::apply_times(&f, &t); // folder dates: best effort
        }
        Ok(())
    } else {
        match cs_exec::copy_verified(src, dst)? {
            cs_exec::CopyResult::Copied { warning, .. } => {
                if let Some(w) = warning
                    && !warnings.contains(&w)
                {
                    warnings.push(w);
                }
            }
            cs_exec::CopyResult::AlreadyPresent => {}
        }
        let dm = fs::metadata(dst)?;
        files.push((dst.to_path_buf(), dm.len(), dm.modified().ok()));
        job.tick(src, m.len());
        Ok(())
    }
}

/// Remove what a failed copy created (only our own new files/folders).
fn clean_created(files: &[(PathBuf, u64, Option<SystemTime>)], dirs: &[PathBuf]) {
    for (f, ..) in files {
        let _ = fs::remove_file(f);
    }
    for d in dirs.iter().rev() {
        let _ = fs::remove_dir(d);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Copy,
    Move,
}

/// Copy or move items into `dest_dir`. `force_copy` (tests) skips the fast rename.
fn transfer(
    items: &[PathBuf],
    dest_dir: &Path,
    mode: Mode,
    force_copy: bool,
    job: &mut Job,
) -> (OpReport, Option<Undo>) {
    let mut rep = OpReport::default();
    let (mut files, mut dirs, mut moved) = (Vec::new(), Vec::new(), Vec::new());
    for src in items {
        if job.cancel.load(Ordering::Relaxed) {
            rep.stopped = true;
            break;
        }
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let fail = |rep: &mut OpReport, why: String| rep.failed.push((name.clone(), why));
        if let Err(e) = guard(src).and_then(|_| guard(&dest_dir.join("x"))) {
            fail(&mut rep, e);
            continue;
        }
        if is_within(dest_dir, src) {
            fail(&mut rep, "A folder cannot be put inside itself.".into());
            continue;
        }
        if mode == Mode::Move && src.parent().is_some_and(|p| is_within(p, dest_dir) && is_within(dest_dir, p)) {
            fail(&mut rep, "It is already in this folder.".into());
            continue;
        }
        let dst = free_name(dest_dir, &name);
        if mode == Mode::Move && !force_copy {
            match cs_platform::rename_no_replace(src, &dst) {
                Ok(()) => {
                    let mut inv = Vec::new();
                    let _ = inventory(&dst, &mut inv);
                    for (f, b) in inv {
                        job.tick(&f, b);
                    }
                    moved.push((src.clone(), dst.clone()));
                    rep.created.push(dst.to_string_lossy().into_owned());
                    rep.done += 1;
                    continue;
                }
                Err(e) if is_cross_device(&e) => {} // other drive: verified copy below
                Err(e) => {
                    fail(&mut rep, e.to_string());
                    continue;
                }
            }
        }
        let (mut f, mut d) = (Vec::new(), Vec::new());
        match copy_tree(src, &dst, job, &mut f, &mut d, &mut rep.warnings) {
            Ok(()) => {
                if mode == Mode::Move {
                    // Every file verified: now the original can go.
                    let removed = if src.is_dir() { fs::remove_dir_all(src) } else { fs::remove_file(src) };
                    if let Err(e) = removed {
                        rep.warnings.push(format!("“{name}” was copied but the original could not be removed: {e}"));
                    }
                    moved.push((src.clone(), dst.clone()));
                } else {
                    files.extend(f);
                    dirs.extend(d);
                }
                rep.created.push(dst.to_string_lossy().into_owned());
                rep.done += 1;
            }
            Err(e) => {
                clean_created(&f, &d);
                if e.kind() == io::ErrorKind::Interrupted {
                    rep.stopped = true;
                    break;
                }
                fail(&mut rep, e.to_string());
            }
        }
    }
    let undo = match mode {
        Mode::Copy if !files.is_empty() || !dirs.is_empty() => Some(Undo::Copied { files, dirs }),
        Mode::Move if !moved.is_empty() => Some(Undo::Moved(moved)),
        _ => None,
    };
    (rep, undo)
}

fn total_of(items: &[PathBuf]) -> (u64, u64) {
    let mut inv = Vec::new();
    for i in items {
        let _ = inventory(i, &mut inv);
    }
    (inv.len() as u64, inv.iter().map(|x| x.1).sum())
}

// ---------------------------------------------------------------- delete

fn remove_any(p: &Path) -> io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

/// Bring trashed items back (Windows and Linux; macOS has no API for it).
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn restore_trashed(items: &[PathBuf], at: i64) -> Result<usize, String> {
    use trash::os_limited;
    let want: Vec<PathBuf> = items.iter().filter_map(|p| std::path::absolute(p).ok()).collect();
    let found: Vec<trash::TrashItem> = os_limited::list()
        .map_err(err)?
        .into_iter()
        .filter(|t| t.time_deleted >= at - 5 && want.iter().any(|w| same(w, &t.original_path())))
        .collect();
    let n = found.len();
    if n == 0 {
        return Err("They are no longer in the Recycle Bin.".into());
    }
    os_limited::restore_all(found).map_err(|e| format!("Could not restore: {e}"))?;
    Ok(n)
}
#[cfg(target_os = "macos")]
fn restore_trashed(_items: &[PathBuf], _at: i64) -> Result<usize, String> {
    Err("On a Mac, open the Trash in Finder, select the files and choose “Put Back”.".into())
}

fn same(a: &Path, b: &Path) -> bool {
    if cfg!(windows) { a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) } else { a == b }
}

// ---------------------------------------------------------------- undo

fn undo_op(op: Undo) -> Result<String, String> {
    match op {
        Undo::NewFolder(p) => {
            fs::remove_dir(&p)
                .map_err(|_| "The new folder is no longer empty, so it was left in place.".to_string())?;
            Ok("The new folder was removed.".into())
        }
        Undo::Rename { from, to } => {
            cs_platform::rename_no_replace(&to, &from).map_err(|e| format!("Could not rename back: {e}"))?;
            Ok("The old name is back.".into())
        }
        Undo::Copied { files, dirs } => {
            let (mut removed, mut kept) = (0, 0);
            for (f, len, mt) in &files {
                match fs::metadata(f) {
                    Ok(m) if m.len() == *len && m.modified().ok() == *mt => {
                        if fs::remove_file(f).is_ok() {
                            removed += 1;
                        }
                    }
                    Ok(_) => kept += 1, // changed since: keep it
                    Err(_) => {}
                }
            }
            for d in dirs.iter().rev() {
                let _ = fs::remove_dir(d); // only if empty
            }
            Ok(if kept > 0 {
                format!("{removed} copies removed; {kept} were changed after copying and were kept.")
            } else {
                format!("{removed} copies removed.")
            })
        }
        Undo::Moved(pairs) => {
            let mut back = 0;
            let mut problems = Vec::new();
            let mut sink = |_: &Progress| {};
            let never = AtomicBool::new(false);
            let mut job = Job { cancel: &never, report: &mut sink, prog: empty_progress() };
            for (orig, now) in pairs.into_iter().rev() {
                if fs::symlink_metadata(&orig).is_ok() {
                    problems.push(format!("“{}” exists again at the old place", orig.display()));
                    continue;
                }
                let Some(parent) = orig.parent() else { continue };
                let _ = fs::create_dir_all(parent);
                match cs_platform::rename_no_replace(&now, &orig) {
                    Ok(()) => back += 1,
                    Err(e) if is_cross_device(&e) => {
                        let (mut f, mut d, mut w) = (Vec::new(), Vec::new(), Vec::new());
                        match copy_tree(&now, &orig, &mut job, &mut f, &mut d, &mut w) {
                            Ok(()) => {
                                let _ = remove_any(&now);
                                back += 1;
                            }
                            Err(e) => {
                                clean_created(&f, &d);
                                problems.push(e.to_string());
                            }
                        }
                    }
                    Err(e) => problems.push(e.to_string()),
                }
            }
            if problems.is_empty() {
                Ok(format!("{back} moved back."))
            } else {
                Err(format!("{back} moved back; problems: {}", problems.join("; ")))
            }
        }
        Undo::Trashed { items, at } => {
            restore_trashed(&items, at).map(|n| format!("{n} restored from the Recycle Bin."))
        }
        Undo::Replaced { edited, original, backup } => {
            if !backup.exists() {
                return Err("The original is no longer in ChronoSort's backup folder.".into());
            }
            // The edited file steps aside (kept next to the backup), the original comes back.
            if edited.exists() {
                let aside = free_name(
                    backup.parent().unwrap_or(Path::new(".")),
                    &format!("edited-{}", edited.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
                );
                cs_platform::rename_no_replace(&edited, &aside)
                    .map_err(|e| format!("Could not move the edited picture aside: {e}"))?;
            }
            cs_platform::rename_no_replace(&backup, &original)
                .map_err(|e| format!("Could not put the original back: {e}"))?;
            Ok("The original picture is back.".into())
        }
        Undo::MetaEdited { file, backup, after } => {
            restore_meta(&file, &backup, &after)?;
            Ok("The old information is back.".into())
        }
        Undo::MetaEditedMany(list) => {
            let mut back = 0;
            let mut problems = Vec::new();
            for (file, backup, after) in &list {
                match restore_meta(file, backup, after) {
                    Ok(()) => back += 1,
                    Err(e) => problems
                        .push(format!("{}: {e}", file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())),
                }
            }
            if problems.is_empty() {
                Ok(format!("The old information is back in {back} {}.", if back == 1 { "file" } else { "files" }))
            } else {
                Err(format!("{back} put back; not put back: {}", problems.join("; ")))
            }
        }
    }
}

/// Put a file back from its verified backup, only if nobody changed it since `after`.
fn restore_meta(file: &Path, backup: &Path, after: &str) -> Result<(), String> {
    let now = cs_core::hash_file(file).map_err(err)?;
    if now != after {
        return Err("The file was changed again since, so it was left as it is.".into());
    }
    let tmp = free_name(
        file.parent().unwrap_or(Path::new(".")),
        &format!(".{}.chronosort-undo", file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
    );
    fs::copy(backup, &tmp).map_err(err)?;
    if cs_core::hash_file(&tmp).map_err(err)? != cs_core::hash_file(backup).map_err(err)? {
        let _ = fs::remove_file(&tmp);
        return Err("The backup could not be copied back correctly.".into());
    }
    fs::remove_file(file).map_err(err)?;
    fs::rename(&tmp, file).map_err(err)?;
    Ok(())
}

fn empty_progress() -> Progress {
    Progress { files_done: 0, files_total: 0, bytes_done: 0, bytes_total: 0, current: String::new() }
}

pub fn push(op: Op) {
    let mut h = HISTORY.lock().unwrap_or_else(|e| e.into_inner());
    h.push(op);
    if h.len() > 100 {
        h.remove(0);
    }
}

struct BusyGuard;
impl BusyGuard {
    fn take() -> Res<Self> {
        if BUSY.swap(true, Ordering::SeqCst) {
            return Err("Another file job is still running.".into());
        }
        CANCEL.store(false, Ordering::SeqCst);
        Ok(Self)
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------- commands

fn paths_of(state: &AppState, ids: &[u64]) -> Res<Vec<PathBuf>> {
    ids.iter()
        .map(|&id| {
            state.browse.path(id).ok_or_else(|| "Some items are no longer listed; press F5 and try again.".to_string())
        })
        .collect()
}

/// Recycle Bin status for a folder (the UI asks before showing the delete choices).
#[tauri::command]
pub fn fs_bin_status(path: String) -> cs_platform::BinStatus {
    cs_platform::recycle_bin(Path::new(&path))
}

#[tauri::command]
pub fn fs_check_name(name: String) -> Res<String> {
    check_name(&name)
}

#[tauri::command]
pub fn fs_new_folder(dir: String, name: String) -> Res<String> {
    let name = check_name(&name)?;
    let p = Path::new(&dir).join(&name);
    guard(&p)?;
    fs::create_dir(&p).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            "Something with that name is already here.".into()
        } else {
            e.to_string()
        }
    })?;
    push(Op { label: format!("New folder “{name}”"), undo: Undo::NewFolder(p.clone()) });
    Ok(p.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn fs_rename(state: State<'_, AppState>, id: u64, name: String) -> Res<String> {
    let from = state.browse.path(id).ok_or("This item is no longer listed; press F5.")?;
    let name = check_name(&name)?;
    guard(&from)?;
    let to = from.with_file_name(&name);
    if to == from {
        return Ok(to.to_string_lossy().into_owned());
    }
    // A case-only change ("img.jpg" -> "IMG.jpg") is the same file on Windows/macOS.
    let case_only = to.to_string_lossy().to_lowercase() == from.to_string_lossy().to_lowercase();
    if case_only {
        fs::rename(&from, &to).map_err(err)?;
    } else {
        cs_platform::rename_no_replace(&from, &to).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                "Something with that name is already here.".into()
            } else {
                e.to_string()
            }
        })?;
    }
    let old = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    push(Op { label: format!("Rename “{old}” to “{name}”"), undo: Undo::Rename { from, to: to.clone() } });
    Ok(to.to_string_lossy().into_owned())
}

/// Copy or move listed items into a folder, with progress (`fileop-progress`).
#[tauri::command]
pub async fn fs_transfer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<u64>,
    dest: String,
    mode: String,
) -> Res<OpReport> {
    let items = paths_of(&state, &ids)?;
    let mode = if mode == "move" { Mode::Move } else { Mode::Copy };
    let dest = PathBuf::from(dest);
    if !dest.is_dir() {
        return Err("The target folder does not exist.".into());
    }
    let _busy = BusyGuard::take()?;
    tauri::async_runtime::spawn_blocking(move || {
        let (files_total, bytes_total) = total_of(&items);
        let mut emit = |p: &Progress| {
            let _ = app.emit("fileop-progress", p.clone());
        };
        let mut job =
            Job { cancel: &CANCEL, report: &mut emit, prog: Progress { files_total, bytes_total, ..empty_progress() } };
        let (rep, undo) = transfer(&items, &dest, mode, false, &mut job);
        if let Some(u) = undo {
            let what = if mode == Mode::Move { "Move" } else { "Copy" };
            push(Op {
                label: format!(
                    "{what} {} item(s) to “{}”",
                    rep.done,
                    dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                ),
                undo: u,
            });
        }
        Ok(rep)
    })
    .await
    .map_err(err)?
}

/// Delete listed items: `permanent` false = Recycle Bin (refused where there is none).
#[tauri::command]
pub async fn fs_delete(state: State<'_, AppState>, ids: Vec<u64>, permanent: bool) -> Res<OpReport> {
    let items = paths_of(&state, &ids)?;
    for p in &items {
        guard(p)?;
    }
    if !permanent && let Some(p) = items.first() {
        let b = cs_platform::recycle_bin(p);
        if !b.available {
            return Err(format!("{} Use “Delete permanently” instead.", b.reason.unwrap_or_default()));
        }
    }
    let _busy = BusyGuard::take()?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut rep = OpReport::default();
        if permanent {
            for p in &items {
                match remove_any(p) {
                    Ok(()) => rep.done += 1,
                    Err(e) => rep.failed.push((
                        p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        e.to_string(),
                    )),
                }
            }
        } else {
            let at = crate::now_unix();
            match trash::delete_all(&items) {
                Ok(()) => {
                    rep.done = items.len();
                    push(Op {
                        label: format!("Delete {} item(s) to the Recycle Bin", items.len()),
                        undo: Undo::Trashed { items, at },
                    });
                }
                Err(e) => return Err(format!("Could not move to the Recycle Bin: {e}")),
            }
        }
        Ok(rep)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn fs_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// What Ctrl+Z would undo next (None = nothing).
#[tauri::command]
pub fn fs_last_undo() -> Option<String> {
    HISTORY.lock().unwrap_or_else(|e| e.into_inner()).last().map(|o| o.label.clone())
}

#[tauri::command]
pub async fn fs_undo() -> Res<String> {
    let op = HISTORY.lock().unwrap_or_else(|e| e.into_inner()).pop().ok_or("Nothing to undo.")?;
    let _busy = BusyGuard::take()?;
    let label = op.label.clone();
    tauri::async_runtime::spawn_blocking(move || undo_op(op.undo).map(|m| format!("Undone: {label}. {m}")))
        .await
        .map_err(err)?
}

#[cfg(test)]
pub fn take_last_for_test() -> Undo {
    HISTORY.lock().unwrap().pop().unwrap().undo
}
#[cfg(test)]
pub fn undo_for_test(u: Undo) -> Result<String, String> {
    undo_op(u)
}
/// Tests that use the shared undo history take this first (tests run in parallel).
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn job_run(items: &[PathBuf], dest: &Path, mode: Mode, force_copy: bool) -> (OpReport, Option<Undo>) {
        let mut sink = |_: &Progress| {};
        let cancel = AtomicBool::new(false);
        let mut job = Job { cancel: &cancel, report: &mut sink, prog: empty_progress() };
        transfer(items, dest, mode, force_copy, &mut job)
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(check_name("  Holiday 2004 ").unwrap(), "Holiday 2004");
        for bad in ["", "a/b", "a\\b", "x:y", "what?", "con", "CON.txt", "lpt1.jpg", "dots.", ".."] {
            assert!(check_name(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn clash_names_count_up() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.jpg"), b"1").unwrap();
        fs::write(d.path().join("a (2).jpg"), b"2").unwrap();
        assert_eq!(free_name(d.path(), "a.jpg"), d.path().join("a (3).jpg"));
        assert_eq!(free_name(d.path(), "b.jpg"), d.path().join("b.jpg"));
        fs::create_dir(d.path().join("Folder")).unwrap();
        assert_eq!(free_name(d.path(), "Folder"), d.path().join("Folder (2)"));
    }

    #[test]
    fn roots_are_refused() {
        let root = if cfg!(windows) { PathBuf::from("C:\\") } else { PathBuf::from("/") };
        assert!(guard(&root).is_err());
        if cfg!(windows) {
            let win = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("notepad.exe");
            assert!(guard(&win).is_err());
        }
        let d = tempfile::tempdir().unwrap();
        assert!(guard(&d.path().join("x.jpg")).is_ok());
    }

    fn tree(base: &Path) -> PathBuf {
        let f = base.join("Trip");
        fs::create_dir_all(f.join("day2")).unwrap();
        fs::write(f.join("a.jpg"), b"aaaa").unwrap();
        fs::write(f.join("day2").join("b.jpg"), b"bbbbbbbb").unwrap();
        f
    }

    #[test]
    fn copy_folder_never_overwrites_and_undo_removes_only_copies() {
        let d = tempfile::tempdir().unwrap();
        let src = tree(d.path());
        let dest = d.path().join("dest");
        fs::create_dir(&dest).unwrap();
        fs::create_dir(dest.join("Trip")).unwrap(); // clash
        let (rep, undo) = job_run(std::slice::from_ref(&src), &dest, Mode::Copy, false);
        assert_eq!(rep.done, 1, "{:?}", rep.failed);
        let copy = dest.join("Trip (2)");
        assert_eq!(fs::read(copy.join("day2/b.jpg")).unwrap(), b"bbbbbbbb");
        assert!(src.join("a.jpg").exists(), "original stays");
        // The user changes one copy afterwards: undo must keep it.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(copy.join("a.jpg"), b"edited").unwrap();
        let msg = undo_op(undo.unwrap()).unwrap();
        assert!(msg.contains("kept"), "{msg}");
        assert!(copy.join("a.jpg").exists());
        assert!(!copy.join("day2").exists());
        assert!(dest.join("Trip").exists(), "the clashing folder is untouched");
    }

    #[test]
    fn move_same_drive_and_undo() {
        let d = tempfile::tempdir().unwrap();
        let src = tree(d.path());
        let dest = d.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let (rep, undo) = job_run(std::slice::from_ref(&src), &dest, Mode::Move, false);
        assert_eq!(rep.done, 1);
        assert!(!src.exists() && dest.join("Trip/day2/b.jpg").exists());
        undo_op(undo.unwrap()).unwrap();
        assert!(src.join("day2/b.jpg").exists() && !dest.join("Trip").exists());
    }

    #[test]
    fn move_by_verified_copy_and_undo() {
        // Same path as a move to another drive: copy + verify, then remove the original.
        let d = tempfile::tempdir().unwrap();
        let src = tree(d.path());
        let file = d.path().join("single.png");
        fs::write(&file, b"png!").unwrap();
        let dest = d.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let (rep, undo) = job_run(&[src.clone(), file.clone()], &dest, Mode::Move, true);
        assert_eq!(rep.done, 2, "{:?}", rep.failed);
        assert!(!src.exists() && !file.exists());
        assert_eq!(fs::read(dest.join("Trip/a.jpg")).unwrap(), b"aaaa");
        undo_op(undo.unwrap()).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"png!");
        assert!(src.join("day2/b.jpg").exists());
    }

    #[test]
    fn folder_into_itself_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let src = tree(d.path());
        let (rep, undo) = job_run(std::slice::from_ref(&src), &src.join("day2"), Mode::Move, false);
        assert_eq!(rep.done, 0);
        assert!(rep.failed[0].1.contains("inside itself"));
        assert!(undo.is_none());
        let (rep, _) = job_run(std::slice::from_ref(&src), d.path(), Mode::Move, false);
        assert!(rep.failed[0].1.contains("already in this folder"));
    }

    #[test]
    fn copy_into_same_folder_makes_numbered_copy() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.jpg");
        fs::write(&f, b"x").unwrap();
        let (rep, _) = job_run(std::slice::from_ref(&f), d.path(), Mode::Copy, false);
        assert_eq!(rep.done, 1);
        assert!(d.path().join("x (2).jpg").exists());
    }

    #[test]
    fn new_folder_and_rename_undo() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let p = fs_new_folder(d.path().to_string_lossy().into(), "New".into()).unwrap();
        assert!(Path::new(&p).is_dir());
        assert!(fs_new_folder(d.path().to_string_lossy().into(), "New".into()).is_err(), "no overwrite");
        let op = HISTORY.lock().unwrap().pop().unwrap();
        undo_op(op.undo).unwrap();
        assert!(!Path::new(&p).exists());

        let a = d.path().join("a.jpg");
        fs::write(&a, b"1").unwrap();
        let b = d.path().join("b.jpg");
        cs_platform::rename_no_replace(&a, &b).unwrap();
        undo_op(Undo::Rename { from: a.clone(), to: b.clone() }).unwrap();
        assert!(a.exists() && !b.exists());
    }

    #[test]
    fn permanent_delete_removes_tree() {
        let d = tempfile::tempdir().unwrap();
        let src = tree(d.path());
        remove_any(&src).unwrap();
        assert!(!src.exists());
    }
}
