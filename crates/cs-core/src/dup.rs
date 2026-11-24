//! Duplicate finder. Read-only: nothing is changed here.
//!
//! * **Identical copies:** same bytes = same BLAKE3 hash. Fast path: only files
//!   that share a size can be identical, so files with a unique size are never read.
//! * **Near-duplicates** (optional, photos only): the same picture resized or saved
//!   again. Every photo gets a 128-bit picture fingerprint (horizontal + vertical
//!   difference hash of a small upright grayscale thumbnail the caller supplies);
//!   two photos match when the fingerprints differ in at most
//!   [`SIMILAR_MAX_DISTANCE`] bits AND their shapes (aspect ratios) agree.
//!   Every file in such a set carries its own content hash, so acting on it can
//!   check each file is still exactly what the user compared.

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
    /// File size in bytes.
    pub size: u64,
    /// BLAKE3 of this file (always set for near-duplicate sets).
    pub hash: Option<String>,
    /// Picture size in pixels, when known (near-duplicate sets).
    pub pixels: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DupGroup {
    /// Identical sets: the shared BLAKE3. Near-duplicate sets: a stable id ("similar-…").
    pub hash: String,
    /// Identical sets: the size of each copy. Near-duplicate sets: the largest file.
    pub size: u64,
    pub files: Vec<DupFile>,
    /// Suggested copy to keep (index into `files`).
    pub keep: usize,
    /// Near-duplicates: the same picture, but NOT the same bytes.
    pub similar: bool,
}

impl DupGroup {
    /// Space freed if all but the suggested copy are removed.
    pub fn reclaimable(&self) -> u64 {
        if self.similar {
            self.files.iter().enumerate().filter(|(i, _)| *i != self.keep).map(|(_, f)| f.size).sum()
        } else {
            self.size * (self.files.len() as u64 - 1)
        }
    }
}

/// Most fingerprint bits (of 128) two photos may differ in to count as the same picture.
/// Resized or re-saved copies differ in 0–6; different photos of the same scene in 15+.
pub const SIMILAR_MAX_DISTANCE: u32 = 10;
/// Shapes must agree this closely (width / height), so crops are not matched.
const SIMILAR_MAX_ASPECT_DIFF: f64 = 0.02;

/// A small upright grayscale picture (any size from about 16 px up), row by row.
#[derive(Debug, Clone)]
pub struct Gray {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

/// What the caller can tell about one photo for near-duplicate matching.
#[derive(Debug, Clone)]
pub struct Look {
    /// Upright small grayscale version (e.g. from the thumbnail cache).
    pub gray: Gray,
    /// Full picture size in pixels, if known (used to suggest the copy to keep).
    pub pixels: Option<(u32, u32)>,
}

/// Picture fingerprint: 64-bit horizontal + 64-bit vertical difference hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Print {
    pub bits: u128,
    /// Width / height of the upright picture, ×10 000.
    pub aspect: u32,
}

impl Print {
    pub fn distance(&self, o: &Print) -> u32 {
        (self.bits ^ o.bits).count_ones()
    }
    pub fn same_picture(&self, o: &Print) -> bool {
        let (a, b) = (f64::from(self.aspect), f64::from(o.aspect));
        (a - b).abs() / a.max(b) <= SIMILAR_MAX_ASPECT_DIFF && self.distance(o) <= SIMILAR_MAX_DISTANCE
    }
}

/// Area-average `g` down to `tw` × `th`.
fn shrink(g: &Gray, tw: u32, th: u32) -> Vec<f32> {
    let (w, h) = (g.w as usize, g.h as usize);
    let mut out = Vec::with_capacity((tw * th) as usize);
    for ty in 0..th as usize {
        let y0 = ty * h / th as usize;
        let y1 = ((ty + 1) * h / th as usize).max(y0 + 1).min(h);
        for tx in 0..tw as usize {
            let x0 = tx * w / tw as usize;
            let x1 = ((tx + 1) * w / tw as usize).max(x0 + 1).min(w);
            let mut sum = 0u32;
            for y in y0..y1 {
                sum += g.px[y * w + x0..y * w + x1].iter().map(|&v| u32::from(v)).sum::<u32>();
            }
            out.push(sum as f32 / ((y1 - y0) * (x1 - x0)) as f32);
        }
    }
    out
}

