//! Star ratings (written INTO the photo, the user's choice 2026-11-25) and
//! the "tag" mark (kept in ChronoSort's own list, files untouched).
//!
//! Ratings use the standard tags other programs show: XMP `xmp:Rating` plus
//! EXIF Rating / RatingPercent (Windows Explorer's stars). Each file is backed
//! up (verified copy) into `<folder>/.chronosort/info-backups/<run>/` first,
//! its dates are kept (-P), and one Ctrl+Z undoes the whole batch.

use crate::fileops::Progress;
use crate::fileops::{self, Op, Undo};
use crate::{Res, err};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Stars of a photo (0–5); `None` when ChronoSort cannot read ratings from this kind of file.
pub fn stars_of(p: &Path) -> Option<u8> {
    let fmt = crate::real_format(p);
    (fmt.kind() == cs_model::MediaKind::Image).then(|| cs_meta::rating::rating(p, fmt).unwrap_or(0))
}

#[derive(Serialize, Default)]
pub struct RatingReport {
    pub done: usize,
    /// Already had these stars: nothing written.
    pub unchanged: usize,
    pub failed: Vec<(String, String)>,
}

/// ExifTool arguments for `stars` (0 removes the rating). Windows keeps a
/// percentage too: 1 = 1, 2 = 25, 3 = 50, 4 = 75, 5 = 99.
pub fn rating_args(stars: u8) -> Res<Vec<String>> {
    if stars > 5 {
        return Err("A rating is 0 to 5 stars.".into());
    }
    let mut a: Vec<String> = ["-charset", "filename=utf8", "-overwrite_original", "-P"].map(String::from).to_vec();
    if stars == 0 {
        a.extend(["-XMP-xmp:Rating=", "-EXIF:Rating=", "-EXIF:RatingPercent="].map(String::from));
    } else {
        let pct = [0, 1, 25, 50, 75, 99][stars as usize];
        a.extend([
            format!("-XMP-xmp:Rating={stars}"),
            format!("-EXIF:Rating={stars}"),
            format!("-EXIF:RatingPercent={pct}"),
        ]);
    }
    // Windows' own XMP percentage would disagree with the new stars: remove it.
    a.push("-XMP-microsoft:RatingPercent=".into());
    Ok(a)
}

/// Give `stars` (0 = none) to the photos. Blocking.
pub fn rate_files(files: &[PathBuf], stars: u8, emit: impl Fn(Progress)) -> Res<RatingReport> {
    let args = rating_args(stars)?;
    if files.is_empty() {
        return Err("Select one or more photos first.".into());
    }
    let mut et = cs_geo::exiftool::ExifTool::start(&crate::exiftool::path()?).map_err(err)?;
    let run = format!("{}", crate::now_unix());
    let mut rep = RatingReport::default();
    let mut undo = Vec::new();
    let total = files.len() as u64;
    for (i, file) in files.iter().enumerate() {
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        emit(Progress {
            files_done: i as u64,
            files_total: total,
            bytes_done: 0,
            bytes_total: 0,
            current: name.clone(),
        });
        match rate_one(&mut et, file, &args, stars, &run) {
            Ok(Some(u)) => {
                rep.done += 1;
                undo.push(u);
            }
            Ok(None) => rep.unchanged += 1,
            Err(e) => rep.failed.push((name, e)),
        }
    }
    emit(Progress { files_done: total, files_total: total, bytes_done: 0, bytes_total: 0, current: String::new() });
    if !undo.is_empty() {
        let n = undo.len();
        let what = if stars == 0 { "Remove the stars from".to_string() } else { format!("Give {stars} ★ to") };
        fileops::push(Op {
            label: format!("{what} {n} {}", if n == 1 { "photo" } else { "photos" }),
            undo: Undo::MetaEditedMany(undo),
        });
    }
    Ok(rep)
}

/// One file: `None` when it already has these stars.
fn rate_one(
    et: &mut cs_geo::exiftool::ExifTool,
    file: &Path,
    args: &[String],
    stars: u8,
    run: &str,
) -> Res<Option<(PathBuf, PathBuf, String)>> {
    fileops::guard(file)?;
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !crate::meta::WRITABLE.contains(&ext.as_str()) {
        return Err("ChronoSort cannot write into this kind of file.".into());
    }
    let fmt = crate::real_format(file);
    if cs_meta::rating::rating(file, fmt).unwrap_or(0) == stars {
        return Ok(None);
    }
    let dir = file.parent().ok_or("no folder")?;
    let name = file.file_name().ok_or("no name")?.to_string_lossy().into_owned();
    let bdir = dir.join(".chronosort").join("info-backups").join(run);
    let saved = fileops::free_name(&bdir, &name);
    let before = crate::meta::backup(file, &saved)?;
    let mut a = args.to_vec();
    a.push(file.to_string_lossy().into_owned());
    let out = et.run(&a).map_err(err);
    let ok = matches!(&out, Ok(l) if l.iter().any(|x| x.contains("1 image files updated")));
    if !ok {
        if cs_core::hash_file(file).ok().as_deref() != Some(before.as_str()) {
            let _ = std::fs::copy(&saved, file);
        }
        let _ = std::fs::remove_file(&saved);
        return Err(match out {
            Ok(l) => l
                .into_iter()
                .find(|x| x.contains("Error") || x.contains("Warning"))
                .unwrap_or_else(|| "nothing was written".into()),
            Err(e) => e,
        });
    }
    let after = cs_core::hash_file(file).map_err(err)?;
    Ok(Some((file.to_path_buf(), saved, after)))
}

