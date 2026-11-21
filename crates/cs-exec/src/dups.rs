//! Acting on duplicates: move the extra copies to a "Duplicates" folder
//! (undoable) or send them to the Recycle Bin.
//!
//! Safety rules, checked again right before touching anything:
//! * the copy to keep must still exist and still have the scanned content;
//! * an extra copy is only touched if it still has exactly that content;
//! * the kept copy itself is never touched, so one copy always remains.

use crate::{CopyResult, ExecError, Result, copy_verified, ensure_dirs, join_rel, open_db};
use cs_db::JournalState;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What to do with one group of identical files.
#[derive(Debug, Clone)]
pub struct DupAction {
    pub keep: PathBuf,
    /// BLAKE3 of the group's content (from the scan).
    pub hash: String,
    /// Extra copies: absolute path + where to put it under the Duplicates folder.
    pub extras: Vec<(PathBuf, String)>,
}

#[derive(Debug, Clone)]
pub enum DupMode {
    /// Move extras into this folder, keeping their folder structure. Undoable.
    MoveTo(PathBuf),
    /// Send extras to the OS Recycle Bin / Trash.
    Trash,
}

#[derive(Debug, Default, Serialize)]
pub struct DupReport {
    pub run_id: i64,
    pub moved: usize,
    pub trashed: usize,
    pub bytes_freed: u64,
    /// (path, reason) for anything left untouched.
    pub skipped: Vec<(String, String)>,
}

fn is_cross_device(e: &io::Error) -> bool {
    // ERROR_NOT_SAME_DEVICE (Windows) / EXDEV (Unix)
    matches!(e.raw_os_error(), Some(17) if cfg!(windows)) || matches!(e.raw_os_error(), Some(18) if cfg!(unix))
}

/// Pick `name_1.ext`, `name_2.ext` … if `dst` is taken.
fn free_name(dst: PathBuf) -> PathBuf {
    if !dst.exists() {
        return dst;
    }
    let stem = dst.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = dst.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..).map(|n| dst.with_file_name(format!("{stem}_{n}{ext}"))).find(|p| !p.exists()).unwrap()
}