/// Fingerprint of a picture, or `None` when it is too small or too plain to compare
/// (an all-black frame would otherwise "match" every other dark frame).
pub fn fingerprint(g: &Gray) -> Option<Print> {
    if g.w < 9 || g.h < 9 || g.px.len() != (g.w as usize) * (g.h as usize) {
        return None;
    }
    let hz = shrink(g, 9, 8);
    let vt = shrink(g, 8, 9);
    let mean = hz.iter().sum::<f32>() / hz.len() as f32;
    let spread = (hz.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / hz.len() as f32).sqrt();
    if spread < 2.0 {
        return None;
    }
    let mut bits = 0u128;
    for y in 0..8 {
        for x in 0..8 {
            bits = (bits << 1) | u128::from(hz[y * 9 + x] < hz[y * 9 + x + 1]);
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            bits = (bits << 1) | u128::from(vt[y * 8 + x] < vt[(y + 1) * 8 + x]);
        }
    }
    let aspect = (f64::from(g.w) / f64::from(g.h) * 10_000.0).round() as u32;
    Some(Print { bits, aspect })
}

/// The caller's way to look at a photo (file, its BLAKE3); `None` = cannot be read.
pub type LookFn<'a> = &'a (dyn Fn(&DupFile, &str) -> Option<Look> + Sync);

#[derive(Debug, Default, Serialize)]
pub struct DupScan {
    pub roots: Vec<PathBuf>,
    pub groups: Vec<DupGroup>,
    pub files_scanned: usize,
    pub files_hashed: usize,
    pub bytes_reclaimable: u64,
    pub errors: Vec<(String, String)>,
    /// Near-duplicate sets found (they are also in `groups`, with `similar` set).
    pub similar_groups: usize,
    /// Photos that could not be compared for near-duplicates (unreadable, too plain).
    pub not_compared: usize,
}

