//! Dates hidden in file names and folder names.
//!
//! Uses the `regex` crate, which guarantees linear-time matching (no
//! catastrophic backtracking on hostile names).

use cs_model::{PartialDate, Precision};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    /// Full date + time, e.g. IMG_20050312_142207, 2005-03-12 14.22.07
    DateTime,
    /// Date only, e.g. 2004-05-06 party
    Date,
    /// WhatsApp: IMG-20190101-WA0001 (EXIF stripped by the app)
    WhatsApp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NameDate {
    pub date: PartialDate,
    pub kind: NameKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderKind {
    /// `2005/03/…`: a year folder whose child is a month folder.
    YearMonthTree,
    /// Folder name starting with a date: `2005-03 Holiday`, `2005_03_12`.
    DatePrefix,
    /// Folder named exactly as a year: `2003`.
    YearExact,
    /// Year somewhere in the name: `Xmas 2003`, `Old_1987`, `Photos from 2016`.
    YearToken,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FolderDate {
    pub date: PartialDate,
    pub kind: FolderKind,
    pub folder: String,
}

// Year 1826..=2099. Digits must not touch other digits (avoid matching inside serials).
static WHATSAPP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:IMG|VID|AUD|PTT)-(\d{4})(\d{2})(\d{2})-WA\d+").unwrap());
static DATETIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)(?:^|[^0-9])
          (18[2-9][0-9]|19[0-9]{2}|20[0-9]{2}) [-_.]? (0[1-9]|1[0-2]) [-_.]? (0[1-9]|[12][0-9]|3[01])
          (?: (?:\s+at\s+|[\sT_-]{0,3}) ([01][0-9]|2[0-3]) [-_.:h]? ([0-5][0-9]) [-_.:m]? ([0-5][0-9]) )?
          (?:[^0-9]|$)",
    )
    .unwrap()
});
static YEAR_EXACT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(18[2-9][0-9]|19[0-9]{2}|20[0-9]{2})$").unwrap());
/// Maximal digit runs; a run of exactly 4 digits in range is a year token.
static DIGIT_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9]+").unwrap());
static DATE_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(18[2-9][0-9]|19[0-9]{2}|20[0-9]{2})[-_. ](0[1-9]|1[0-2])(?:[-_. ](0[1-9]|[12][0-9]|3[01]))?(?:[^0-9]|$)",
    )
    .unwrap()
});
static MONTH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:(0?[1-9]|1[0-2])(?:[^0-9].*)?|(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*|(18[2-9][0-9]|19[0-9]{2}|20[0-9]{2})[-_. ](0[1-9]|1[0-2]).*)$",
    )
    .unwrap()
});

fn n<T: std::str::FromStr>(c: &regex::Captures, i: usize) -> Option<T> {
    c.get(i)?.as_str().parse().ok()
}

