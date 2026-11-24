//! `cs-geo`: add a place to photos, entirely offline.
//!
//! * Places come from ExifTool's built-in GeoNames city list (no internet).
//! * A city writes GPS coordinates plus the place names; "country only" writes
//!   just the country name, never made-up coordinates.
//! * Every photo is backed up (verified copy) before it is changed, into
//!   `<folder>/.chronosort/place-backups/<run>/`, with a manifest, so the whole
//!   run can be undone. Photos that already have a location are skipped unless
//!   the user asks to replace it.

pub mod exiftool;
pub mod places;

pub use places::{Country, Place, Places, Region, fold, parse_listgeo};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What to write.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    City {
        place: Place,
    },
    /// A general location: region + country, no town and no map position.
    Region {
        name: String,
        country: String,
        code: String,
    },
    Country {
        name: String,
        code: String,
    },
}

impl Target {
    pub fn label(&self) -> String {
        match self {
            Target::City { place } => place.label(),
            Target::Region { name, country, .. } => format!("{name}, {country}"),
            Target::Country { name, .. } => name.clone(),
        }
    }
}

/// ExifTool arguments for one file (one per line; no shell involved).
pub fn write_args(target: &Target, file: &Path, video: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-charset", "filename=utf8", "-overwrite_original", "-P"].map(String::from).to_vec();
    match target {
        Target::City { place: p } => {
            let (lat, lon) = (format!("{:.6}", p.lat.abs()), format!("{:.6}", p.lon.abs()));
            let (ns, ew) = (if p.lat >= 0.0 { "N" } else { "S" }, if p.lon >= 0.0 { "E" } else { "W" });
            if video {
                a.push(format!("-Keys:GPSCoordinates={:.6}, {:.6}", p.lat, p.lon));
                a.push(format!("-UserData:GPSCoordinates={:.6}, {:.6}", p.lat, p.lon));
            } else {
                a.extend([
                    format!("-EXIF:GPSLatitude={lat}"),
                    format!("-EXIF:GPSLatitudeRef={ns}"),
                    format!("-EXIF:GPSLongitude={lon}"),
                    format!("-EXIF:GPSLongitudeRef={ew}"),
                ]);
            }
            a.push(format!("-XMP-photoshop:City={}", p.city));
            if !p.region.is_empty() {
                a.push(format!("-XMP-photoshop:State={}", p.region));
            }
            a.push(format!("-XMP-photoshop:Country={}", p.country));
            a.push(format!("-XMP-iptcCore:CountryCode={}", p.country_code));
        }
        Target::Region { name, country, code } => {
            clear_finer(&mut a, video, false);
            a.push(format!("-XMP-photoshop:State={name}"));
            a.push(format!("-XMP-photoshop:Country={country}"));
            a.push(format!("-XMP-iptcCore:CountryCode={code}"));
        }
        Target::Country { name, code } => {
            clear_finer(&mut a, video, true);
            a.push(format!("-XMP-photoshop:Country={name}"));
            a.push(format!("-XMP-iptcCore:CountryCode={code}"));
        }
    }
    a.push(file.display().to_string());
    a
}

/// A coarser place replaces a finer one: remove the old map position and town
/// (and, for "country only", the old region), so the photo never says "Paris"
/// and "Germany" at once. Removing tags a file does not have is harmless.
fn clear_finer(a: &mut Vec<String>, video: bool, region_too: bool) {
    if video {
        a.extend(["-Keys:GPSCoordinates=", "-UserData:GPSCoordinates="].map(String::from));
    } else {
        a.extend(["-GPS:all=", "-XMP-exif:GPSLatitude=", "-XMP-exif:GPSLongitude="].map(String::from));
    }
    a.push("-XMP-photoshop:City=".into());
    if region_too {
        a.push("-XMP-photoshop:State=".into());
    }
}

/// One changed file in a run (for undo).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changed {
    pub rel: String,
    /// Content hash before and after the change.
    pub before: String,
    pub after: String,
}

/// The record of one run, stored next to its backups.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub run: String,
    pub place: String,
    pub changed: Vec<Changed>,
    pub undone: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub run: String,
    pub changed: usize,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

pub fn backups_root(folder: &Path) -> PathBuf {
    folder.join(".chronosort").join("place-backups")
}

fn manifest_path(folder: &Path, run: &str) -> PathBuf {
    backups_root(folder).join(run).join("manifest.json")
}