/// Find identical files in one or more folders. `include_all` = every file type,
/// otherwise only photos and videos.
pub fn find_duplicates(
    roots: &[PathBuf],
    include_all: bool,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> io::Result<DupScan> {
    find_duplicates_ext(roots, include_all, None, progress)
}

/// Like [`find_duplicates`]; with `look`, also finds near-duplicate photos
/// (every photo is then read once for its content hash, and looked at once).
pub fn find_duplicates_ext(
    roots: &[PathBuf],
    include_all: bool,
    look: Option<LookFn<'_>>,
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
                DupFile {
                    abs: f.abs,
                    root: ri,
                    rel: f.rel,
                    mtime,
                    kind: f.kind,
                    format: f.format.name().into(),
                    size: f.size,
                    hash: None,
                    pixels: None,
                },
                f.size,
            ));
        }
    }
    scan.files_scanned = files.len();

    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, (_, size)) in files.iter().enumerate() {
        by_size.entry(*size).or_default().push(i);
    }
    let mut candidates: Vec<usize> = by_size.values().filter(|v| v.len() > 1).flatten().copied().collect();
    if look.is_some() {
        // Near-duplicates need every photo's hash (to check it again before acting).
        let have: HashSet<usize> = candidates.iter().copied().collect();
        candidates.extend((0..files.len()).filter(|i| files[*i].0.kind == MediaKind::Image && !have.contains(i)));
    }
    let photos = if look.is_some() { files.iter().filter(|f| f.0.kind == MediaKind::Image).count() } else { 0 };
    // One progress bar over both steps: hashing, then looking at the photos.
    let total = candidates.len() + photos;
    scan.files_hashed = candidates.len();
    let done = AtomicUsize::new(0);
    let tick = || {
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_multiple_of(25) || n == total {
            progress(n, total);
        }
    };
    progress(0, total);
    let hashed: Vec<(usize, io::Result<String>)> = candidates
        .par_iter()
        .map(|&i| {
            let h = crate::hash_file(&files[i].0.abs);
            tick();
            (i, h)
        })
        .collect();

    let mut by_hash: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, h) in hashed {
        match h {
            Ok(h) => {
                files[i].0.hash = Some(h.clone());
                by_hash.entry(h).or_default().push(i);
            }
            Err(e) => scan.errors.push((files[i].0.abs.display().to_string(), e.to_string())),
        }
    }
    let rank = |f: &DupFile| crate::original_rank(&f.abs.to_string_lossy(), f.mtime);
    // Per content: the copy suggested to keep (the representative for near-duplicates).
    let mut rep_of: BTreeMap<String, usize> = BTreeMap::new();
    for (hash, idx) in &by_hash {
        let best = *idx.iter().min_by_key(|&&i| rank(&files[i].0)).expect("non-empty");
        rep_of.insert(hash.clone(), best);
    }
    for (hash, mut idx) in by_hash.into_iter().filter(|(_, v)| v.len() > 1) {
        idx.sort_by(|a, b| files[*a].0.abs.cmp(&files[*b].0.abs));
        let size = files[idx[0]].1;
        let group_files: Vec<DupFile> = idx.iter().map(|&i| files[i].0.clone()).collect();
        let keep = (0..group_files.len()).min_by_key(|&k| rank(&group_files[k])).unwrap_or(0);
        scan.groups.push(DupGroup { hash, size, files: group_files, keep, similar: false });
    }

    if let Some(look) = look {
        // Only one file per distinct content: identical copies are handled above.
        let reps: Vec<(String, usize)> = rep_of
            .iter()
            .filter(|(_, i)| files[**i].0.kind == MediaKind::Image)
            .map(|(h, i)| (h.clone(), *i))
            .collect();
        // Photos that could not be hashed still count for the progress bar.
        for _ in reps.len()..photos {
            tick();
        }
        type Seen = (usize, Option<(Print, Option<(u32, u32)>)>);
        let looked: Vec<Seen> = reps
            .par_iter()
            .map(|(h, i)| {
                let l = look(&files[*i].0, h);
                tick();
                (*i, l.and_then(|l| fingerprint(&l.gray).map(|p| (p, l.pixels))))
            })
            .collect();
        let mut prints: Vec<(usize, Print)> = vec![];
        for (i, l) in looked {
            match l {
                Some((p, px)) => {
                    files[i].0.pixels = px;
                    prints.push((i, p));
                }
                None => scan.not_compared += 1,
            }
        }
        for set in similar_sets(&prints) {
            let mut group_files: Vec<DupFile> = set.iter().map(|&i| files[i].0.clone()).collect();
            group_files.sort_by(|a, b| a.abs.cmp(&b.abs));
            // Keep the copy with the most pixels, then the biggest file, then the "original" place.
            let keep = (0..group_files.len())
                .min_by_key(|&k| {
                    let f = &group_files[k];
                    let px = f.pixels.map_or(0, |(w, h)| u64::from(w) * u64::from(h));
                    (std::cmp::Reverse(px), std::cmp::Reverse(f.size), rank(f))
                })
                .unwrap_or(0);
            let mut id = blake3::Hasher::new();
            for f in &group_files {
                id.update(f.hash.as_deref().unwrap_or_default().as_bytes());
            }
            let size = group_files.iter().map(|f| f.size).max().unwrap_or(0);
            let hash = format!("similar-{}", &id.finalize().to_hex()[..32]);
            scan.groups.push(DupGroup { hash, size, files: group_files, keep, similar: true });
            scan.similar_groups += 1;
        }
    }
    scan.groups.sort_by(|a, b| b.reclaimable().cmp(&a.reclaimable()).then(a.hash.cmp(&b.hash)));
    scan.bytes_reclaimable = scan.groups.iter().filter(|g| !g.similar).map(DupGroup::reclaimable).sum();
    Ok(scan)
}