/// Dates in a file name (extension is ignored).
pub fn name_dates(file_name: &str) -> Vec<NameDate> {
    let stem =
        std::path::Path::new(file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(c) = WHATSAPP.captures(&stem) {
        let d = PartialDate {
            year: n(&c, 1).unwrap_or(0),
            month: n(&c, 2),
            day: n(&c, 3),
            time: None,
            utc_offset_min: None,
            precision: Precision::Day,
        };
        return if d.is_plausible() { vec![NameDate { date: d, kind: NameKind::WhatsApp }] } else { vec![] };
    }
    let mut out = Vec::new();
    for c in DATETIME.captures_iter(&stem).take(4) {
        let mut d = PartialDate {
            year: n(&c, 1).unwrap_or(0),
            month: n(&c, 2),
            day: n(&c, 3),
            time: None,
            utc_offset_min: None,
            precision: Precision::Day,
        };
        let kind = match (n::<u8>(&c, 4), n::<u8>(&c, 5), n::<u8>(&c, 6)) {
            (Some(h), Some(mi), Some(s)) => {
                d.time = Some((h, mi, s));
                d.precision = Precision::Second;
                NameKind::DateTime
            }
            _ => NameKind::Date,
        };
        if d.is_plausible() && valid_day(&d) {
            out.push(NameDate { date: d, kind });
        }
    }
    out
}

fn valid_day(d: &PartialDate) -> bool {
    let (Some(m), Some(day)) = (d.month, d.day) else { return true };
    let leap = (d.year % 4 == 0 && d.year % 100 != 0) || d.year % 400 == 0;
    let max = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    day <= max
}

/// Dates in the folder part of `rel` (relative to the source root, `/`-separated).
/// Only folders *inside* the source are considered, never the source itself.
pub fn folder_dates(rel: &str) -> Vec<FolderDate> {
    let parts: Vec<&str> = rel.split('/').collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    let mut out = Vec::new();
    for (i, dir) in dirs.iter().enumerate() {
        let child = dirs.get(i + 1);
        if let Some(c) = YEAR_EXACT.captures(dir) {
            let year = n(&c, 1).unwrap_or(0);
            let kind = if child.is_some_and(|ch| MONTH.is_match(ch)) {
                FolderKind::YearMonthTree
            } else {
                FolderKind::YearExact
            };
            out.push(FolderDate { date: PartialDate::year(year), kind, folder: dir.to_string() });
        } else if let Some(c) = DATE_PREFIX.captures(dir) {
            let mut d = PartialDate::year(n(&c, 1).unwrap_or(0));
            d.month = n(&c, 2);
            d.precision = Precision::Month;
            if let Some(day) = n(&c, 3) {
                d.day = Some(day);
                d.precision = Precision::Day;
            }
            out.push(FolderDate { date: d, kind: FolderKind::DatePrefix, folder: dir.to_string() });
        } else {
            let years: Vec<i16> = DIGIT_RUN
                .find_iter(dir)
                .filter(|m| m.len() == 4)
                .filter_map(|m| m.as_str().parse().ok())
                .filter(|y| (1826..=2099).contains(y))
                .collect();
            // "1985-1990" style ranges are ambiguous: skip them.
            if let [y] = years[..] {
                out.push(FolderDate {
                    date: PartialDate::year(y),
                    kind: FolderKind::YearToken,
                    folder: dir.to_string(),
                });
            }
        }
    }
    out.retain(|f| f.date.is_plausible());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str) -> Option<(i16, NameKind)> {
        name_dates(name).first().map(|d| (d.date.year, d.kind))
    }

    #[test]
    fn file_names() {
        assert_eq!(one("IMG_20050312_142207.jpg"), Some((2005, NameKind::DateTime)));
        assert_eq!(one("IMG-20190101-WA0001.jpg"), Some((2019, NameKind::WhatsApp)));
        assert_eq!(one("2004-05-06 party.jpg"), Some((2004, NameKind::Date)));
        assert_eq!(one("PXL_20231224_183012345.jpg").map(|x| x.0), Some(2023));
        assert_eq!(one("Screenshot 2021-02-03 at 10.11.12.png"), Some((2021, NameKind::DateTime)));
        assert_eq!(one("DSC_0001.jpg"), None);
        assert_eq!(one("IMG_4521.jpg"), None);
        assert_eq!(one("20050230.jpg"), None, "Feb 30 is not a date");
        assert_eq!(one("serial_120050312999.jpg"), None, "digits glued to other digits");
    }

    #[test]
    fn folders() {
        let k = |rel: &str| folder_dates(rel).into_iter().map(|f| (f.date.year, f.kind)).collect::<Vec<_>>();
        assert_eq!(k("2005/03/scan_001.jpg"), vec![(2005, FolderKind::YearMonthTree)]);
        assert_eq!(k("2005/March/a.jpg"), vec![(2005, FolderKind::YearMonthTree)]);
        assert_eq!(k("2005/2005-03 Holiday/a.jpg")[0], (2005, FolderKind::YearMonthTree));
        assert_eq!(k("2003/DSC_0100.jpg"), vec![(2003, FolderKind::YearExact)]);
        assert_eq!(k("Xmas 2003/photo1.jpg"), vec![(2003, FolderKind::YearToken)]);
        assert_eq!(k("Box_1985_scans/s.jpg"), vec![(1985, FolderKind::YearToken)]);
        assert_eq!(k("2005-03 Holiday/a.jpg"), vec![(2005, FolderKind::DatePrefix)]);
        assert_eq!(k("1985-1990 mixed/a.jpg"), vec![]);
        assert_eq!(k("Camera/DSC_0001.jpg"), vec![]);
        assert_eq!(k("2005.jpg"), vec![], "the file name itself is not a folder");
    }
}
