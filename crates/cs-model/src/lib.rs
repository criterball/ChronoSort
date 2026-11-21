//! Shared data types for ChronoSort. No I/O, no OS code.

use serde::{Deserialize, Serialize};

/// What a file is, decided by content sniffing (not by extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    /// .xmp, .json (Takeout), .THM, .aae … travels with its main file.
    Sidecar,
    /// OS clutter such as Thumbs.db, desktop.ini, .DS_Store: never copied.
    Junk,
    Unknown,
}

/// Where a piece of date evidence came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSource {
    ExifOriginal,
    ExifDigitized,
    ExifModify,
    QuickTime,
    XmpSidecar,
    TakeoutJson,
    FileName,
    FolderPath,
    AiDateStamp,
    AiFaces,
    AiEra,
    FsModified,
    FsCreated,
    User,
}

/// How precise a date is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    Decade,
    Year,
    Month,
    Day,
    Second,
}

/// A date that may be known only partly (e.g. just the year from a folder).
/// Times are *local* wall-clock times unless `utc_offset_min` says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialDate {
    pub year: i16,
    pub month: Option<u8>,
    pub day: Option<u8>,
    /// (hour, minute, second)
    pub time: Option<(u8, u8, u8)>,
    /// Offset from UTC in minutes, if the source recorded one.
    pub utc_offset_min: Option<i16>,
    pub precision: Precision,
}

impl PartialDate {
    pub fn year(year: i16) -> Self {
        Self { year, month: None, day: None, time: None, utc_offset_min: None, precision: Precision::Year }
    }

    pub fn ymd_hms(year: i16, month: u8, day: u8, h: u8, mi: u8, s: u8) -> Self {
        Self {
            year,
            month: Some(month),
            day: Some(day),
            time: Some((h, mi, s)),
            utc_offset_min: None,
            precision: Precision::Second,
        }
    }

    /// Basic range validation (does not know about the camera, only the calendar).
    pub fn is_plausible(&self) -> bool {
        let m_ok = self.month.is_none_or(|m| (1..=12).contains(&m));
        let d_ok = self.day.is_none_or(|d| (1..=31).contains(&d));
        let t_ok = self.time.is_none_or(|(h, m, s)| h < 24 && m < 60 && s < 61);
        // Photography starts in 1826 (Niépce).
        (1826..=9999).contains(&self.year) && m_ok && d_ok && t_ok
    }
}

/// One piece of evidence for "when was this taken".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub source: DateSource,
    pub date: PartialDate,
    /// 0.0 ..= 1.0, after sanity rules adjusted it.
    pub weight: f32,
    /// Human-readable reason, shown in the review grid.
    pub note: String,
}

/// Outcome for one media item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "route", rename_all = "snake_case")]
pub enum Decision {
    /// Confident enough to sort without asking.
    Auto { year: i16, confidence: f32 },
    /// Needs the user. `suggested` may be empty.
    Review { suggested: Option<i16>, confidence: f32, reason: String },
    /// Do not copy (junk files).
    Skip { reason: String },
}

/// Copy keeps the source untouched. Move = copy, verify, then send the source
/// to the OS trash (never a hard delete).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferMode {
    Copy,
    Move,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plausibility() {
        assert!(PartialDate::ymd_hms(2005, 3, 12, 14, 22, 7).is_plausible());
        assert!(!PartialDate::ymd_hms(2005, 13, 12, 14, 22, 7).is_plausible());
        assert!(!PartialDate::year(1700).is_plausible());
        assert!(PartialDate::year(1826).is_plausible());
    }

    #[test]
    fn decision_json_shape() {
        let d = Decision::Auto { year: 2005, confidence: 0.97 };
        let s = serde_json::to_string(&d).unwrap();
        assert_eq!(s, r#"{"route":"auto","year":2005,"confidence":0.97}"#);
    }
}

/// Calendar helpers (proleptic Gregorian, UTC). No external date crate needed.
pub mod civil {
    /// Days since 1970-01-01 (H. Hinnant's algorithm).
    pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// Inverse of [`days_from_civil`]: (year, month, day).
    pub fn civil_from_days(z: i64) -> (i64, u8, u8) {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let y = yoe + era * 400 + i64::from(m <= 2);
        (y, m, d)
    }

    pub fn unix(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
        days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s
    }

    /// UTC calendar fields of a unix timestamp: (y, m, d, h, mi, s).
    pub fn from_unix(t: i64) -> (i64, u8, u8, u8, u8, u8) {
        let (y, m, d) = civil_from_days(t.div_euclid(86_400));
        let s = t.rem_euclid(86_400);
        (y, m, d, (s / 3600) as u8, (s % 3600 / 60) as u8, (s % 60) as u8)
    }

    /// Seconds since the epoch of a SystemTime (negative before 1970).
    pub fn system_to_unix(t: std::time::SystemTime) -> i64 {
        match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn roundtrip() {
            for t in [-2_208_988_800i64, -1, 0, 951_782_400, 1_110_637_327, 4_102_444_800] {
                let (y, m, d, h, mi, s) = from_unix(t);
                assert_eq!(unix(y, m.into(), d.into(), h.into(), mi.into(), s.into()), t);
            }
            assert_eq!(from_unix(1_110_637_327), (2005, 3, 12, 14, 22, 7));
        }
    }
}

/// Analysis result for one file, as shown in the review grid and stored in the DB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemReport {
    /// Path relative to the source root, `/`-separated.
    pub rel_path: String,
    pub kind: MediaKind,
    /// Detected container format (by content), e.g. "jpeg", "mp4".
    pub format: String,
    pub size: u64,
    /// Files that travel together share a group key.
    pub group: String,
    /// BLAKE3 of the content, hex.
    pub hash: Option<String>,
    /// First file (by path) with identical content, if this is a duplicate.
    pub duplicate_of: Option<String>,
    pub decision: Decision,
    pub evidence: Vec<Evidence>,
    /// Warnings such as "damaged", "extension_mismatch", "metadata_error".
    pub flags: Vec<String>,
}
