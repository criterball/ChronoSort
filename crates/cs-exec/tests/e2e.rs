//! End-to-end on the synthetic corpus: analyse → plan → copy → verify → re-run
//! → undo → interrupted run + recovery. The source must never change.

use cs_core::{DatePolicy, analyze, hash_file};
use cs_exec::{ExecOptions, execute, open_db, recover, undo};
use cs_model::civil::unix;
use cs_plan::{MonthPolicy, PlanOptions, Skipped, build};
use std::collections::BTreeMap;
use std::path::Path;

fn policy() -> DatePolicy {
    DatePolicy::new(unix(2026, 11, 20, 0, 0, 0))
}

/// Every file under `root` (excluding ChronoSort's state dir): rel -> (hash, mtime, created).
fn snapshot(root: &Path) -> BTreeMap<String, (String, std::time::SystemTime, Option<std::time::SystemTime>)> {
    let mut m = BTreeMap::new();
    for e in walk(root) {
        let t = cs_platform::read_times(&e).unwrap();
        let rel = e.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        m.insert(rel, (hash_file(&e).unwrap(), t.modified, t.created));
    }
    m
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            if e.file_name() == ".chronosort" {
                continue;
            }
            if e.file_type().unwrap().is_dir() { stack.push(e.path()) } else { out.push(e.path()) }
        }
    }
    out
}

fn flat() -> PlanOptions {
    PlanOptions { months: MonthPolicy::Flatten, ..Default::default() }
}

#[test]
fn copy_verify_rerun_undo() {
    let d = tempfile::tempdir().unwrap();
    cs_testgen::generate(d.path()).unwrap();
    let src = d.path().join(cs_testgen::SOURCE_DIR);
    let dest = d.path().join("Sorted_Photos");
    let source_before = snapshot(&src);

    let a = analyze(&src, &policy()).unwrap();

    // Month folders in the source: must ask first, and refuse to run until answered.
    let ask = build(&a.items, &dest, &PlanOptions::default()).unwrap();
    let q = ask.month_question.as_ref().expect("2005/03 should raise the month question");
    assert_eq!(q.folders, vec![("2005/03".to_string(), 1)]);
    assert!(execute(&src, &dest, &ask, &ExecOptions::default()).is_err());
    let keep = build(&a.items, &dest, &PlanOptions { months: MonthPolicy::Keep, ..Default::default() }).unwrap();
    assert!(keep.files.iter().any(|f| f.dst_rel == "2005/03/scan_001.jpg"));

    // ---- copy (flatten, keep names) --------------------------------------------------
    let plan = build(&a.items, &dest, &flat()).unwrap();
    let dst_of = |s: &str| plan.files.iter().find(|f| f.src_rel == s).map(|f| f.dst_rel.as_str());
    assert_eq!(dst_of("Camera/DSC_0001.jpg"), Some("2005/DSC_0001.jpg"));
    assert_eq!(dst_of("CardA/DSC_0001.jpg"), Some("2005/DSC_0001_1.jpg"), "clash gets _1");
    assert_eq!(dst_of("2005/03/scan_001.jpg"), Some("2005/scan_001.jpg"));
    assert_eq!(dst_of("Raw/DSC_0700.NEF"), Some("2013/DSC_0700.NEF"));
    assert_eq!(dst_of("Takeout/Google Photos/Photos from 2016/IMG_1234.jpg.json"), Some("2016/IMG_1234.jpg.json"));
    assert_eq!(dst_of("Backup/DSC_0001.jpg"), None, "duplicate is not copied");
    assert!(
        plan.skipped
            .iter()
            .any(|s| matches!(s, Skipped::Duplicate { src_rel, .. } if src_rel == "Backup/DSC_0001.jpg"))
    );
    assert_eq!(dst_of("2003/DSC_0100.jpg"), None, "review items wait for the user");

    let rep = execute(&src, &dest, &plan, &ExecOptions::default()).unwrap();
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
    assert_eq!(rep.copied, plan.files.len());

    let out = snapshot(&dest);
    assert_eq!(out.len(), plan.files.len(), "exactly the planned files, nothing else");
    for f in &plan.files {
        let (h, m, c) = &out[&f.dst_rel];
        let (sh, sm, sc) = &source_before[&f.src_rel];
        assert_eq!(h, sh, "content of {}", f.dst_rel);
        assert_eq!(m, sm, "modified time of {}", f.dst_rel);
        if cs_platform::CAN_SET_CREATED {
            assert_eq!(c, sc, "created time of {}", f.dst_rel);
        }
    }
    assert!(!walk(&dest).iter().any(|p| p.to_string_lossy().contains("chronosort-partial")));
    assert_eq!(snapshot(&src), source_before, "source untouched");

    // ---- re-run: nothing new to copy ---------------------------------------------------
    let a2 = analyze(&src, &policy()).unwrap();
    let again = build(&a2.items, &dest, &flat()).unwrap();
    assert!(again.files.is_empty(), "already sorted: {:?}", again.files.iter().map(|f| &f.src_rel).collect::<Vec<_>>());
    assert!(again.skipped.iter().any(|s| matches!(s, Skipped::AlreadyInDestination { .. })));

    // ---- undo ----------------------------------------------------------------------
    let u = undo(&dest, None).unwrap();
    assert_eq!(u.removed, rep.copied);
    assert!(u.failed.is_empty() && u.kept_modified.is_empty());
    assert!(snapshot(&dest).is_empty(), "destination emptied");
    assert!(std::fs::read_dir(&dest).unwrap().all(|e| e.unwrap().file_name() == ".chronosort"), "year folders removed");
    assert_eq!(snapshot(&src), source_before, "source untouched after undo");
}

