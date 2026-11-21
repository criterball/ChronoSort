//! `cs-exec`: carry out a plan safely, and undo it.
//!
//! For every file:
//! 1. journal `pending` (before any byte is written)
//! 2. stream source → `.<name>.chronosort-partial`, hashing while reading
//! 3. fsync, restore modified/accessed/created times
//! 4. re-read the copy and compare hashes; check the source did not change
//! 5. rename into place, **never replacing** an existing file
//! 6. journal `done` (+ hash); move mode: source → OS trash, journal `source_trashed`
//!
//! A crash at any point leaves either nothing, a `*.chronosort-partial` file
//! (removed by [`recover`]), or a complete verified copy.

use cs_db::{Db, JournalState};
use cs_model::TransferMode;
use cs_plan::{Plan, STATE_DIR};
use cs_platform::{CAN_SET_CREATED, Times, apply_times, read_times, rename_no_replace};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

mod dups;
pub use dups::{DupAction, DupMode, DupReport, resolve_duplicates};

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Db(#[from] cs_db::DbError),
    #[error("plan is not ready: {0}")]
    NotReady(String),
    #[error("not enough free space: need {need} bytes, {free} available")]
    NoSpace { need: u64, free: u64 },
    #[error("unsafe folders: {0}")]
    UnsafePaths(String),
}

pub type Result<T> = std::result::Result<T, ExecError>;

/// Called after each chunk with (files done, total files, bytes copied).
pub type ProgressFn = std::sync::Arc<dyn Fn(usize, usize, u64) + Send + Sync>;

#[derive(Clone)]
pub struct ExecOptions {
    pub mode: TransferMode,
    /// Stop after this many files (used by tests to simulate an interruption).
    pub stop_after: Option<usize>,
    /// Parallel copies of small files (overlaps per-file fsync latency).
    pub threads: usize,
    /// How a moved source is disposed of. Default: the OS trash / Recycle Bin.
    /// (Replaceable so tests never touch the real Recycle Bin.)
    pub trash: fn(&Path) -> io::Result<()>,
    pub progress: Option<ProgressFn>,
}

impl std::fmt::Debug for ExecOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecOptions").field("mode", &self.mode).field("threads", &self.threads).finish_non_exhaustive()
    }
}

/// Send a file to the operating system's trash (never a hard delete).
pub fn os_trash(p: &Path) -> io::Result<()> {
    trash::delete(p).map_err(io::Error::other)
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self { mode: TransferMode::Copy, stop_after: None, threads: 4, trash: os_trash, progress: None }
    }
}

/// Files per journal transaction.
const CHUNK_FILES: usize = 64;
/// Files at least this large are copied one at a time (kind to hard drives).
const LARGE_FILE: u64 = 16 << 20;

#[derive(Debug, Default, Serialize)]
pub struct ExecReport {
    pub run_id: i64,
    pub copied: usize,
    pub bytes: u64,
    pub already_present: usize,
    pub trashed: usize,
    pub failed: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub stopped_early: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct RecoverReport {
    pub completed: usize,
    pub failed: usize,
    pub partials_removed: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct UndoReport {
    pub run_id: i64,
    pub removed: usize,
    pub restored: usize,
    /// Copies changed since they were made: left alone.
    pub kept_modified: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub dirs_removed: usize,
}

/// Slack for filesystems that store times coarsely (FAT32: 2 s).
const TIME_SLACK: Duration = Duration::from_secs(2);
/// Reserve kept free on the destination beyond the plan's size.
const SPACE_MARGIN: u64 = 256 << 20;

pub fn db_path(dest: &Path) -> PathBuf {
    dest.join(STATE_DIR).join("chronosort.db")
}

pub fn open_db(dest: &Path) -> Result<Db> {
    fs::create_dir_all(dest.join(STATE_DIR))?;
    Ok(Db::open(&db_path(dest))?)
}

/// Join a `/`-separated relative path safely: refuses `..`, roots and drive
/// prefixes so a crafted name can never escape the destination.
pub fn join_rel(root: &Path, rel: &str) -> io::Result<PathBuf> {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        let c = Path::new(part).components().collect::<Vec<_>>();
        match c.as_slice() {
            [Component::Normal(n)] => p.push(n),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsafe path component {part:?}"))),
        }
    }
    Ok(p)
}

fn partial_path(dst: &Path) -> PathBuf {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dst.with_file_name(format!(".{name}.chronosort-partial"))
}

fn norm(p: &Path) -> io::Result<String> {
    let a = std::path::absolute(p)?;
    let s = a.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/').to_string();
    Ok(if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s })
}

