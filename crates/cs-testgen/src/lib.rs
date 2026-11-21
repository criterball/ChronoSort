//! `cs-testgen`: builds a synthetic "Old_Photos" tree full of real-world edge
//! cases, plus `manifest.json` describing what ChronoSort is expected to do
//! with every file. The sorter is tested against this before it ever touches
//! a real family archive.

use cs_platform::{Times, apply_times};
use image::{ImageEncoder, RgbImage};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub mod eval;
pub mod mp4;
pub mod tiff;

use tiff::*;

pub const SOURCE_DIR: &str = "Old_Photos";
pub const GENERATOR_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Confident: sort without asking.
    Auto,
    /// Must be shown to the user (with `year` as the suggestion, if any).
    Review,
    /// Must not be copied (OS junk).
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Path relative to `Old_Photos`, with `/` separators.
    pub path: String,
    pub route: Route,
    pub year: Option<i16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Files that must travel together (RAW+JPG, sidecars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    pub note: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub generator_version: u32,
    pub source_dir: String,
    pub entries: Vec<Entry>,
}

// ---------- time helpers (no external date crate needed) ----------

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn unix(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
    days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s
}

fn st(unix: i64) -> SystemTime {
    if unix >= 0 {
        SystemTime::UNIX_EPOCH + Duration::from_secs(unix as u64)
    } else {
        SystemTime::UNIX_EPOCH - Duration::from_secs(unix.unsigned_abs())
    }
}

/// Typical situation: the archive was copied from an old PC in mid-2024, so the
/// filesystem dates say 2024 and must NOT be trusted.
fn copied_2024() -> Times {
    let t = st(unix(2024, 6, 1, 12, 0, 0));
    Times { modified: t, accessed: Some(t), created: Some(t) }
}

// ---------- content builders ----------

/// Small, visually distinct image per seed (so nothing is an accidental duplicate).
fn pixels(seed: u32) -> RgbImage {
    RgbImage::from_fn(96, 64, |x, y| {
        image::Rgb([
            (x.wrapping_mul(seed) % 256) as u8,
            (y.wrapping_mul(seed.wrapping_mul(3)) % 256) as u8,
            (seed.wrapping_mul(37).wrapping_add(x ^ y) % 256) as u8,
        ])
    })
}

fn jpeg(seed: u32) -> Vec<u8> {
    let img = pixels(seed);
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8)
        .expect("jpeg encode");
    buf
}

fn png(seed: u32) -> Vec<u8> {
    let img = pixels(seed);
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8)
        .expect("png encode");
    buf
}

#[derive(Default)]
struct Exif<'a> {
    make: Option<&'a str>,
    model: Option<&'a str>,
    datetime: Option<&'a str>,
    original: Option<&'a str>,
    offset: Option<&'a str>,
}

impl Exif<'_> {
    fn tiff(&self) -> Vec<u8> {
        let a = |s: &str| Val::Ascii(s.to_string());
        let mut ifd0 = Vec::new();
        if let Some(v) = self.make {
            ifd0.push((TAG_MAKE, a(v)));
        }
        if let Some(v) = self.model {
            ifd0.push((TAG_MODEL, a(v)));
        }
        if let Some(v) = self.datetime {
            ifd0.push((TAG_DATETIME, a(v)));
        }
        let mut exif = Vec::new();
        if let Some(v) = self.original {
            exif.push((TAG_DATETIME_ORIGINAL, a(v)));
            exif.push((TAG_DATETIME_DIGITIZED, a(v)));
        }
        if let Some(v) = self.offset {
            exif.push((TAG_OFFSET_TIME_ORIGINAL, a(v)));
        }
        build(&ifd0, &exif)
    }
    fn jpeg(&self, seed: u32) -> Vec<u8> {
        insert_into_jpeg(&jpeg(seed), &self.tiff())
    }
}

fn camera(original: &str) -> Exif<'_> {
    Exif {
        make: Some("Canon"),
        model: Some("Canon EOS 300D DIGITAL"),
        datetime: Some(original),
        original: Some(original),
        ..Default::default()
    }
}

// ---------- writer ----------

struct Gen {
    root: PathBuf,
    entries: Vec<Entry>,
}

