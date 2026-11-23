//! The EXIF / file information panel and its edit mode.
//!
//! Reading and writing use the ExifTool that ships with ChronoSort (no
//! network). Before a write the file is backed up (verified copy) into
//! `<folder>/.chronosort/info-backups/<run>/`, so Ctrl+Z can put it back.

use crate::fileops::{self, Op, Undo};
use crate::{AppState, Res, err};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Serialize)]
pub struct Tag {
    pub group: String,
    pub name: String,
    pub value: String,
}

#[derive(Serialize, Default)]
pub struct MetaInfo {
    /// Every tag ExifTool found ("group" = where it is stored, e.g. ExifIFD, XMP-dc).
    pub tags: Vec<Tag>,
    /// Signed decimal degrees, if the photo has a position.
    pub gps: Option<(f64, f64)>,
    /// Whether ChronoSort can write information into this file type.
    pub writable: bool,
}

fn run_exiftool(args: Vec<String>) -> Res<Vec<String>> {
    let mut et = cs_geo::exiftool::ExifTool::start(&crate::places::exiftool()?).map_err(err)?;
    et.run(&args).map_err(err)
}

fn value_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join("; "),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

const WRITABLE: &[&str] = &[
    "jpg", "jpeg", "jpe", "tif", "tiff", "png", "webp", "heic", "heif", "dng", "cr2", "nef", "arw", "orf", "rw2", "pef",
];

#[tauri::command]
pub async fn meta_read(state: State<'_, AppState>, id: u64) -> Res<MetaInfo> {
    let file = state.browse.path(id).ok_or("This file is no longer listed; press F5.")?;
    tauri::async_runtime::spawn_blocking(move || read(&file)).await.map_err(err)?
}

pub fn read(file: &Path) -> Res<MetaInfo> {
    let out = run_exiftool(
        // No -q here: it would also hide the {ready} line that ends each answer.
        ["-charset", "filename=utf8", "-j", "-G1", "-c", "%+.6f", "-api", "LargeFileSupport=1"]
            .into_iter()
            .map(String::from)
            .chain([file.to_string_lossy().into_owned()])
            .collect(),
    )?;
    let text = out.join("\n");
    let start = text.find('[').ok_or("ExifTool gave no answer for this file.")?;
    let end = text.rfind(']').filter(|&e| e > start).ok_or("ExifTool gave an incomplete answer.")?;
    let json: serde_json::Value = serde_json::from_str(&text[start..=end]).map_err(err)?;
    let obj = json.get(0).and_then(|o| o.as_object()).ok_or("no information")?;
    let mut info = MetaInfo::default();
    let (mut lat, mut lon) = (None, None);
    for (k, v) in obj {
        let (group, name) = k.split_once(':').unwrap_or(("", k.as_str()));
        // Leave out ExifTool's own bookkeeping and big binary blobs.
        if matches!(group, "ExifTool" | "System") && !matches!(name, "FileSize" | "FileModifyDate" | "FileCreateDate")
            || name == "SourceFile"
        {
            continue;
        }
        let value = value_text(v);
        if value.starts_with("(Binary data") {
            continue;
        }
        if name == "GPSLatitude" && lat.is_none() {
            lat = value.trim().parse::<f64>().ok();
        }
        if name == "GPSLongitude" && lon.is_none() {
            lon = value.trim().parse::<f64>().ok();
        }
        info.tags.push(Tag { group: group.into(), name: name.into(), value });
    }
    info.gps = lat.zip(lon);
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    info.writable = WRITABLE.contains(&ext.as_str());
    Ok(info)
}

/// Fields the edit mode can change. `None` = leave as it is; `Some("")` = remove.
#[derive(Deserialize, Default, Debug)]
pub struct MetaChanges {
    pub date_taken: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub author: Option<String>,
    pub copyright: Option<String>,
    /// New position (signed decimal degrees).
    pub gps: Option<(f64, f64)>,
    /// Remove the position (and the place names with it).
    #[serde(default)]
    pub clear_gps: bool,
    /// Names of a town picked from the offline list; written together with `gps`.
    pub place: Option<PlaceNames>,
}

#[derive(Deserialize, Default, Debug)]
pub struct PlaceNames {
    pub city: String,
    #[serde(default)]
    pub region: String,
    pub country: String,
    #[serde(default)]
    pub country_code: String,
}