/// Carry out the user's choices. `journal_dir` holds the undo journal
/// (the Duplicates folder itself for MoveTo; an app folder for Trash).
pub fn resolve_duplicates(
    actions: &[DupAction],
    mode: &DupMode,
    scanned_roots: &[PathBuf],
    journal_dir: &Path,
    trash: fn(&Path) -> io::Result<()>,
) -> Result<DupReport> {
    if let DupMode::MoveTo(dir) = mode {
        for r in scanned_roots {
            crate::check_roots(r, dir).map_err(|_| {
                ExecError::UnsafePaths(
                    "choose a Duplicates folder outside the folders that were checked, \
                     otherwise the copies would be found again next time"
                        .into(),
                )
            })?;
        }
        fs::create_dir_all(dir)?;
    }
    let db = open_db(journal_dir)?;
    let (label, dest_desc) = match mode {
        DupMode::MoveTo(d) => ("duplicates-move", d.display().to_string()),
        DupMode::Trash => ("duplicates-trash", "Recycle Bin".to_string()),
    };
    let src_desc = scanned_roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(" | ");
    let run_id = db.create_run(&src_desc, &dest_desc, "move")?;
    db.set_run_status(run_id, label)?;
    let mut rep = DupReport { run_id, ..Default::default() };

    for a in actions {
        let keep_ok = a.keep.is_file() && cs_core::hash_file(&a.keep).ok().as_deref() == Some(a.hash.as_str());
        if !keep_ok {
            for (p, _) in &a.extras {
                rep.skipped.push((p.display().to_string(), "the copy to keep is missing or has changed".into()));
            }
            continue;
        }
        for (src, rel) in &a.extras {
            let same_file = fs::canonicalize(src).ok() == fs::canonicalize(&a.keep).ok();
            if same_file || !src.is_file() {
                rep.skipped.push((src.display().to_string(), "not found (or it is the kept copy)".into()));
                continue;
            }
            let size = fs::metadata(src)?.len();
            if cs_core::hash_file(src)? != a.hash {
                rep.skipped.push((src.display().to_string(), "changed since the scan".into()));
                continue;
            }
            match mode {
                DupMode::MoveTo(dir) => {
                    let dst = free_name(join_rel(dir, rel)?);
                    let dst_rel = dst.strip_prefix(dir).map_err(io::Error::other)?.to_string_lossy().replace('\\', "/");
                    let jid = db.journal_begin(run_id, "relocate", &src.to_string_lossy(), &dst_rel)?;
                    if let Some(parent) = dst.parent() {
                        ensure_dirs(dir, parent, &db, run_id)?;
                    }
                    let res = match cs_platform::rename_no_replace(src, &dst) {
                        Ok(()) => Ok(()),
                        // Different drive: verified copy, then the original goes to the trash.
                        Err(e) if is_cross_device(&e) => match copy_verified(src, &dst) {
                            Ok(CopyResult::Copied { .. }) => trash(src),
                            Ok(CopyResult::AlreadyPresent) => Err(io::Error::other("target name taken")),
                            Err(e) => Err(e),
                        },
                        Err(e) => Err(e),
                    };
                    match res {
                        Ok(()) => {
                            db.journal_set(jid, JournalState::Done, Some(&a.hash), None)?;
                            rep.moved += 1;
                            rep.bytes_freed += size;
                        }
                        Err(e) => {
                            db.journal_set(jid, JournalState::Failed, None, Some(&e.to_string()))?;
                            rep.skipped.push((src.display().to_string(), e.to_string()));
                        }
                    }
                }
                DupMode::Trash => {
                    let jid = db.journal_begin(run_id, "trash", &src.to_string_lossy(), "")?;
                    db.durable_barrier()?;
                    match trash(src) {
                        Ok(()) => {
                            db.journal_set(jid, JournalState::Done, Some(&a.hash), None)?;
                            rep.trashed += 1;
                            rep.bytes_freed += size;
                        }
                        Err(e) => {
                            db.journal_set(jid, JournalState::Failed, None, Some(&e.to_string()))?;
                            rep.skipped.push((src.display().to_string(), e.to_string()));
                        }
                    }
                }
            }
        }
    }
    db.set_run_status(run_id, &format!("{label}-complete"))?;
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_trash(p: &Path) -> io::Result<()> {
        let bin = p.ancestors().nth(3).unwrap().join("FakeTrash");
        fs::create_dir_all(&bin)?;
        let n = fs::read_dir(&bin)?.count();
        fs::rename(p, bin.join(format!("{n}_{}", p.file_name().unwrap().to_string_lossy())))
    }

    fn setup() -> (tempfile::TempDir, PathBuf, Vec<DupAction>) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("data").join("Photos");
        fs::create_dir_all(root.join("Backup")).unwrap();
        fs::write(root.join("a.jpg"), b"same bytes").unwrap();
        fs::write(root.join("Backup").join("a.jpg"), b"same bytes").unwrap();
        fs::write(root.join("Backup").join("a (1).jpg"), b"same bytes").unwrap();
        let hash = cs_core::hash_file(&root.join("a.jpg")).unwrap();
        let action = DupAction {
            keep: root.join("a.jpg"),
            hash,
            extras: vec![
                (root.join("Backup").join("a.jpg"), "Photos/Backup/a.jpg".into()),
                (root.join("Backup").join("a (1).jpg"), "Photos/Backup/a (1).jpg".into()),
            ],
        };
        (d, root, vec![action])
    }

    #[test]
    fn move_to_folder_and_undo() {
        let (d, root, actions) = setup();
        let dup = d.path().join("Duplicates");
        let r =
            resolve_duplicates(&actions, &DupMode::MoveTo(dup.clone()), std::slice::from_ref(&root), &dup, fake_trash)
                .unwrap();
        assert_eq!((r.moved, r.skipped.len()), (2, 0));
        assert!(root.join("a.jpg").exists(), "kept copy untouched");
        assert!(!root.join("Backup").join("a.jpg").exists());
        assert!(dup.join("Photos/Backup/a (1).jpg").exists());

        let u = crate::undo(&dup, None).unwrap();
        assert_eq!(u.restored, 2);
        assert!(root.join("Backup").join("a.jpg").exists() && root.join("Backup").join("a (1).jpg").exists());
        assert!(!dup.join("Photos").exists(), "empty folders removed again");
    }

    #[test]
    fn refuses_when_kept_copy_changed_and_never_touches_it() {
        let (d, root, mut actions) = setup();
        fs::write(root.join("a.jpg"), b"edited!").unwrap();
        let dup = d.path().join("Duplicates");
        let r =
            resolve_duplicates(&actions, &DupMode::MoveTo(dup.clone()), std::slice::from_ref(&root), &dup, fake_trash)
                .unwrap();
        assert_eq!(r.moved, 0);
        assert_eq!(r.skipped.len(), 2);
        assert!(root.join("Backup").join("a.jpg").exists(), "extras stay when the keeper is not safe");

        // The kept copy listed as an "extra" must never be removed.
        fs::write(root.join("a.jpg"), b"same bytes").unwrap();
        actions[0].extras.push((root.join("a.jpg"), "Photos/a.jpg".into()));
        let r = resolve_duplicates(
            &actions,
            &DupMode::Trash,
            std::slice::from_ref(&root),
            &d.path().join("journal"),
            fake_trash,
        )
        .unwrap();
        assert_eq!(r.trashed, 2);
        assert!(root.join("a.jpg").exists(), "one copy always remains");
    }

    #[test]
    fn duplicates_folder_inside_scanned_folder_is_refused() {
        let (_d, root, actions) = setup();
        let inside = root.join("Duplicates");
        assert!(
            resolve_duplicates(
                &actions,
                &DupMode::MoveTo(inside.clone()),
                std::slice::from_ref(&root),
                &inside,
                fake_trash
            )
            .is_err()
        );
    }
}
