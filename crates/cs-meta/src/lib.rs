//! `cs-meta`: extract raw date facts from a file, its name and its folders.
//! No weighting or decisions happen here (that is `cs-date`), only reading.
//!
//! All parsers are bounded: they read at most a fixed number of bytes, walk a
//! fixed number of boxes/segments, and treat any inconsistency as "no data".

use cs_model::PartialDate;
use cs_scan::{FileEntry, Format};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};

mod damage;
mod exif;
pub mod names;
pub mod preview;
mod qt;
mod sidecar;

pub use names::{FolderDate, FolderKind, NameDate, NameKind};

/// Camera metadata from EXIF.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExifInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub software: Option<String>,
    /// DateTimeOriginal (+ OffsetTimeOriginal)
    pub original: Option<PartialDate>,
    /// DateTimeDigitized / CreateDate
    pub digitized: Option<PartialDate>,
    /// DateTime (0x0132): last modification, often by editing software.
    pub modify: Option<PartialDate>,
}

/// Everything we could read about one file. Missing = `None`, never guessed.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub exif: Option<ExifInfo>,
    /// QuickTime/MP4 `mvhd` creation time, seconds since 1970 **UTC**.
    pub qt_created_utc: Option<i64>,
    /// Apple `com.apple.quicktime.creationdate`: local capture time with offset.
    pub qt_local: Option<PartialDate>,
    /// Dates from an XMP sidecar file.
    pub xmp: Vec<PartialDate>,
    /// Dates from the XMP packet inside a JPEG (written by editors and scanners).
    pub xmp_embedded: Vec<PartialDate>,
    /// Google Takeout `photoTakenTime`, seconds since 1970 UTC.
    pub takeout_taken_utc: Option<i64>,
    pub name_dates: Vec<NameDate>,
    pub folder_dates: Vec<FolderDate>,
    pub fs_modified: Option<i64>,
    pub fs_created: Option<i64>,
    /// Set when the file is structurally broken (truncated, missing end marker).
    pub damaged: Option<String>,
    /// Non-fatal problems ("EXIF unreadable: …").
    pub warnings: Vec<String>,
}

/// Max bytes handed to the EXIF parser for TIFF-based files (RAW can be huge,
/// but EXIF sits near the start). Prevents reading multi-GB files into RAM.
const TIFF_EXIF_LIMIT: u64 = 64 << 20;
/// Sidecars larger than this are ignored (XMP/JSON are normally a few KB).
const SIDECAR_LIMIT: u64 = 4 << 20;

/// Read all facts for one scanned file.
pub fn read_facts(f: &FileEntry) -> Facts {
    let mut facts = Facts {
        name_dates: names::name_dates(&f.name),
        folder_dates: names::folder_dates(&f.rel),
        ..Default::default()
    };
    if let Ok(t) = cs_platform::read_times(&f.abs) {
        facts.fs_modified = Some(cs_model::civil::system_to_unix(t.modified));
        facts.fs_created = t.created.map(cs_model::civil::system_to_unix);
    }
    if let Err(e) = read_content(f, &mut facts) {
        facts.warnings.push(format!("read error: {e}"));
    }
    facts
}

fn read_content(f: &FileEntry, facts: &mut Facts) -> io::Result<()> {
    let mut file = File::open(&f.abs)?;
    match f.format {
        Format::Jpeg | Format::Png | Format::Webp | Format::Heif | Format::Avif | Format::Tiff => {
            match exif::read(&mut file, f.format, TIFF_EXIF_LIMIT) {
                Ok(Some(e)) => facts.exif = Some(e),
                Ok(None) => {}
                Err(e) => facts.warnings.push(format!("metadata_error: EXIF unreadable ({e})")),
            }
            if f.format == Format::Jpeg {
                file.seek(SeekFrom::Start(0))?;
                if let Some(x) = sidecar::jpeg_xmp(&mut BufReader::new(&mut file))? {
                    facts.xmp_embedded = sidecar::xmp_dates(&x);
                }
            }
            file.seek(SeekFrom::Start(0))?;
            facts.damaged = damage::check(&mut file, f.format, f.size)?;
        }
        Format::Mp4 | Format::Mov | Format::ThreeGp | Format::Cr3 => {
            facts.qt_created_utc = qt::mvhd_created_utc(&mut BufReader::new(&mut file), f.size)?;
            facts.qt_local = qt::apple_creationdate(&mut BufReader::new(&mut file), f.size)?
                .and_then(|s| sidecar::parse_xmp_date(&s))
                .filter(|d| d.time.is_some());
        }
        Format::Xmp => facts.xmp = sidecar::xmp_dates(&read_limited(&mut file)?),
        Format::Json => facts.takeout_taken_utc = sidecar::takeout_taken(&read_limited(&mut file)?),
        _ => {}
    }
    Ok(())
}