/// "2004:05:06 14:30:00" (seconds optional; "-" or "/" also accepted).
pub fn normalize_date(s: &str) -> Res<String> {
    let t = s.trim().replace(['-', '/'], ":").replace('T', " ");
    let (d, time) = t.split_once(' ').unwrap_or((&t, "00:00:00"));
    let dp: Vec<u32> = d
        .split(':')
        .map(|x| x.parse().map_err(|_| "bad"))
        .collect::<Result<_, _>>()
        .map_err(|_| "Type the date as YYYY-MM-DD HH:MM.")?;
    let tp: Vec<u32> = time
        .split(':')
        .map(|x| x.parse().map_err(|_| "bad"))
        .collect::<Result<_, _>>()
        .map_err(|_| "Type the time as HH:MM or HH:MM:SS.")?;
    let ok = dp.len() == 3 && (1800..=2200).contains(&dp[0]) && (1..=12).contains(&dp[1]) && (1..=31).contains(&dp[2]);
    let tok = (2..=3).contains(&tp.len()) && tp[0] < 24 && tp[1] < 60 && tp.get(2).is_none_or(|s| *s < 60);
    if !ok || !tok {
        return Err("That date or time is not valid (YYYY-MM-DD HH:MM).".into());
    }
    Ok(format!(
        "{:04}:{:02}:{:02} {:02}:{:02}:{:02}",
        dp[0],
        dp[1],
        dp[2],
        tp[0],
        tp[1],
        tp.get(2).copied().unwrap_or(0)
    ))
}

/// ExifTool arguments (one per line, no shell) for the changes.
pub fn write_args(c: &MetaChanges) -> Res<Vec<String>> {
    let mut a: Vec<String> = ["-charset", "filename=utf8", "-overwrite_original", "-P"].map(String::from).to_vec();
    let clean = |s: &str| s.replace(['\n', '\r'], " ").trim().to_string();
    if let Some(d) = &c.date_taken {
        if d.trim().is_empty() {
            return Err("The date taken cannot be removed here, only changed.".into());
        }
        let d = normalize_date(d)?;
        a.extend([
            format!("-EXIF:DateTimeOriginal={d}"),
            format!("-EXIF:CreateDate={d}"),
            format!("-XMP-exif:DateTimeOriginal={d}"),
        ]);
    }
    if let Some(t) = &c.title {
        let t = clean(t);
        a.extend([format!("-XMP-dc:Title={t}"), format!("-EXIF:XPTitle={t}")]);
    }
    if let Some(t) = &c.description {
        let t = clean(t);
        a.extend([format!("-XMP-dc:Description={t}"), format!("-EXIF:ImageDescription={t}")]);
    }
    if let Some(k) = &c.keywords {
        let words: Vec<String> = k.iter().map(|w| clean(w)).filter(|w| !w.is_empty()).collect();
        a.push("-XMP-dc:Subject=".into());
        a.extend(words.iter().map(|w| format!("-XMP-dc:Subject+={w}")));
        a.push(format!("-EXIF:XPKeywords={}", words.join(";")));
    }
    if let Some(t) = &c.author {
        let t = clean(t);
        a.extend([format!("-XMP-dc:Creator={t}"), format!("-EXIF:Artist={t}")]);
    }
    if let Some(t) = &c.copyright {
        let t = clean(t);
        a.extend([format!("-XMP-dc:Rights={t}"), format!("-EXIF:Copyright={t}")]);
    }
    let gps = if c.clear_gps { Some(None) } else { c.gps.map(Some) };
    match gps {
        Some(Some((lat, lon))) => {
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                return Err("That position is outside the map.".into());
            }
            a.extend([
                format!("-EXIF:GPSLatitude={:.6}", lat.abs()),
                format!("-EXIF:GPSLatitudeRef={}", if lat >= 0.0 { "N" } else { "S" }),
                format!("-EXIF:GPSLongitude={:.6}", lon.abs()),
                format!("-EXIF:GPSLongitudeRef={}", if lon >= 0.0 { "E" } else { "W" }),
            ]);
            if let Some(p) = &c.place {
                let (city, country) = (clean(&p.city), clean(&p.country));
                if city.is_empty() || country.is_empty() {
                    return Err("The town needs a name and a country.".into());
                }
                // Same tags as "Add a place"; an empty value removes an old region / code.
                a.extend([
                    format!("-XMP-photoshop:City={city}"),
                    format!("-XMP-photoshop:State={}", clean(&p.region)),
                    format!("-XMP-photoshop:Country={country}"),
                    format!("-XMP-iptcCore:CountryCode={}", clean(&p.country_code)),
                ]);
            }
        }
        Some(None) => a.extend(
            [
                "-GPS:all=",
                "-XMP-photoshop:City=",
                "-XMP-photoshop:State=",
                "-XMP-photoshop:Country=",
                "-XMP-iptcCore:CountryCode=",
            ]
            .map(String::from),
        ),
        None if c.place.is_some() => return Err("A town can only be saved together with its position.".into()),
        None => {}
    }
    if a.len() == 4 {
        return Err("Nothing was changed.".into());
    }
    Ok(a)
}

