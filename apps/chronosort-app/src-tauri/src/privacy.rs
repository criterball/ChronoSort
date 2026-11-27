//! Lossless JPEG crop to a new file (viewer → Crop), and removing information
//! from photos before sharing them (right-click → Remove photo information).
//!
//! Removing information works like star ratings: one ExifTool run per file, a
//! verified backup first (`.chronosort/info-backups/<run>/`), file dates kept,
//! and one Undo for the whole batch.

use crate::fileops::{self, Op, Undo};
use crate::{AppState, Res, err};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{Emitter, State};

#[derive(Serialize)]
pub struct Cropped {
    pub path: String,
    pub name: String,
    /// Upright size of the new picture.
    pub width: u32,
    pub height: u32,
    /// The edges had to move to the 8/16-pixel block grid.
    pub moved: bool,
}

/// Crop the JPEG `id` losslessly to the rectangle (upright pixels, as shown) into a new file.
#[tauri::command]
pub async fn jpeg_lossless_crop(state: State<'_, AppState>, id: u64, x: u32, y: u32, w: u32, h: u32) -> Res<Cropped> {
    let src = state.browse.path(id).ok_or("This file is no longer listed; press F5.")?;
    tauri::async_runtime::spawn_blocking(move || lossless_crop_file(&src, x, y, w, h)).await.map_err(err)?
}

pub fn lossless_crop_file(src: &Path, x: u32, y: u32, w: u32, h: u32) -> Res<Cropped> {
    use cs_image::jpegcrop;
    let bytes = std::fs::read(src).map_err(err)?;
    let (sw, sh, _, _) = jpegcrop::grid(&bytes).map_err(|e| e.to_string())?;
    let o = cs_meta::orientation_and_thumbnail(src, cs_scan::Format::Jpeg).0.unwrap_or(1);
    let (rx, ry, rw, rh) = jpegcrop::upright_to_stored(o, sw, sh, x, y, w, h);
    let (out, used) = jpegcrop::crop(&bytes, rx, ry, rw, rh).map_err(|e| e.to_string())?;
    let moved = used != (rx, ry, rw.min(sw - rx.min(sw)), rh.min(sh - ry.min(sh)));
    let dir = src.parent().ok_or("no folder")?;
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
    let target = fileops::free_name(dir, &format!("{stem}_crop.jpg"));
    fileops::guard(&target)?;
    {
        use std::io::Write;
        let mut f = std::fs::File::options().write(true).create_new(true).open(&target).map_err(err)?;
        f.write_all(&out).map_err(err)?;
        f.sync_all().map_err(err)?;
    }
    // The EXIF size and the small built-in preview still describe the whole picture: fix them.
    if let Ok(exe) = crate::places::exiftool()
        && let Ok(mut et) = cs_geo::exiftool::ExifTool::start(&exe)
    {
        let args: Vec<String> = vec![
            "-charset".into(),
            "filename=utf8".into(),
            "-overwrite_original".into(),
            format!("-ExifIFD:ExifImageWidth={}", used.2),
            format!("-ExifIFD:ExifImageHeight={}", used.3),
            "-IFD1:all=".into(),
            target.to_string_lossy().into_owned(),
        ];
        let _ = et.run(&args);
    }
    if let (Ok(t), Ok(f)) = (cs_platform::read_times(src), std::fs::File::options().write(true).open(&target)) {
        let _ = cs_platform::apply_times(&f, &t);
    }
    let m = std::fs::metadata(&target).map_err(err)?;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    fileops::push(Op {
        label: format!("Lossless crop “{name}”"),
        undo: Undo::Copied { files: vec![(target.clone(), m.len(), m.modified().ok())], dirs: vec![] },
    });
    let (width, height) = if (5..=8).contains(&o) { (used.3, used.2) } else { (used.2, used.3) };
    Ok(Cropped { path: target.to_string_lossy().into_owned(), name, width, height, moved })
}

// ------------------------------------------------------- remove information