fn save_manifest(folder: &Path, m: &Manifest) -> std::io::Result<()> {
    let p = manifest_path(folder, &m.run);
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(m).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, p)
}

/// Copy `src` to `dst` (new file) and check the copy by hash; returns the hash.
fn backup(src: &Path, dst: &Path) -> std::io::Result<String> {
    if let Some(d) = dst.parent() {
        std::fs::create_dir_all(d)?;
    }
    let h = cs_core::hash_file(src)?;
    let mut out = std::fs::File::create_new(dst)?;
    std::io::copy(&mut std::fs::File::open(src)?, &mut out)?;
    out.sync_all()?;
    drop(out);
    if cs_core::hash_file(dst)? != h {
        let _ = std::fs::remove_file(dst);
        return Err(std::io::Error::other("the backup copy did not match"));
    }
    Ok(h)
}

/// A file to tag: path relative to `folder` and whether it is a video.
pub struct Job {
    pub rel: String,
    pub video: bool,
}

/// Add `target` to the files. Backs each file up first; a file whose write
/// fails is put back from its backup at once. `progress(done, total)`.
pub fn apply(
    exiftool: &Path,
    folder: &Path,
    jobs: &[Job],
    target: &Target,
    run: &str,
    progress: &dyn Fn(usize, usize),
    cancel: &std::sync::atomic::AtomicBool,
) -> std::io::Result<Report> {
    let dir = backups_root(folder).join(run);
    std::fs::create_dir_all(dir.join("files"))?;
    let mut m = Manifest { run: run.into(), place: target.label(), ..Default::default() };
    save_manifest(folder, &m)?;
    let mut et = exiftool::ExifTool::start(exiftool)?;
    let mut rep = Report { run: run.into(), ..Default::default() };
    for (i, j) in jobs.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        progress(i, jobs.len());
        let file = folder.join(&j.rel);
        let saved = dir.join("files").join(&j.rel);
        let before = match backup(&file, &saved) {
            Ok(h) => h,
            Err(e) => {
                rep.failed.push((j.rel.clone(), format!("no backup could be made ({e}); left unchanged")));
                continue;
            }
        };
        let out = et.run(&write_args(target, &file, j.video));
        let ok = matches!(&out, Ok(lines) if lines.iter().any(|l| l.contains("1 image files updated")));
        if !ok {
            // Put the original back if ExifTool left anything changed.
            if cs_core::hash_file(&file).ok().as_deref() != Some(before.as_str()) {
                let _ = std::fs::copy(&saved, &file);
            }
            let _ = std::fs::remove_file(&saved);
            let why = match out {
                Ok(lines) => lines.into_iter().find(|l| l.contains("Error")).unwrap_or_else(|| "not changed".into()),
                Err(e) => e.to_string(),
            };
            rep.failed.push((j.rel.clone(), why));
            continue;
        }
        let after = cs_core::hash_file(&file)?;
        m.changed.push(Changed { rel: j.rel.clone(), before, after });
        save_manifest(folder, &m)?; // after every file: a crash keeps the undo record
        rep.changed += 1;
    }
    progress(jobs.len(), jobs.len());
    Ok(rep)
}