/// Verified copy of `src` to the new file `dst`.
fn backup(src: &Path, dst: &Path) -> Res<String> {
    std::fs::create_dir_all(dst.parent().ok_or("no folder")?).map_err(err)?;
    let h = cs_core::hash_file(src).map_err(err)?;
    std::fs::copy(src, dst).map_err(err)?;
    if cs_core::hash_file(dst).map_err(err)? != h {
        let _ = std::fs::remove_file(dst);
        return Err("The safety copy did not match; nothing was changed.".into());
    }
    Ok(h)
}

#[tauri::command]
pub async fn meta_write(state: State<'_, AppState>, id: u64, changes: MetaChanges) -> Res<MetaInfo> {
    let file = state.browse.path(id).ok_or("This file is no longer listed; press F5.")?;
    tauri::async_runtime::spawn_blocking(move || write(&file, &changes)).await.map_err(err)?
}

pub fn write(file: &Path, changes: &MetaChanges) -> Res<MetaInfo> {
    fileops::guard(file)?;
    let mut args = write_args(changes)?;
    let dir = file.parent().ok_or("no folder")?;
    let run = format!("{}", crate::now_unix());
    let bdir = dir.join(".chronosort").join("info-backups").join(&run);
    let name = file.file_name().ok_or("no name")?.to_string_lossy().into_owned();
    let saved: PathBuf = fileops::free_name(&bdir, &name);
    let before = backup(file, &saved)?;
    args.push(file.to_string_lossy().into_owned());
    let out = run_exiftool(args);
    let ok = matches!(&out, Ok(l) if l.iter().any(|x| x.contains("1 image files updated")));
    if !ok {
        if cs_core::hash_file(file).ok().as_deref() != Some(before.as_str()) {
            let _ = std::fs::copy(&saved, file);
        }
        let _ = std::fs::remove_file(&saved);
        let why = match out {
            Ok(l) => l
                .into_iter()
                .find(|x| x.contains("Error") || x.contains("Warning"))
                .unwrap_or_else(|| "nothing was written".into()),
            Err(e) => e,
        };
        return Err(format!("The information could not be saved: {why}"));
    }
    let after = cs_core::hash_file(file).map_err(err)?;
    fileops::push(Op {
        label: format!("Change the information of “{name}”"),
        undo: Undo::MetaEdited { file: file.to_path_buf(), backup: saved, after },
    });
    read(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_normalized() {
        assert_eq!(normalize_date("2004-05-06 14:30").unwrap(), "2004:05:06 14:30:00");
        assert_eq!(normalize_date("2004:05:06 14:30:15").unwrap(), "2004:05:06 14:30:15");
        assert_eq!(normalize_date("1999/12/31").unwrap(), "1999:12:31 00:00:00");
        for bad in ["2004-13-01", "yesterday", "2004-05-06 25:00", ""] {
            assert!(normalize_date(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn args_cover_fields_and_strip_line_breaks() {
        let c = MetaChanges {
            title: Some("Beach\nday".into()),
            keywords: Some(vec!["sea".into(), " ".into(), "sun".into()]),
            gps: Some((-33.86, 151.21)),
            ..Default::default()
        };
        let a = write_args(&c).unwrap();
        assert!(a.contains(&"-XMP-dc:Title=Beach day".to_string()));
        assert!(a.contains(&"-XMP-dc:Subject+=sea".to_string()) && a.contains(&"-EXIF:XPKeywords=sea;sun".to_string()));
        assert!(
            a.contains(&"-EXIF:GPSLatitudeRef=S".to_string()) && a.contains(&"-EXIF:GPSLongitudeRef=E".to_string())
        );
        assert!(a.iter().all(|x| !x.contains('\n')));
        assert!(write_args(&MetaChanges::default()).is_err());
        assert!(write_args(&MetaChanges { gps: Some((95.0, 0.0)), ..Default::default() }).is_err());
        assert!(
            write_args(&MetaChanges { clear_gps: true, ..Default::default() })
                .unwrap()
                .contains(&"-GPS:all=".to_string())
        );
    }

    #[test]
    fn picked_town_writes_names_and_clear_removes_them() {
        let town = || PlaceNames {
            city: "Adelaide".into(),
            region: "South Australia".into(),
            country: "Australia".into(),
            country_code: "AU".into(),
        };
        let a = write_args(&MetaChanges { gps: Some((-34.928, 138.6)), place: Some(town()), ..Default::default() })
            .unwrap();
        for t in [
            "-XMP-photoshop:City=Adelaide",
            "-XMP-photoshop:State=South Australia",
            "-XMP-photoshop:Country=Australia",
            "-XMP-iptcCore:CountryCode=AU",
        ] {
            assert!(a.contains(&t.to_string()), "{t}");
        }
        // Names without a position, or without a name, are refused.
        assert!(write_args(&MetaChanges { place: Some(town()), ..Default::default() }).is_err());
        let nameless = PlaceNames { city: " ".into(), ..town() };
        assert!(
            write_args(&MetaChanges { gps: Some((1.0, 1.0)), place: Some(nameless), ..Default::default() }).is_err()
        );
        let cleared = write_args(&MetaChanges { clear_gps: true, ..Default::default() }).unwrap();
        assert!(cleared.contains(&"-XMP-photoshop:City=".to_string()));
        assert!(cleared.contains(&"-XMP-photoshop:Country=".to_string()));
    }
}

#[cfg(test)]
mod exiftool_tests {
    use super::*;

    /// Real ExifTool: read, write with backup, undo puts the file back byte for byte.
    #[test]
    fn read_write_undo_roundtrip() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if crate::places::exiftool().is_err() {
            eprintln!("skip: no ExifTool");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("photo.jpg");
        let img = image::RgbImage::from_pixel(32, 24, image::Rgb([200, 120, 40]));
        img.save(&f).unwrap();
        let before = std::fs::read(&f).unwrap();
        let t = std::time::Instant::now();
        let info = read(&f).unwrap();
        assert!(t.elapsed().as_secs() < 20, "reading took {:?}", t.elapsed());
        assert!(info.writable && info.gps.is_none());
        let c = MetaChanges {
            date_taken: Some("2004-05-06 14:30".into()),
            title: Some("Beach".into()),
            keywords: Some(vec!["sea".into(), "sun".into()]),
            gps: Some((-34.928, 138.6)),
            place: Some(PlaceNames {
                city: "Adelaide".into(),
                region: "South Australia".into(),
                country: "Australia".into(),
                country_code: "AU".into(),
            }),
            ..Default::default()
        };
        let after = write(&f, &c).unwrap();
        let get = |n: &str| after.tags.iter().find(|t| t.name == n).map(|t| t.value.clone()).unwrap_or_default();
        assert_eq!(get("DateTimeOriginal"), "2004:05:06 14:30:00");
        assert_eq!(get("Title"), "Beach");
        assert!(get("Subject").contains("sea") && get("Subject").contains("sun"));
        let (lat, lon) = after.gps.unwrap();
        assert!((lat + 34.928).abs() < 1e-4 && (lon - 138.6).abs() < 1e-4, "{lat},{lon}");
        assert_eq!(
            (get("City"), get("State"), get("Country")),
            ("Adelaide".into(), "South Australia".into(), "Australia".into())
        );
        assert_eq!(get("CountryCode"), "AU");
        // Undo: newest op in the shared history is ours.
        let op = crate::fileops::take_last_for_test();
        crate::fileops::undo_for_test(op).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), before, "undo must restore the exact bytes");
    }
}
