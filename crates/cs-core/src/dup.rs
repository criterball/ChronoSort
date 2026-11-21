//! Exact-duplicate finder: same bytes = same BLAKE3 hash.
//!
//! Fast path: only files that share a size can be identical, so files with a
//! unique size are never read. Read-only: nothing is changed here.

use cs_model::MediaKind;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, Serialize)]
pub struct DupFile {
    pub abs: PathBuf,
    /// Which of the scanned folders it is in (index into `roots`).
    pub root: usize,
    /// Path relative to its root, `/`-separated.
    pub rel: String,
    pub mtime: Option<i64>,
    pub kind: MediaKind,
    pub format: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DupGroup {
    pub hash: String,
    pub size: u64,
    pub files: Vec<DupFile>,
    /// Suggested copy to keep (index into `files`).
    pub keep: usize,
}

impl DupGroup {
    /// Space freed if all but one copy are removed.
    pub fn reclaimable(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

#[derive(Debug, Default, Serialize)]
pub struct DupScan {
    pub roots: Vec<PathBuf>,
    pub groups: Vec<DupGroup>,
    pub files_scanned: usize,
    pub files_hashed: usize,
    pub bytes_reclaimable: u64,
    pub errors: Vec<(String, String)>,
}

/// Find identical files in one or more folders. `include_all` = every file type,
/// otherwise only photos and videos.
pub fn find_duplicates(
    roots: &[PathBuf],
    include_all: bool,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> io::Result<DupScan> {
    let mut scan = DupScan { roots: roots.to_vec(), ..Default::default() };
    let mut seen = HashSet::new();
    let mut files: Vec<(DupFile, u64)> = Vec::new();
    for (ri, root) in roots.iter().enumerate() {
        let r = cs_scan::scan(root)?;
        scan.errors.extend(r.errors.into_iter().map(|(p, e)| (p.display().to_string(), e)));
        for f in r.files {
            let wanted = match f.kind {
                MediaKind::Image | MediaKind::Video => true,
                MediaKind::Junk => false,
                _ => include_all,
            };
            // Overlapping folders: never count the same file twice.
            let key = std::fs::canonicalize(&f.abs).unwrap_or_else(|_| f.abs.clone());
            if !wanted || f.size == 0 || !seen.insert(key) {
                continue;
            }
            let mtime = std::fs::metadata(&f.abs).and_then(|m| m.modified()).ok().map(cs_model::civil::system_to_unix);
            files.push((
                DupFile { abs: f.abs, root: ri, rel: f.rel, mtime, kind: f.kind, format: f.format.name().into() },
                f.size,
            ));
        }
    }
    scan.files_scanned = files.len();

    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, (_, size)) in files.iter().enumerate() {
        by_size.entry(*size).or_default().push(i);
    }
    let candidates: Vec<usize> = by_size.values().filter(|v| v.len() > 1).flatten().copied().collect();
    let total = candidates.len();
    scan.files_hashed = total;
    let done = AtomicUsize::new(0);
    progress(0, total);
    let hashed: Vec<(usize, io::Result<String>)> = candidates
        .par_iter()
        .map(|&i| {
            let h = crate::hash_file(&files[i].0.abs);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_multiple_of(25) || n == total {
                progress(n, total);
            }
            (i, h)
        })
        .collect();

    let mut by_hash: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, h) in hashed {
        match h {
            Ok(h) => by_hash.entry(h).or_default().push(i),
            Err(e) => scan.errors.push((files[i].0.abs.display().to_string(), e.to_string())),
        }
    }
    for (hash, mut idx) in by_hash.into_iter().filter(|(_, v)| v.len() > 1) {
        idx.sort_by(|a, b| files[*a].0.abs.cmp(&files[*b].0.abs));
        let size = files[idx[0]].1;
        let group_files: Vec<DupFile> = idx.iter().map(|&i| files[i].0.clone()).collect();
        let keep = (0..group_files.len())
            .min_by_key(|&k| crate::original_rank(&group_files[k].abs.to_string_lossy(), group_files[k].mtime))
            .unwrap_or(0);
        scan.groups.push(DupGroup { hash, size, files: group_files, keep });
    }
    scan.groups.sort_by(|a, b| b.reclaimable().cmp(&a.reclaimable()).then(a.hash.cmp(&b.hash)));
    scan.bytes_reclaimable = scan.groups.iter().map(DupGroup::reclaimable).sum();
    Ok(scan)
}

/// Write the result as CSV (opens in Excel / LibreOffice). Never overwrites.
pub fn write_csv(scan: &DupScan, path: &Path) -> io::Result<()> {
    use std::io::Write;
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut f = std::fs::File::options().write(true).create_new(true).open(path)?;
    f.write_all("\u{feff}group,keep,size_bytes,path,blake3\r\n".as_bytes())?; // BOM: Excel reads UTF-8 names
    for (gi, g) in scan.groups.iter().enumerate() {
        for (fi, file) in g.files.iter().enumerate() {
            let line = format!(
                "{},{},{},{},{}\r\n",
                gi + 1,
                if fi == g.keep { "keep" } else { "extra" },
                g.size,
                q(&file.abs.display().to_string()),
                g.hash
            );
            f.write_all(line.as_bytes())?;
        }
    }
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_groups_and_suggests_the_original() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("Photos");
        let b = d.path().join("Backup copy");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let jpg = |n: u8| {
            let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
            v.extend(std::iter::repeat_n(n, 100));
            v
        };
        std::fs::write(a.join("one.jpg"), jpg(1)).unwrap();
        std::fs::write(b.join("one.jpg"), jpg(1)).unwrap();
        std::fs::write(b.join("one (1).jpg"), jpg(1)).unwrap();
        std::fs::write(a.join("two.jpg"), jpg(2)).unwrap(); // same size, different content
        std::fs::write(a.join("notes.txt"), b"hello").unwrap();
        std::fs::write(b.join("notes.txt"), b"hello").unwrap();

        let s = find_duplicates(&[a.clone(), b.clone()], false, &|_, _| {}).unwrap();
        assert_eq!(s.groups.len(), 1, "text files are ignored unless include_all");
        let g = &s.groups[0];
        assert_eq!(g.files.len(), 3);
        assert_eq!(g.files[g.keep].abs, a.join("one.jpg"), "the copy outside 'Backup copy' is kept");
        assert_eq!(s.bytes_reclaimable, 2 * 104);

        let all = find_duplicates(&[a.clone(), b], true, &|_, _| {}).unwrap();
        assert_eq!(all.groups.len(), 2);

        // Same folder twice (or nested) never reports a file as its own duplicate.
        let twice = find_duplicates(&[a.clone(), a], false, &|_, _| {}).unwrap();
        assert!(twice.groups.is_empty());

        let csv = d.path().join("report.csv");
        write_csv(&s, &csv).unwrap();
        assert!(std::fs::read_to_string(&csv).unwrap().contains("keep"));
        assert!(write_csv(&s, &csv).is_err(), "never overwrites");
    }
}