/// ExifTool arguments: "all" = everything except the turn and the colour profile
/// (so the photo still looks the same); "place" = position and place names only.
pub fn strip_args(what: &str) -> Res<Vec<String>> {
    let mut a: Vec<String> = ["-charset", "filename=utf8", "-overwrite_original", "-P"].map(String::from).to_vec();
    match what {
        "all" => a.extend(
            ["-all=", "-tagsFromFile", "@", "-Orientation", "-ICC_Profile", "-ColorSpaceTags"].map(String::from),
        ),
        "place" => a.extend(
            [
                "-gps:all=",
                "-XMP-exif:GPS*=",
                "-XMP-photoshop:City=",
                "-XMP-photoshop:State=",
                "-XMP-photoshop:Country=",
                "-XMP-iptcCore:CountryCode=",
                "-XMP-iptcCore:Location=",
                "-XMP-iptcExt:LocationShown=",
                "-XMP-iptcExt:LocationCreated=",
                "-IPTC:City=",
                "-IPTC:Sub-location=",
                "-IPTC:Province-State=",
                "-IPTC:Country-PrimaryLocationName=",
                "-IPTC:Country-PrimaryLocationCode=",
                "-Keys:GPSCoordinates=",
                "-UserData:GPSCoordinates=",
            ]
            .map(String::from),
        ),
        _ => return Err("Choose what to remove.".into()),
    }
    Ok(a)
}

#[derive(Clone, Serialize)]
struct Progress {
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    current: String,
}

#[derive(Serialize, Default)]
pub struct StripReport {
    pub done: usize,
    pub unchanged: usize,
    pub failed: Vec<(String, String)>,
}

/// Remove information from photos (see [`strip_args`]); one Undo puts it all back.
#[tauri::command]
pub async fn strip_meta(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<u64>,
    what: String,
) -> Res<StripReport> {
    let files: Vec<PathBuf> = ids.into_iter().filter_map(|id| state.browse.path(id)).collect();
    tauri::async_runtime::spawn_blocking(move || {
        strip_files(&files, &what, |p| {
            let _ = app.emit("fileop-progress", p);
        })
    })
    .await
    .map_err(err)?
}

fn strip_files(files: &[PathBuf], what: &str, emit: impl Fn(Progress)) -> Res<StripReport> {
    let args = strip_args(what)?;
    if files.is_empty() {
        return Err("Select one or more photos first.".into());
    }
    let mut et = cs_geo::exiftool::ExifTool::start(&crate::places::exiftool()?).map_err(err)?;
    let run = format!("{}", crate::now_unix());
    let mut rep = StripReport::default();
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
        match strip_one(&mut et, file, &args, &run) {
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
        let what = if what == "place" { "Remove the place from" } else { "Remove the information from" };
        fileops::push(Op {
            label: format!("{what} {n} {}", if n == 1 { "photo" } else { "photos" }),
            undo: Undo::MetaEditedMany(undo),
        });
    }
    Ok(rep)
}

