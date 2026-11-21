//! `cs-core`: the analysis pipeline shared by the CLI and the desktop app.
//! scan → read facts (parallel) → hash → decide per group → reports.
//! Read-only: nothing here writes to the source.

pub mod dup;

use cs_date::Policy;
use cs_meta::Facts;
use cs_model::{Decision, ItemReport, MediaKind};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub use cs_date::Policy as DatePolicy;

#[derive(Debug)]
pub struct Analysis {
    pub items: Vec<ItemReport>,
    pub errors: Vec<(PathBuf, String)>,
    pub skipped_links: Vec<PathBuf>,
}

/// BLAKE3 of a file's content, streamed (constant memory).
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().to_hex().to_string())
}

/// Sort key for choosing which identical copy is the original:
/// not-a-copy path first, then oldest modified time, then shortest path, then name.
pub(crate) fn original_rank(rel: &str, mtime: Option<i64>) -> (bool, i64, usize, String) {
    let l = rel.to_lowercase();
    let looks_copied = ["backup", "copy", "kopie", "duplicate", " (1)", " (2)", "- copy"].iter().any(|k| l.contains(k));
    (looks_copied, mtime.unwrap_or(i64::MAX), rel.len(), rel.to_string())
}

pub fn analyze(root: &Path, policy: &Policy) -> io::Result<Analysis> {
    analyze_with_progress(root, policy, &|_, _| {})
}

/// Like [`analyze`], calling `progress(done, total)` as files are read.
pub fn analyze_with_progress(
    root: &Path,
    policy: &Policy,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> io::Result<Analysis> {
    let scan = cs_scan::scan(root)?;
    let files = scan.files;
    let total = files.len();
    let counter = std::sync::atomic::AtomicUsize::new(0);
    progress(0, total);
    // Read facts + hashes in parallel; order is preserved by index.
    let per_file: Vec<(Facts, Result<String, String>)> = files
        .par_iter()
        .map(|f| {
            let r = if f.kind == MediaKind::Junk {
                (Facts::default(), Err("junk".into()))
            } else {
                (cs_meta::read_facts(f), hash_file(&f.abs).map_err(|e| e.to_string()))
            };
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if n.is_multiple_of(25) || n == total {
                progress(n, total);
            }
            r
        })
        .collect();

    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() {
        if f.kind != MediaKind::Junk {
            groups.entry(f.group.as_str()).or_default().push(i);
        }
    }
    let mut outcome: HashMap<usize, cs_date::Outcome> = HashMap::new();
    for idx in groups.values() {
        let facts: Vec<&Facts> = idx.iter().map(|&i| &per_file[i].0).collect();
        let o = cs_date::decide(&facts, policy);
        for &i in idx {
            outcome.insert(i, o.clone());
        }
    }

    // Exact duplicates: pick the most "original-looking" copy as primary.
    let mut primary: HashMap<&str, usize> = HashMap::new();
    for i in 0..files.len() {
        let Ok(h) = per_file[i].1.as_deref() else { continue };
        let key = |j: usize| original_rank(&files[j].rel, per_file[j].0.fs_modified);
        primary
            .entry(h)
            .and_modify(|p| {
                if key(i) < key(*p) {
                    *p = i
                }
            })
            .or_insert(i);
    }
    let mut items = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        let hash = per_file[i].1.as_ref().ok().cloned();
        let duplicate_of =
            hash.as_deref().and_then(|h| primary.get(h)).filter(|&&p| p != i).map(|&p| files[p].rel.clone());
        let (decision, evidence, mut flags) = match outcome.remove(&i) {
            Some(o) => (o.decision, o.evidence, o.flags),
            None => (Decision::Skip { reason: "operating-system clutter file".into() }, vec![], vec![]),
        };
        if f.ext_mismatch {
            flags.push(format!("extension_mismatch: content is {}", f.format.name()));
        }
        if let Err(e) = &per_file[i].1
            && f.kind != MediaKind::Junk
        {
            flags.push(format!("hash failed: {e}"));
        }
        items.push(ItemReport {
            rel_path: f.rel.clone(),
            kind: f.kind,
            format: f.format.name().into(),
            size: f.size,
            group: f.group.clone(),
            hash,
            duplicate_of,
            decision,
            evidence,
            flags,
        });
    }
    Ok(Analysis { items, errors: scan.errors, skipped_links: scan.skipped_links })
}