#[test]
fn undo_keeps_files_changed_after_copy() {
    let d = tempfile::tempdir().unwrap();
    cs_testgen::generate(d.path()).unwrap();
    let src = d.path().join(cs_testgen::SOURCE_DIR);
    let dest = d.path().join("Sorted");
    let a = analyze(&src, &policy()).unwrap();
    let plan = build(&a.items, &dest, &flat()).unwrap();
    execute(&src, &dest, &plan, &ExecOptions::default()).unwrap();
    let edited = dest.join("2005").join("DSC_0001.jpg");
    std::fs::write(&edited, b"user edited this").unwrap();
    let u = undo(&dest, None).unwrap();
    assert_eq!(u.kept_modified, vec!["2005/DSC_0001.jpg".to_string()]);
    assert!(edited.exists(), "a file the user changed is never deleted");
}

#[test]
fn interrupted_run_resumes_cleanly() {
    let d = tempfile::tempdir().unwrap();
    cs_testgen::generate(d.path()).unwrap();
    let src = d.path().join(cs_testgen::SOURCE_DIR);
    let dest = d.path().join("Sorted");
    let a = analyze(&src, &policy()).unwrap();
    let plan = build(&a.items, &dest, &flat()).unwrap();

    // Stop after 5 files…
    let r1 = execute(&src, &dest, &plan, &ExecOptions { stop_after: Some(5), ..Default::default() }).unwrap();
    assert!(r1.stopped_early && r1.copied == 5);

    // …and simulate a crash in the middle of the 6th: journal says pending, a partial file exists.
    let f = &plan.files[5];
    let db = open_db(&dest).unwrap();
    let jid = db
        .journal_begin(r1.run_id, "copy", &cs_exec::join_rel(&src, &f.src_rel).unwrap().to_string_lossy(), &f.dst_rel)
        .unwrap();
    let dst = cs_exec::join_rel(&dest, &f.dst_rel).unwrap();
    std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
    let partial = dst.with_file_name(format!(".{}.chronosort-partial", dst.file_name().unwrap().to_string_lossy()));
    std::fs::write(&partial, b"half a file").unwrap();
    drop(db);

    let rec = recover(&dest).unwrap();
    assert_eq!((rec.partials_removed, rec.failed), (1, 1));
    assert!(!partial.exists());
    assert_eq!(open_db(&dest).unwrap().journal_state(jid).unwrap(), Some(cs_db::JournalState::Failed));

    // Run again: the 5 finished files are recognised, the rest are copied.
    let plan2 = build(&analyze(&src, &policy()).unwrap().items, &dest, &flat()).unwrap();
    assert_eq!(plan2.files.len(), plan.files.len() - 5);
    let r2 = execute(&src, &dest, &plan2, &ExecOptions::default()).unwrap();
    assert!(r2.failed.is_empty());
    assert_eq!(snapshot(&dest).len(), plan.files.len());
}

/// Test stand-in for the Recycle Bin: moves the file into a "trash" folder
/// next to the corpus so nothing reaches the real Recycle Bin.
fn fake_trash(p: &Path) -> std::io::Result<()> {
    let mut root = p.to_path_buf();
    while root.file_name().is_some_and(|n| n != cs_testgen::SOURCE_DIR) {
        root.pop();
    }
    let bin = root.parent().unwrap().join("FakeTrash");
    std::fs::create_dir_all(&bin)?;
    let n = std::fs::read_dir(&bin)?.count();
    std::fs::rename(p, bin.join(format!("{n}_{}", p.file_name().unwrap().to_string_lossy())))
}

#[test]
fn move_mode_then_undo_restores_originals() {
    use cs_model::TransferMode;
    let d = tempfile::tempdir().unwrap();
    cs_testgen::generate(d.path()).unwrap();
    let src = d.path().join(cs_testgen::SOURCE_DIR);
    let dest = d.path().join("Sorted");
    let before = snapshot(&src);
    let a = analyze(&src, &policy()).unwrap();
    let plan = build(&a.items, &dest, &flat()).unwrap();
    let opts = ExecOptions { mode: TransferMode::Move, trash: fake_trash, ..Default::default() };
    let r = execute(&src, &dest, &plan, &opts).unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert_eq!(r.trashed, plan.files.len(), "every moved original went to the trash");

    let after = snapshot(&src);
    for f in &plan.files {
        assert!(!after.contains_key(&f.src_rel), "{} should have been moved", f.src_rel);
        assert_eq!(snapshot(&dest)[&f.dst_rel].0, before[&f.src_rel].0, "copy of {} verified", f.src_rel);
    }
    // Duplicates, review items and junk stay where they were.
    assert!(after.contains_key("Backup/DSC_0001.jpg"));
    assert!(after.contains_key("2003/DSC_0100.jpg"));
    assert!(after.contains_key("Camera/Thumbs.db"));

    // Undo copies everything back (with original times) and removes the copies.
    let u = undo(&dest, None).unwrap();
    assert_eq!((u.restored, u.removed), (plan.files.len(), plan.files.len()));
    let restored = snapshot(&src);
    for (k, (h, m, c)) in &before {
        let (h2, m2, c2) = &restored[k];
        assert_eq!((h, m), (h2, m2), "{k} restored with its modified time");
        if cs_platform::CAN_SET_CREATED {
            assert_eq!(c, c2, "{k} restored with its created time");
        }
    }
}