/// Destination inside the source (or the reverse) would make ChronoSort read
/// its own output, or write into the archive it is supposed to protect.
fn check_roots(src: &Path, dest: &Path) -> Result<()> {
    let (s, d) = (norm(src)?, norm(dest)?);
    let inside = |a: &str, b: &str| a == b || a.starts_with(&format!("{b}/"));
    if inside(&d, &s) {
        return Err(ExecError::UnsafePaths("the destination is inside the source folder".into()));
    }
    if inside(&s, &d) {
        return Err(ExecError::UnsafePaths("the source is inside the destination folder".into()));
    }
    Ok(())
}

enum CopyResult {
    Copied { hash: String, warning: Option<String> },
    AlreadyPresent,
}

fn close_enough(a: SystemTime, b: SystemTime) -> bool {
    a.duration_since(b).or_else(|_| b.duration_since(a)).map(|d| d <= TIME_SLACK).unwrap_or(false)
}

/// Create the missing folders between `root` and `dir`, recording each one.
fn ensure_dirs(root: &Path, dir: &Path, db: &Db, run_id: i64) -> Result<()> {
    let mut missing = Vec::new();
    let mut d = dir;
    while d != root && !d.exists() {
        missing.push(d.to_path_buf());
        d = d.parent().ok_or_else(|| io::Error::other("folder outside destination"))?;
    }
    for m in missing.into_iter().rev() {
        match fs::create_dir(&m) {
            Ok(()) => {
                let rel = m.strip_prefix(root).map_err(io::Error::other)?.to_string_lossy().replace('\\', "/");
                db.add_created_dir(run_id, &rel)?;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn hash_reader(mut r: impl Read, mut sink: Option<&mut File>) -> io::Result<(String, u64)> {
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n_total = 0u64;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        if let Some(out) = sink.as_deref_mut() {
            out.write_all(&buf[..n])?;
        }
        n_total += n as u64;
    }
    Ok((h.finalize().to_hex().to_string(), n_total))
}

/// The verified copy. `dst` must not exist; it is never overwritten.
fn copy_verified(src: &Path, dst: &Path) -> io::Result<CopyResult> {
    let before = fs::metadata(src)?;
    let times: Times = read_times(src)?;
    if dst.exists() {
        // Planner avoided clashes; this is a race with something else writing.
        let same = cs_core::hash_file(dst)? == cs_core::hash_file(src)?;
        return if same {
            Ok(CopyResult::AlreadyPresent)
        } else {
            Err(io::Error::new(io::ErrorKind::AlreadyExists, "a different file appeared at the destination name"))
        };
    }
    let partial = partial_path(dst);
    if partial.exists() {
        fs::remove_file(&partial)?; // our own leftover (name is ours by construction)
    }
    let result = (|| -> io::Result<CopyResult> {
        let mut out = File::options().write(true).create_new(true).open(&partial)?;
        let (src_hash, n) = hash_reader(File::open(src)?, Some(&mut out))?;
        out.sync_all()?;
        apply_times(&out, &times)?;
        drop(out);

        let after = fs::metadata(src)?;
        if after.len() != before.len() || after.modified()? != before.modified()? || n != before.len() {
            return Err(io::Error::other("source changed while it was being copied"));
        }
        let (dst_hash, _) = hash_reader(File::open(&partial)?, None)?;
        if dst_hash != src_hash {
            return Err(io::Error::other("verification failed: copy differs from source"));
        }
        let t = read_times(&partial)?;
        let mut warning = None;
        if !close_enough(t.modified, times.modified) {
            return Err(io::Error::other("modified time could not be preserved"));
        }
        if t.modified != times.modified {
            warning = Some("destination filesystem rounds file times (e.g. FAT/exFAT)".to_string());
        }
        if CAN_SET_CREATED
            && let (Some(want), Some(got)) = (times.created, t.created)
            && !close_enough(want, got)
        {
            warning = Some("creation time could not be preserved on this filesystem".to_string());
        }
        rename_no_replace(&partial, dst)?;
        if let Some(parent) = dst.parent() {
            cs_platform::sync_dir(parent)?;
        }
        Ok(CopyResult::Copied { hash: src_hash, warning })
    })();
    if result.is_err() && partial.exists() {
        let _ = fs::remove_file(&partial);
    }
    result
}

/// Finish the bookkeeping of an interrupted run (crash, power cut, killed process).
pub fn recover(dest: &Path) -> Result<RecoverReport> {
    let mut r = RecoverReport::default();
    if !db_path(dest).exists() {
        return Ok(r);
    }
    let db = open_db(dest)?;
    for row in db.journal_pending_all()? {
        if row.op == "trash" {
            let gone = !PathBuf::from(&row.src).exists();
            let state = if gone { JournalState::Done } else { JournalState::Failed };
            db.journal_set(row.id, state, None, (!gone).then_some("interrupted before completion"))?;
            if gone {
                r.completed += 1
            } else {
                r.failed += 1
            }
            continue;
        }
        let dst = join_rel(dest, &row.dst)?;
        let partial = partial_path(&dst);
        if partial.exists() {
            fs::remove_file(&partial)?;
            r.partials_removed += 1;
        }
        let src = PathBuf::from(&row.src);
        let complete = match row.op.as_str() {
            // Rename-style move: done once the file is at the destination and gone from the source.
            "relocate" => dst.is_file() && !src.exists(),
            // Sent to the trash: done once the source is gone.
            "trash" => !src.exists(),
            _ => dst.exists() && src.exists() && cs_core::hash_file(&dst)? == cs_core::hash_file(&src)?,
        };
        if complete {
            let h = if dst.is_file() { Some(cs_core::hash_file(&dst)?) } else { None };
            db.journal_set(row.id, JournalState::Done, h.as_deref(), None)?;
            r.completed += 1;
        } else {
            db.journal_set(row.id, JournalState::Failed, None, Some("interrupted before completion"))?;
            r.failed += 1;
        }
    }
    Ok(r)
}

/// Execute a ready plan from `src_root` into `dest`.
pub fn execute(src_root: &Path, dest: &Path, plan: &Plan, opts: &ExecOptions) -> Result<ExecReport> {
    if !plan.ready() {
        return Err(ExecError::NotReady("the source has month folders: choose keep or flatten first".into()));
    }
    check_roots(src_root, dest)?;
    fs::create_dir_all(dest)?;
    let free = cs_platform::free_space(dest)?;
    let need = plan.total_bytes + SPACE_MARGIN;
    if free < need {
        return Err(ExecError::NoSpace { need, free });
    }
    recover(dest)?;
    let db = open_db(dest)?;
    let mode = match opts.mode {
        TransferMode::Copy => "copy",
        TransferMode::Move => "move",
    };
    let run_id = db.create_run(&src_root.to_string_lossy(), &dest.to_string_lossy(), mode)?;
    let mut rep = ExecReport { run_id, ..Default::default() };
    let plan_json = serde_json::to_vec_pretty(plan).map_err(io::Error::other)?;
    fs::write(dest.join(STATE_DIR).join(format!("run-{run_id}-plan.json")), plan_json)?;
    db.set_run_status(run_id, "running")?;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(opts.threads.max(1)).build().map_err(io::Error::other)?;

    // Index ranges of whole groups (the planner emits group members contiguously).
    let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, f) in plan.files.iter().enumerate() {
        match groups.last_mut() {
            Some(g) if plan.files[g.start].group == f.group => g.end = i + 1,
            _ => groups.push(i..i + 1),
        }
    }

    let limit = opts.stop_after.unwrap_or(usize::MAX);
    let mut done = 0usize;
    let mut gi = 0;
    while gi < groups.len() && done < limit {
        // Next chunk: whole groups, about CHUNK_FILES files.
        let first = groups[gi].start;
        let mut last = first;
        while gi < groups.len() && (last == first || groups[gi].end - first <= CHUNK_FILES) {
            last = groups[gi].end;
            gi += 1;
        }
        let last = last.min(first + (limit - done));
        let idx: Vec<usize> = (first..last).collect();

        // 1) Record intent (pending) for the whole chunk in one transaction.
        struct Job {
            i: usize,
            jid: i64,
            src: PathBuf,
            dst: PathBuf,
            pre: Option<String>,
        }
        let jobs: Vec<Job> = db.batch(|db| {
            let mut v = Vec::new();
            for &i in &idx {
                let f = &plan.files[i];
                let src = join_rel(src_root, &f.src_rel)?;
                let dst = join_rel(dest, &f.dst_rel)?;
                let jid = db.journal_begin(run_id, mode, &src.to_string_lossy(), &f.dst_rel)?;
                let pre = match dst.parent().map_or(Ok(()), |p| ensure_dirs(dest, p, db, run_id)) {
                    Ok(()) => None,
                    Err(e) => Some(e.to_string()),
                };
                v.push(Job { i, jid, src, dst, pre });
            }
            Ok(v)
        })?;

        // 2) Copy: small files in parallel, large ones sequentially.
        let run_one = |j: &Job| match &j.pre {
            Some(e) => Err(io::Error::other(e.clone())),
            None => copy_verified(&j.src, &j.dst),
        };
        let (small, large): (Vec<&Job>, Vec<&Job>) = jobs.iter().partition(|j| plan.files[j.i].size < LARGE_FILE);
        let mut results: Vec<(&Job, io::Result<CopyResult>)> = pool.install(|| {
            use rayon::prelude::*;
            small.par_iter().map(|j| (*j, run_one(j))).collect()
        });
        results.extend(large.into_iter().map(|j| (j, run_one(j))));
        results.sort_by_key(|(j, _)| j.i);

        // 3) Record outcomes in one transaction.
        let mut ok = vec![false; plan.files.len()];
        db.batch(|db| {
            for (j, res) in &results {
                let f = &plan.files[j.i];
                match res {
                    Ok(CopyResult::Copied { hash, warning }) => {
                        db.journal_set(j.jid, JournalState::Done, Some(hash), None)?;
                        rep.copied += 1;
                        rep.bytes += f.size;
                        ok[j.i] = true;
                        if let Some(w) = warning
                            && !rep.warnings.contains(w)
                        {
                            rep.warnings.push(w.clone());
                        }
                    }
                    Ok(CopyResult::AlreadyPresent) => {
                        db.journal_set(j.jid, JournalState::Failed, None, Some("identical file already present"))?;
                        rep.already_present += 1;
                    }
                    Err(e) => {
                        db.journal_set(j.jid, JournalState::Failed, None, Some(&e.to_string()))?;
                        rep.failed.push((f.src_rel.clone(), e.to_string()));
                    }
                }
            }
            Ok(())
        })?;
        done += jobs.len();
        if let Some(p) = &opts.progress {
            p(done, plan.files.len(), rep.bytes);
        }

        // 4) Move mode: a group's sources go to the trash only when the whole
        //    group was copied and verified, and only after the journal is on disk.
        if opts.mode == TransferMode::Move {
            let complete: Vec<&std::ops::Range<usize>> = groups
                .iter()
                .filter(|g| g.start >= first && g.end <= last && (g.start..g.end).all(|i| ok[i]))
                .collect();
            if !complete.is_empty() {
                db.durable_barrier()?;
                let by_index: std::collections::HashMap<usize, &Job> = jobs.iter().map(|j| (j.i, j)).collect();
                db.batch(|db| {
                    for g in complete {
                        for i in g.start..g.end {
                            let j = by_index[&i];
                            match (opts.trash)(&j.src) {
                                Ok(()) => {
                                    db.journal_set(j.jid, JournalState::SourceTrashed, None, None)?;
                                    rep.trashed += 1;
                                }
                                Err(e) => rep
                                    .warnings
                                    .push(format!("copied, but could not move {} to the trash: {e}", j.src.display())),
                            }
                        }
                    }
                    Ok(())
                })?;
            }
        }
    }
    rep.stopped_early = done < plan.files.len();
    db.set_run_status(run_id, if rep.stopped_early { "interrupted" } else { "complete" })?;
    Ok(rep)
}

/// Reverse a run (default: the latest). Copies are removed only if they are
/// still byte-identical to what was written; moved files are copied back to
/// their original location first.
pub fn undo(dest: &Path, run: Option<i64>) -> Result<UndoReport> {
    recover(dest)?;
    let db = open_db(dest)?;
    let run_id = match run {
        Some(r) => r,
        None => db.last_run()?.ok_or_else(|| ExecError::NotReady("nothing to undo".into()))?,
    };
    let mut rep = UndoReport { run_id, ..Default::default() };
    for row in db.journal_entries(run_id)?.into_iter().rev() {
        match row.state {
            JournalState::Done | JournalState::SourceTrashed => {}
            _ => continue,
        }
        if row.op == "trash" {
            rep.failed.push((row.src.clone(), "in the Recycle Bin: restore it from there".into()));
            continue;
        }
        let dst = join_rel(dest, &row.dst)?;
        if !dst.exists() {
            rep.failed.push((row.dst.clone(), "copy no longer exists".into()));
            continue;
        }
        if row.hash.as_deref() != Some(cs_core::hash_file(&dst)?.as_str()) {
            rep.kept_modified.push(row.dst.clone());
            continue;
        }
        if row.op == "relocate" {
            // Move the duplicate back to where it was found.
            let src = PathBuf::from(&row.src);
            if src.exists() {
                rep.failed.push((row.src.clone(), "a file is already back at the original place".into()));
                continue;
            }
            if let Some(p) = src.parent() {
                fs::create_dir_all(p)?;
            }
            let back = match cs_platform::rename_no_replace(&dst, &src) {
                Ok(()) => Ok(()),
                Err(_) => copy_verified(&dst, &src).map(|_| ()).and_then(|_| fs::remove_file(&dst)),
            };
            match back {
                Ok(()) => {
                    db.journal_set(row.id, JournalState::Undone, None, None)?;
                    rep.restored += 1;
                }
                Err(e) => rep.failed.push((row.src.clone(), format!("could not move back: {e}"))),
            }
            continue;
        }
        if row.state == JournalState::SourceTrashed {
            let src = PathBuf::from(&row.src);
            if let Some(p) = src.parent() {
                fs::create_dir_all(p)?;
            }
            match copy_verified(&dst, &src) {
                Ok(_) => rep.restored += 1,
                Err(e) => {
                    rep.failed.push((row.src.clone(), format!("could not restore: {e}")));
                    continue; // keep the destination copy: it is the only one left
                }
            }
        }
        fs::remove_file(&dst)?;
        db.journal_set(row.id, JournalState::Undone, None, None)?;
        rep.removed += 1;
    }
    for d in db.created_dirs(run_id)? {
        if fs::remove_dir(join_rel(dest, &d)?).is_ok() {
            rep.dirs_removed += 1; // only succeeds when empty
        }
    }
    db.set_run_status(run_id, "undone")?;
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_paths_cannot_escape() {
        let root = Path::new("/dest");
        assert!(join_rel(root, "2005/a.jpg").is_ok());
        assert!(join_rel(root, "../evil.jpg").is_err());
        assert!(join_rel(root, "2005/../../evil.jpg").is_err());
        assert!(join_rel(root, "").is_err());
        #[cfg(windows)]
        assert!(join_rel(root, "C:/Windows/evil.dll").is_err());
    }

    #[test]
    fn nested_roots_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("Old_Photos");
        assert!(check_roots(&src, &src.join("Sorted")).is_err());
        assert!(check_roots(&src.join("sub"), &src).is_err());
        assert!(check_roots(&src, &d.path().join("Sorted_Photos")).is_ok());
        assert!(check_roots(&src, &d.path().join("Old_Photos2")).is_ok(), "prefix of a name is not nesting");
    }
}