/// Sets (of file indexes) whose fingerprints say "same picture". A photo joins a
/// set only if it matches EVERY photo already in it, so a chain of small
/// differences (A≈B, B≈C) never pulls in a photo that does not match A.
fn similar_sets(prints: &[(usize, Print)]) -> Vec<Vec<usize>> {
    let n = prints.len();
    let mut set_of: Vec<Option<usize>> = vec![None; n];
    let mut sets: Vec<Vec<usize>> = vec![]; // positions into `prints`
    for a in 0..n {
        for b in (a + 1)..n {
            if !prints[a].1.same_picture(&prints[b].1) {
                continue;
            }
            match (set_of[a], set_of[b]) {
                (None, None) => {
                    set_of[a] = Some(sets.len());
                    set_of[b] = Some(sets.len());
                    sets.push(vec![a, b]);
                }
                (Some(s), None) | (None, Some(s)) => {
                    let new = if set_of[a].is_none() { a } else { b };
                    if sets[s].iter().all(|&m| prints[m].1.same_picture(&prints[new].1)) {
                        set_of[new] = Some(s);
                        sets[s].push(new);
                    }
                }
                (Some(_), Some(_)) => {}
            }
        }
    }
    sets.into_iter().map(|s| s.into_iter().map(|p| prints[p].0).collect()).collect()
}