impl Gen {
    /// Write a new file (never overwrites) and stamp its filesystem times.
    fn put(&self, rel: &str, bytes: &[u8], times: Times) -> io::Result<()> {
        let p = self.root.join(rel);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::File::options().write(true).create_new(true).open(&p)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        apply_times(&f, &times)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn case(
        &mut self,
        rel: &str,
        bytes: &[u8],
        route: Route,
        year: Option<i16>,
        tags: &[&'static str],
        note: &'static str,
    ) -> io::Result<&mut Entry> {
        self.put(rel, bytes, copied_2024())?;
        self.entries.push(Entry {
            path: rel.to_string(),
            route,
            year,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            group: None,
            duplicate_of: None,
            note: note.to_string(),
        });
        Ok(self.entries.last_mut().unwrap())
    }
}

/// Generate the corpus into `out/Old_Photos` and write `out/manifest.json`.
/// Fails if `out/Old_Photos` already exists (the generator never overwrites).
pub fn generate(out: &Path) -> io::Result<Manifest> {
    use Route::*;
    let root = out.join(SOURCE_DIR);
    if root.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", root.display())));
    }
    std::fs::create_dir_all(&root)?;
    let mut g = Gen { root, entries: Vec::new() };

    // --- EXIF -------------------------------------------------------------
    let dsc1 = camera("2005:03:12 14:22:07").jpeg(1);
    g.case("Camera/DSC_0001.jpg", &dsc1, Auto, Some(2005), &["exif"], "Plain EXIF DateTimeOriginal")?;

    let e = Exif { offset: Some("+01:00"), ..camera("2010:12:31 23:30:00") };
    g.case(
        "Camera/DSC_0002.jpg",
        &e.jpeg(2),
        Auto,
        Some(2010),
        &["exif", "tz", "year_boundary"],
        "EXIF local time 23:30 on New Year's Eve; local wall time decides the year, not UTC",
    )?;

    g.case(
        "2003/DSC_0100.jpg",
        &camera("2000:01:01 00:00:00").jpeg(3),
        Review,
        Some(2003),
        &["exif_reset_date", "folder_year"],
        "Camera clock was never set (2000-01-01 00:00); folder suggests 2003",
    )?;

    g.case(
        "2001/DSC_0500.jpg",
        &camera("2007:08:09 10:11:12").jpeg(4),
        Review,
        Some(2007),
        &["conflict", "exif", "folder_year"],
        "EXIF says 2007 but folder says 2001: show both",
    )?;

    g.case(
        "Misc/DSC_0900.jpg",
        &camera("2099:01:01 08:00:00").jpeg(5),
        Review,
        None,
        &["future_date"],
        "Date in the future is impossible",
    )?;

    let e = Exif {
        make: Some("Apple"),
        model: Some("iPhone 12"),
        original: Some("2015:06:01 12:00:00"),
        ..Default::default()
    };
    g.case(
        "Misc/IMG_4521.jpg",
        &e.jpeg(6),
        Review,
        None,
        &["camera_anachronism"],
        "iPhone 12 was released in 2020, so a 2015 date is wrong",
    )?;

    // Scanner: EXIF date is the *scan* date.
    let e = Exif {
        make: Some("EPSON"),
        model: Some("Perfection V600"),
        datetime: Some("2021:05:05 09:00:00"),
        ..Default::default()
    };
    g.case(
        "Old_1987/scan0001.jpg",
        &e.jpeg(7),
        Review,
        Some(1987),
        &["scanner", "folder_year"],
        "Scanned print: EXIF date (2021) is when it was scanned; folder suggests 1987",
    )?;

    // --- Filenames ----------------------------------------------------------
    g.case(
        "Misc/IMG_20050312_142207.jpg",
        &jpeg(8),
        Auto,
        Some(2005),
        &["filename_datetime"],
        "Phone-style filename with full date and time",
    )?;
    g.case(
        "Misc/IMG-20190101-WA0001.jpg",
        &jpeg(9),
        Auto,
        Some(2019),
        &["filename_whatsapp"],
        "WhatsApp strips EXIF; the date is in the name",
    )?;
    g.case(
        "Misc/2004-05-06 party.jpg",
        &png(10),
        Auto,
        Some(2004),
        &["filename_date", "extension_mismatch"],
        "PNG content with a .jpg extension: detect by content, keep the bytes untouched",
    )?;

    // --- Folders ------------------------------------------------------------
    g.case(
        "2005/03/scan_001.jpg",
        &jpeg(11),
        Auto,
        Some(2005),
        &["folder_year_month"],
        "Year folder containing month folders implies the year",
    )?;
    g.case(
        "Xmas 2003/photo1.jpg",
        &jpeg(12),
        Review,
        Some(2003),
        &["folder_event_year"],
        "Year in an event folder name is a medium-confidence hint",
    )?;
    for (i, seed) in [(1, 13u32), (2, 14), (3, 15)] {
        let rel = format!("Box_1985_scans/scan{i:03}.jpg");
        let e = g.case(
            &rel,
            &jpeg(seed),
            Review,
            Some(1985),
            &["folder_event_year", "batch"],
            "Batch of scans: confirming one should offer the same year to its siblings",
        )?;
        e.group = Some("batch:Box_1985_scans".into());
    }

    // --- Sidecars -----------------------------------------------------------
    let xmp = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/" xmp:CreateDate="1999-08-15T10:00:00" photoshop:DateCreated="1999-08-15"/></rdf:RDF></x:xmpmeta>"#;
    g.case("Sidecar/photo_x.jpg", &jpeg(16), Auto, Some(1999), &["xmp_sidecar"], "Date only in the .xmp sidecar")?
        .group = Some("photo_x".into());
    g.case("Sidecar/photo_x.xmp", xmp.as_bytes(), Auto, Some(1999), &["sidecar"], "Travels with photo_x.jpg")?.group =
        Some("photo_x".into());

    // Takeout: photoTakenTime is the real date; creationTime is the upload date (a trap).
    let json = format!(
        r#"{{"title":"IMG_1234.jpg","photoTakenTime":{{"timestamp":"{}"}},"creationTime":{{"timestamp":"{}"}}}}"#,
        unix(2016, 6, 27, 4, 0, 0),
        unix(2020, 9, 13, 12, 26, 40)
    );
    g.case(
        "Takeout/Google Photos/Photos from 2016/IMG_1234.jpg",
        &jpeg(17),
        Auto,
        Some(2016),
        &["takeout"],
        "Google Takeout: use photoTakenTime, not creationTime (upload date)",
    )?
    .group = Some("takeout_1234".into());
    g.case(
        "Takeout/Google Photos/Photos from 2016/IMG_1234.jpg.json",
        json.as_bytes(),
        Auto,
        Some(2016),
        &["sidecar", "takeout"],
        "Takeout JSON sidecar",
    )?
    .group = Some("takeout_1234".into());

    // RAW+JPG pair. NEF is TIFF-based, so a TIFF with EXIF is a fair stand-in.
    let nef = build(
        &[(TAG_MAKE, Val::Ascii("NIKON CORPORATION".into())), (TAG_MODEL, Val::Ascii("NIKON D90".into()))],
        &[(TAG_DATETIME_ORIGINAL, Val::Ascii("2013:04:20 16:45:00".into()))],
    );
    let e = Exif {
        make: Some("NIKON CORPORATION"),
        model: Some("NIKON D90"),
        original: Some("2013:04:20 16:45:00"),
        ..Default::default()
    };
    g.case("Raw/DSC_0700.NEF", &nef, Auto, Some(2013), &["raw", "pair"], "RAW file")?.group = Some("DSC_0700".into());
    g.case("Raw/DSC_0700.JPG", &e.jpeg(18), Auto, Some(2013), &["pair"], "JPEG twin of the RAW")?.group =
        Some("DSC_0700".into());

    // --- Video --------------------------------------------------------------
    g.case(
        "Videos/MOV_0001.mp4",
        &mp4::build(b"isom", unix(2012, 7, 4, 18, 0, 0), 5),
        Auto,
        Some(2012),
        &["video", "quicktime_utc"],
        "MP4 mvhd creation time (UTC), far from a year boundary",
    )?;
    g.case(
        "Videos/MOV_0002.mov",
        &mp4::build(b"qt  ", unix(2010, 12, 31, 23, 30, 0), 5),
        Review,
        Some(2010),
        &["video", "quicktime_utc", "year_boundary"],
        "UTC 23:30 on Dec 31 with no time zone: could be 2011 locally (e.g. Europe), so ask",
    )?;

    // --- Collisions, duplicates, Unicode -------------------------------------
    g.case(
        "Backup/DSC_0001.jpg",
        &dsc1,
        Auto,
        Some(2005),
        &["duplicate"],
        "Byte-identical copy of Camera/DSC_0001.jpg",
    )?
    .duplicate_of = Some("Camera/DSC_0001.jpg".into());
    g.case(
        "CardA/DSC_0001.jpg",
        &camera("2005:03:12 14:22:07").jpeg(19),
        Auto,
        Some(2005),
        &["name_collision", "same_timestamp"],
        "Different photo, same name AND same second as Camera/DSC_0001.jpg: must get a unique name, never overwrite",
    )?;
    g.case(
        "Unicode/Zo\u{eb}_caf\u{e9}.jpg",
        &camera("2008:08:08 08:08:08").jpeg(20),
        Auto,
        Some(2008),
        &["unicode_nfc"],
        "Precomposed (NFC) accents",
    )?;
    g.case(
        "Unicode/Zoe\u{308}_cafe\u{301}.jpg",
        &camera("2008:08:08 08:08:09").jpeg(21),
        Auto,
        Some(2008),
        &["unicode_nfd", "name_collision"],
        "Same visible name in decomposed (NFD) form: distinct on NTFS, identical on macOS",
    )?;

    // --- Long path -----------------------------------------------------------
    let deep = (1..=6).map(|i| format!("{i}_{}", "Deep_Folder_From_An_Old_Backup_Drive")).collect::<Vec<_>>().join("/");
    let rel = format!("{deep}/DSC_1111_with_a_rather_long_descriptive_file_name.jpg");
    g.case(
        &rel,
        &camera("2011:11:11 11:11:11").jpeg(22),
        Auto,
        Some(2011),
        &["long_path"],
        "Full path is longer than 260 characters",
    )?;

    // --- Damaged and hostile files --------------------------------------------
    let mut broken = camera("2006:06:06 06:06:06").jpeg(23);
    broken.truncate(broken.len() / 2);
    g.case(
        "Damaged/DSC_0666.jpg",
        &broken,
        Review,
        Some(2006),
        &["truncated", "damaged"],
        "Image data is cut off but EXIF is intact: copy it (preservation) and flag it as damaged",
    )?;
    g.case(
        "Damaged/evil_exif.jpg",
        &insert_into_jpeg(&jpeg(24), &build_malicious()),
        Review,
        None,
        &["malicious_exif", "damaged"],
        "Hostile EXIF (huge count, self-loops): parser must not hang or crash",
    )?;

    // --- Filesystem date only ----------------------------------------------------
    let rel = "Misc/random.jpg";
    let t = Times {
        modified: st(unix(1998, 7, 1, 10, 0, 0)),
        accessed: None,
        created: Some(st(unix(2024, 6, 1, 12, 0, 0))),
    };
    g.put(rel, &jpeg(25), t)?;
    g.entries.push(Entry {
        path: rel.into(),
        route: Review,
        year: Some(1998),
        tags: vec!["fs_mtime_only".into()],
        group: None,
        duplicate_of: None,
        note: "Only the file's modified date (1998) is a clue; creation date is just the copy date".into(),
    });

    // --- OS junk ------------------------------------------------------------------
    g.case(
        "Camera/Thumbs.db",
        &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
        Skip,
        None,
        &["junk"],
        "Windows thumbnail cache",
    )?;
    g.case("Camera/desktop.ini", b"[.ShellClassInfo]\r\n", Skip, None, &["junk"], "Windows folder settings")?;
    g.case("Camera/.DS_Store", b"\0\0\0\x01Bud1", Skip, None, &["junk"], "macOS folder settings")?;

    g.entries.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = Manifest { generator_version: GENERATOR_VERSION, source_dir: SOURCE_DIR.into(), entries: g.entries };
    let json = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
    let mut f = std::fs::File::options().write(true).create_new(true).open(out.join("manifest.json"))?;
    f.write_all(&json)?;
    f.sync_all()?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(unix(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(unix(2005, 3, 12, 14, 22, 7), 1_110_637_327);
        assert_eq!(unix(1904, 1, 1, 0, 0, 0), -2_082_844_800);
    }

    #[test]
    fn generates_and_refuses_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let m = generate(d.path()).unwrap();
        assert!(m.entries.len() >= 30);
        for e in &m.entries {
            let p = d.path().join(SOURCE_DIR).join(&e.path);
            assert!(p.exists(), "missing {}", e.path);
        }
        // duplicate really is byte-identical
        let a = std::fs::read(d.path().join("Old_Photos/Camera/DSC_0001.jpg")).unwrap();
        let b = std::fs::read(d.path().join("Old_Photos/Backup/DSC_0001.jpg")).unwrap();
        assert_eq!(a, b);
        // filesystem dates are the fake "copy date", except the mtime-only case
        let t = cs_platform::read_times(&d.path().join("Old_Photos/Misc/random.jpg")).unwrap();
        assert_eq!(t.modified, st(unix(1998, 7, 1, 10, 0, 0)));
        // second run must refuse
        assert_eq!(generate(d.path()).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn long_path_case_is_long() {
        let d = tempfile::tempdir().unwrap();
        let m = generate(d.path()).unwrap();
        let e = m.entries.iter().find(|e| e.tags.iter().any(|t| t == "long_path")).unwrap();
        assert!(d.path().join(SOURCE_DIR).join(&e.path).as_os_str().len() > 260);
    }
}