#[derive(Debug, Default, Serialize)]
pub struct UndoReport {
    pub restored: usize,
    /// Changed again after the place was added: left alone.
    pub kept: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Put every file of a run back the way it was (only files that were not
/// changed again since).
pub fn undo(folder: &Path, run: &str) -> std::io::Result<UndoReport> {
    let mut m: Manifest =
        serde_json::from_slice(&std::fs::read(manifest_path(folder, run))?).map_err(std::io::Error::other)?;
    let dir = backups_root(folder).join(run).join("files");
    let mut rep = UndoReport::default();
    for c in &m.changed {
        let file = folder.join(&c.rel);
        let saved = dir.join(&c.rel);
        match cs_core::hash_file(&file) {
            Ok(h) if h == c.after => {}
            Ok(h) if h == c.before => {
                rep.restored += 1; // already back
                continue;
            }
            _ => {
                rep.kept.push(c.rel.clone());
                continue;
            }
        }
        if cs_core::hash_file(&saved).ok().as_deref() != Some(c.before.as_str()) {
            rep.failed.push((c.rel.clone(), "the backup is missing or damaged".into()));
            continue;
        }
        let tmp = file.with_extension("chronosort-undo");
        let r = std::fs::copy(&saved, &tmp).and_then(|_| std::fs::rename(&tmp, &file));
        match r {
            Ok(()) if cs_core::hash_file(&file).ok().as_deref() == Some(c.before.as_str()) => rep.restored += 1,
            Ok(()) => rep.failed.push((c.rel.clone(), "the restored file does not match".into())),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                rep.failed.push((c.rel.clone(), e.to_string()));
            }
        }
    }
    m.undone = rep.failed.is_empty();
    save_manifest(folder, &m)?;
    Ok(rep)
}

/// Runs that can still be undone in `folder`, newest first.
pub fn runs(folder: &Path) -> Vec<Manifest> {
    let mut out: Vec<Manifest> = std::fs::read_dir(backups_root(folder))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path().join("manifest.json")).ok()?).ok())
        .filter(|m: &Manifest| !m.undone && !m.changed.is_empty())
        .collect();
    out.sort_by(|a, b| b.run.cmp(&a.run));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paris() -> Place {
        Place {
            city: "Paris".into(),
            region: "Île-de-France".into(),
            country_code: "FR".into(),
            country: "France".into(),
            time_zone: "Europe/Paris".into(),
            population: 2_100_000,
            lat: 48.8534,
            lon: 2.3488,
        }
    }

    #[test]
    fn arguments() {
        let a = write_args(&Target::City { place: paris() }, Path::new("D:/x/a.jpg"), false);
        assert!(
            a.contains(&"-EXIF:GPSLatitude=48.853400".to_string())
                && a.contains(&"-EXIF:GPSLongitudeRef=E".to_string())
        );
        assert_eq!(a.last().unwrap(), "D:/x/a.jpg");
        let mut south = paris();
        (south.lat, south.lon) = (-34.9286, -58.5);
        let a = write_args(&Target::City { place: south }, Path::new("a.jpg"), false);
        assert!(
            a.contains(&"-EXIF:GPSLatitudeRef=S".to_string()) && a.contains(&"-EXIF:GPSLongitudeRef=W".to_string())
        );
        assert!(a.contains(&"-EXIF:GPSLatitude=34.928600".to_string()), "magnitude + reference");
        let v = write_args(&Target::City { place: paris() }, Path::new("a.mov"), true);
        assert!(v.iter().any(|x| x.starts_with("-Keys:GPSCoordinates=48.853400, 2.348800")));
        let c = write_args(&Target::Country { name: "France".into(), code: "FR".into() }, Path::new("a.jpg"), false);
        assert!(!c.iter().any(|x| x.contains("GPS") && !x.ends_with('=')), "country only: no made-up coordinates");
        assert!(c.contains(&"-GPS:all=".to_string()) && c.contains(&"-XMP-photoshop:State=".to_string()));
        let r = Target::Region { name: "Tuscany".into(), country: "Italy".into(), code: "IT".into() };
        assert_eq!(r.label(), "Tuscany, Italy");
        let a = write_args(&r, Path::new("a.mov"), true);
        assert!(
            a.contains(&"-XMP-photoshop:State=Tuscany".to_string()) && a.contains(&"-XMP-photoshop:City=".to_string())
        );
        assert!(a.contains(&"-Keys:GPSCoordinates=".to_string()) && !a.contains(&"-XMP-photoshop:State=".to_string()));
    }

    #[test]
    fn undo_restores_only_unchanged_files() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path();
        std::fs::write(f.join("a.jpg"), b"original a").unwrap();
        std::fs::write(f.join("b.jpg"), b"original b").unwrap();
        let run = "20261121-1";
        let dir = backups_root(f).join(run).join("files");
        let mut m = Manifest { run: run.into(), place: "Paris".into(), ..Default::default() };
        for n in ["a.jpg", "b.jpg"] {
            let before = backup(&f.join(n), &dir.join(n)).unwrap();
            std::fs::write(f.join(n), format!("tagged {n}")).unwrap();
            m.changed.push(Changed { rel: n.into(), before, after: cs_core::hash_file(&f.join(n)).unwrap() });
        }
        save_manifest(f, &m).unwrap();
        assert_eq!(runs(f).len(), 1);
        std::fs::write(f.join("b.jpg"), b"edited again later").unwrap();
        let r = undo(f, run).unwrap();
        assert_eq!(r.restored, 1);
        assert_eq!(r.kept, vec!["b.jpg".to_string()], "a file changed again is left alone");
        assert_eq!(std::fs::read(f.join("a.jpg")).unwrap(), b"original a");
        assert_eq!(std::fs::read(f.join("b.jpg")).unwrap(), b"edited again later");
    }
}