/// Write the result as CSV (opens in Excel / LibreOffice). Never overwrites.
pub fn write_csv(scan: &DupScan, path: &Path) -> io::Result<()> {
    use std::io::Write;
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut f = std::fs::File::options().write(true).create_new(true).open(path)?;
    // BOM: Excel reads UTF-8 names
    f.write_all("\u{feff}group,kind,keep,size_bytes,pixels,path,blake3\r\n".as_bytes())?;
    for (gi, g) in scan.groups.iter().enumerate() {
        for (fi, file) in g.files.iter().enumerate() {
            let line = format!(
                "{},{},{},{},{},{},{}\r\n",
                gi + 1,
                if g.similar { "similar" } else { "identical" },
                if fi == g.keep { "keep" } else { "extra" },
                file.size,
                file.pixels.map(|(w, h)| format!("{w}x{h}")).unwrap_or_default(),
                q(&file.abs.display().to_string()),
                file.hash.as_deref().unwrap_or(&g.hash)
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

    /// A smooth synthetic "photo" (same picture at any size): `seed` picks the scene,
    /// `shift` moves it (a different shot of the same scene), `noise` adds JPEG-like grain.
    fn scene(seed: u32, w: u32, h: u32, shift: f32, noise: u8) -> Gray {
        let mut px = Vec::with_capacity((w * h) as usize);
        let s = seed as f32;
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 / w as f32 + shift, y as f32 / h as f32);
                let val = 128.0
                    + 70.0
                        * (u * (3.0 + s) * std::f32::consts::TAU + s).sin()
                        * (v * (2.0 + s * 0.5) * std::f32::consts::TAU).cos()
                    + 40.0 * ((u + v) * (5.0 + s) * std::f32::consts::PI).sin();
                let grain =
                    if noise == 0 { 0.0 } else { (((x * 31 + y * 17 + seed) % 7) as f32 - 3.0) * f32::from(noise) };
                px.push((val + grain).clamp(0.0, 255.0) as u8);
            }
        }
        Gray { w, h, px }
    }

    #[test]
    fn fingerprint_matches_resized_and_resaved_copies_only() {
        let orig = fingerprint(&scene(1, 400, 300, 0.0, 0)).unwrap();
        let small = fingerprint(&scene(1, 120, 90, 0.0, 0)).unwrap();
        let grainy = fingerprint(&scene(1, 400, 300, 0.0, 2)).unwrap();
        assert!(orig.same_picture(&small), "resized: distance {}", orig.distance(&small));
        assert!(orig.same_picture(&grainy), "re-saved: distance {}", orig.distance(&grainy));
        let other = fingerprint(&scene(2, 400, 300, 0.0, 0)).unwrap();
        assert!(!orig.same_picture(&other), "different photo: distance {}", orig.distance(&other));
        let moved = fingerprint(&scene(1, 400, 300, 0.25, 0)).unwrap();
        assert!(!orig.same_picture(&moved), "another shot of the scene: distance {}", orig.distance(&moved));
        // Same content squeezed into another shape (a crop / stretch) is not "the same photo".
        let squeezed = fingerprint(&scene(1, 400, 200, 0.0, 0)).unwrap();
        assert!(!orig.same_picture(&squeezed));
        // Too plain or too small to compare.
        assert!(fingerprint(&Gray { w: 64, h: 64, px: vec![12; 64 * 64] }).is_none());
        assert!(fingerprint(&Gray { w: 4, h: 4, px: vec![0; 16] }).is_none());
        assert!(fingerprint(&Gray { w: 64, h: 64, px: vec![0; 10] }).is_none(), "bad buffer");
    }

    #[test]
    fn a_set_only_takes_photos_that_match_all_members() {
        let p = |bits: u128| Print { bits, aspect: 15_000 };
        // B is 8 bits from A, C is 8 bits from B but 16 from A.
        let (a, b, c) = (p(0), p(0xFF), p(0xFFFF));
        let sets = similar_sets(&[(10, a), (11, b), (12, c)]);
        assert_eq!(sets, vec![vec![10, 11]]);
    }

    #[test]
    fn finds_near_duplicates_with_their_own_hashes() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Photos");
        std::fs::create_dir_all(dir.join("Backup")).unwrap();
        // Fake JPEGs: the bytes after the JPEG marker describe the picture.
        let pic = |name: &str, desc: &str| {
            let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
            v.extend_from_slice(desc.as_bytes());
            std::fs::write(dir.join(name), v).unwrap();
        };
        pic("beach.jpg", "1 400 300 0"); // original
        pic("Backup/beach_small.jpg", "1 200 150 0"); // resized copy
        pic("Backup/beach_resaved.jpg", "1 400 300 2"); // saved again
        pic("Backup/beach (1).jpg", "1 400 300 0"); // identical copy of the original
        pic("mountain.jpg", "2 400 300 0"); // another photo
        pic("blank.jpg", "plain"); // too plain to compare
        let look: LookFn = &|f: &DupFile, _h: &str| {
            let b = std::fs::read(&f.abs).ok()?;
            let s = String::from_utf8_lossy(&b[4..]).to_string();
            let n: Vec<u32> = s.split(' ').filter_map(|x| x.parse().ok()).collect();
            if n.len() != 4 {
                return Some(Look { gray: Gray { w: 32, h: 32, px: vec![9; 1024] }, pixels: None });
            }
            // Like the app: the picture at its own size, then a thumbnail (160 px wide here).
            let full = scene(n[0], n[1], n[2], 0.0, n[3] as u8);
            let (tw, th) = (160, 160 * n[2] / n[1]);
            let px = shrink(&full, tw, th).into_iter().map(|v| v.round() as u8).collect();
            Some(Look { gray: Gray { w: tw, h: th, px }, pixels: Some((n[1], n[2])) })
        };
        let s = find_duplicates_ext(std::slice::from_ref(&dir), false, Some(look), &|_, _| {}).unwrap();
        let exact: Vec<_> = s.groups.iter().filter(|g| !g.similar).collect();
        let similar: Vec<_> = s.groups.iter().filter(|g| g.similar).collect();
        assert_eq!(exact.len(), 1, "beach.jpg + beach (1).jpg");
        assert_eq!(similar.len(), 1);
        assert_eq!(s.similar_groups, 1);
        assert_eq!(s.not_compared, 1, "the plain picture");
        let g = similar[0];
        let names: Vec<String> = g.files.iter().map(|f| f.rel.clone()).collect();
        assert_eq!(g.files.len(), 3, "{names:?}");
        assert!(!names.iter().any(|n| n.contains("(1)")), "identical copies are not repeated: {names:?}");
        assert!(!names.iter().any(|n| n.contains("mountain")));
        assert_eq!(g.files[g.keep].rel, "beach.jpg", "most pixels, original place");
        assert!(g.files.iter().all(|f| f.hash.is_some()), "every file has its own hash");
        assert!(g.hash.starts_with("similar-"));
        assert_eq!(
            g.reclaimable(),
            g.files.iter().enumerate().filter(|(i, _)| *i != g.keep).map(|(_, f)| f.size).sum::<u64>()
        );
        // Without `look`, only the identical copy is found (old behaviour).
        let plain = find_duplicates(&[dir], false, &|_, _| {}).unwrap();
        assert_eq!((plain.groups.len(), plain.similar_groups), (1, 0));
    }
}