// ---------- the "tag" mark: ChronoSort's own list ----------

static TAGS: Mutex<Option<BTreeSet<String>>> = Mutex::new(None);

fn key(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

fn tags_file(data: &Path) -> PathBuf {
    data.join("tagged.json")
}

/// The tagged files (ChronoSort's own list in `<data>/tagged.json`; the files are not touched).
pub fn tagged_set(data: &Path) -> Res<BTreeSet<String>> {
    let mut g = TAGS.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let f = tags_file(data);
        *g = Some(match std::fs::read(&f) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
            Err(_) => BTreeSet::new(),
        });
    }
    Ok(g.clone().unwrap_or_default())
}

/// Tag or untag files. Returns how many are tagged in the whole list afterwards.
pub fn is_tagged(set: &BTreeSet<String>, p: &Path) -> bool {
    set.contains(&key(p))
}

/// Tag or untag files. Returns how many are tagged in the whole list afterwards.
pub fn set_tagged(data: &Path, files: &[PathBuf], on: bool) -> Res<usize> {
    let mut set = tagged_set(data)?;
    for p in files {
        if on {
            set.insert(key(p));
        } else {
            set.remove(&key(p));
        }
    }
    let f = tags_file(data);
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d).map_err(err)?;
    }
    // Write a new file, then swap it in, so a crash never leaves half a list.
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&set).map_err(err)?).map_err(err)?;
    std::fs::rename(&tmp, &f).map_err(err)?;
    let n = set.len();
    *TAGS.lock().unwrap_or_else(|e| e.into_inner()) = Some(set);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let a = rating_args(4).unwrap();
        assert!(a.contains(&"-XMP-xmp:Rating=4".to_string()));
        assert!(a.contains(&"-EXIF:RatingPercent=75".to_string()));
        assert!(rating_args(0).unwrap().contains(&"-EXIF:Rating=".to_string()));
        assert!(rating_args(6).is_err());
    }

    /// Real ExifTool: rate, read back with our own reader, rate again (no-op), undo.
    #[test]
    fn rate_read_back_and_undo() {
        let _l = fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("photo.jpg");
        let img = image::RgbImage::from_pixel(40, 30, image::Rgb([120, 140, 160]));
        img.save(&p).unwrap();
        let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
        let original = std::fs::read(&p).unwrap();
        assert_eq!(cs_meta::rating::rating(&p, cs_scan::Format::Jpeg), None);

        let files = vec![p.clone()];
        let rep = rate_files(&files, 3, |_| {}).unwrap();
        assert_eq!((rep.done, rep.unchanged, rep.failed.len()), (1, 0, 0), "{:?}", rep.failed);
        assert_eq!(cs_meta::rating::rating(&p, cs_scan::Format::Jpeg), Some(3));
        // ExifTool keeps the date to the microsecond (Windows stores 100 ns steps).
        let now = std::fs::metadata(&p).unwrap().modified().unwrap();
        let diff = now.duration_since(mtime).unwrap_or_else(|e| e.duration());
        assert!(diff < std::time::Duration::from_millis(1), "dates kept: {diff:?}");
        let info = crate::meta::read(&p).unwrap();
        let tag = |n: &str| info.tags.iter().find(|t| t.name == n).map(|t| t.value.clone());
        assert_eq!(tag("Rating").as_deref(), Some("3"));
        assert_eq!(tag("RatingPercent").as_deref(), Some("50"));
        let rated = std::fs::read(&p).unwrap();

        // Same stars again: nothing written, no extra undo step.
        let rep = rate_files(&files, 3, |_| {}).unwrap();
        assert_eq!((rep.done, rep.unchanged), (0, 1));
        assert_eq!(std::fs::read(&p).unwrap(), rated);

        // Undo puts the exact bytes back.
        let u = fileops::take_last_for_test();
        assert!(matches!(u, Undo::MetaEditedMany(ref v) if v.len() == 1));
        fileops::undo_for_test(u).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), original);
    }
}