/// The largest JPEG preview inside a RAW file (read at most 64 MiB).
pub fn raw_preview(path: &std::path::Path) -> Option<Vec<u8>> {
    preview::raw_preview(path, TIFF_EXIF_LIMIT)
}

/// EXIF orientation (1–8) and embedded JPEG thumbnail of an image file.
pub fn orientation_and_thumbnail(path: &std::path::Path, format: Format) -> (Option<u16>, Option<Vec<u8>>) {
    match File::open(path) {
        Ok(mut f) => exif::orientation_and_thumbnail(&mut f, format, TIFF_EXIF_LIMIT),
        Err(_) => (None, None),
    }
}

fn read_limited(file: &mut File) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    file.take(SIDECAR_LIMIT).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Parse an EXIF date string "YYYY:MM:DD HH:MM:SS" (also tolerates '-' and
/// missing seconds). Blank or zero dates ("0000:00:00 00:00:00") give None.
pub fn parse_exif_datetime(s: &str) -> Option<PartialDate> {
    let b = s.trim().trim_end_matches('\0').as_bytes();
    if b.len() < 10 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let t = b.get(r)?;
        if t.iter().all(u8::is_ascii_digit) { std::str::from_utf8(t).ok()?.parse().ok() } else { None }
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let mut pd = PartialDate {
        year: y as i16,
        month: Some(mo as u8),
        day: Some(d as u8),
        time: None,
        utc_offset_min: None,
        precision: cs_model::Precision::Day,
    };
    if b.len() >= 16
        && let (Some(h), Some(mi)) = (num(11..13), num(14..16))
    {
        let sec = if b.len() >= 19 { num(17..19).unwrap_or(0) } else { 0 };
        pd.time = Some((h as u8, mi as u8, sec as u8));
        pd.precision = cs_model::Precision::Second;
    }
    (y != 0 && pd.is_plausible()).then_some(pd)
}

/// "+01:00" / "-05:30" / "Z" → minutes.
pub fn parse_offset(s: &str) -> Option<i16> {
    let s = s.trim().trim_end_matches('\0');
    if s == "Z" {
        return Some(0);
    }
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    let (h, m) = rest.split_once(':').unwrap_or((rest.get(..2)?, rest.get(2..).unwrap_or("0")));
    let (h, m): (i16, i16) = (h.parse().ok()?, if m.is_empty() { 0 } else { m.parse().ok()? });
    ((0..=14).contains(&h) && (0..60).contains(&m)).then_some(sign * (h * 60 + m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_dates() {
        let d = parse_exif_datetime("2005:03:12 14:22:07").unwrap();
        assert_eq!((d.year, d.month, d.day, d.time), (2005, Some(3), Some(12), Some((14, 22, 7))));
        assert!(parse_exif_datetime("0000:00:00 00:00:00").is_none());
        assert!(parse_exif_datetime("    :  :     :  :  ").is_none());
        assert!(parse_exif_datetime("2005:13:12 14:22:07").is_none());
        assert_eq!(parse_exif_datetime("2005-03-12").unwrap().precision, cs_model::Precision::Day);
        assert_eq!(parse_exif_datetime("2005:03:12 14:22").unwrap().time, Some((14, 22, 0)));
    }

    #[test]
    fn offsets() {
        assert_eq!(parse_offset("+01:00"), Some(60));
        assert_eq!(parse_offset("-05:30"), Some(-330));
        assert_eq!(parse_offset("+0930"), Some(570));
        assert_eq!(parse_offset("Z"), Some(0));
        assert_eq!(parse_offset("+25:00"), None);
        assert_eq!(parse_offset("garbage"), None);
    }
}