/// One file: `None` when there was nothing to remove.
fn strip_one(
    et: &mut cs_geo::exiftool::ExifTool,
    file: &Path,
    args: &[String],
    run: &str,
) -> Res<Option<(PathBuf, PathBuf, String)>> {
    fileops::guard(file)?;
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !crate::meta::WRITABLE.contains(&ext.as_str()) && !matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
        return Err("ChronoSort cannot change this kind of file.".into());
    }
    let dir = file.parent().ok_or("no folder")?;
    let name = file.file_name().ok_or("no name")?.to_string_lossy().into_owned();
    let saved = fileops::free_name(&dir.join(".chronosort").join("info-backups").join(run), &name);
    let before = crate::meta::backup(file, &saved)?;
    let mut a = args.to_vec();
    a.push(file.to_string_lossy().into_owned());
    let out = et.run(&a).map_err(err);
    let changed = matches!(&out, Ok(l) if l.iter().any(|x| x.contains("1 image files updated")));
    let after = cs_core::hash_file(file).map_err(err)?;
    if !changed || after == before {
        // Nothing written (or nothing to remove): put back exactly what was there.
        if after != before {
            let _ = std::fs::copy(&saved, file);
        }
        let _ = std::fs::remove_file(&saved);
        return match out {
            Ok(l) if l.iter().any(|x| x.contains("unchanged")) || after == before => Ok(None),
            Ok(l) => Err(l
                .into_iter()
                .find(|x| x.contains("Error") || x.contains("Warning"))
                .unwrap_or_else(|| "nothing was written".into())),
            Err(e) => Err(e),
        };
    }
    Ok(Some((file.to_path_buf(), saved, after)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        assert!(strip_args("all").unwrap().contains(&"-all=".to_string()));
        assert!(strip_args("all").unwrap().contains(&"-Orientation".to_string()));
        let p = strip_args("place").unwrap();
        assert!(p.contains(&"-gps:all=".to_string()) && !p.contains(&"-all=".to_string()));
        assert!(strip_args("x").is_err());
    }

    /// Real ExifTool: remove the place, then everything; Undo gives back the exact bytes.
    #[test]
    fn strip_and_undo_with_exiftool() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(exe) = crate::places::exiftool() else { return };
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("p.jpg");
        image::RgbImage::from_pixel(32, 24, image::Rgb([90, 120, 150])).save(&f).unwrap();
        let mut et = cs_geo::exiftool::ExifTool::start(&exe).unwrap();
        let set: Vec<String> = [
            "-overwrite_original",
            "-GPSLatitude=48.85",
            "-GPSLatitudeRef=N",
            "-GPSLongitude=2.35",
            "-GPSLongitudeRef=E",
            "-XMP-photoshop:City=Paris",
            "-Artist=Someone",
            "-Orientation#=6",
        ]
        .map(String::from)
        .into_iter()
        .chain([f.to_string_lossy().into_owned()])
        .collect();
        et.run(&set).unwrap();
        let original = std::fs::read(&f).unwrap();
        let read = |et: &mut cs_geo::exiftool::ExifTool| {
            et.run(&[
                "-s".into(),
                "-GPSLatitude".into(),
                "-City".into(),
                "-Artist".into(),
                "-Orientation#".into(),
                f.to_string_lossy().into_owned(),
            ])
            .unwrap()
            .join("\n")
        };
        let r = strip_files(std::slice::from_ref(&f), "place", |_| {}).unwrap();
        assert_eq!((r.done, r.unchanged), (1, 0));
        let t = read(&mut et);
        assert!(!t.contains("GPSLatitude") && !t.contains("Paris") && t.contains("Someone"), "{t}");
        // Again: nothing left to remove → unchanged, no backup kept.
        let r = strip_files(std::slice::from_ref(&f), "place", |_| {}).unwrap();
        assert_eq!((r.done, r.unchanged), (0, 1));
        let r = strip_files(std::slice::from_ref(&f), "all", |_| {}).unwrap();
        assert_eq!(r.done, 1);
        let t = read(&mut et);
        assert!(!t.contains("Someone") && t.contains("Orientation") && t.contains('6'), "{t}");
        // Two undos: everything back, byte for byte.
        crate::fileops::undo_for_test(crate::fileops::take_last_for_test()).unwrap();
        crate::fileops::undo_for_test(crate::fileops::take_last_for_test()).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), original);
    }

    #[test]
    fn lossless_crop_into_a_new_file() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("c.jpg");
        image::RgbImage::from_fn(100, 60, |x, y| image::Rgb([(x * 2) as u8, (y * 4) as u8, 77])).save(&f).unwrap();
        let r = lossless_crop_file(&f, 10, 10, 50, 30).unwrap();
        assert!(r.moved); // 10 is not on the block grid
        assert!(r.name.ends_with("_crop.jpg"));
        let img = image::open(&r.path).unwrap();
        assert_eq!((img.width(), img.height()), (r.width, r.height));
        assert!(r.width >= 50 && r.height >= 30);
        crate::fileops::undo_for_test(crate::fileops::take_last_for_test()).unwrap();
        assert!(!Path::new(&r.path).exists());
    }
}
