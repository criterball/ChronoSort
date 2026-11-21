//! `cs-date`: turn raw facts into weighted evidence and one decision per group.
//!
//! All weights live here so they can be tuned in one place and are covered by
//! the corpus test (every file in `cs-testgen` must land where expected).

use cs_meta::{Facts, FolderKind, NameKind};
use cs_model::civil::from_unix;
use cs_model::{DateSource, Decision, Evidence, PartialDate, Precision};
use std::collections::BTreeMap;

mod cameras;

#[derive(Debug, Clone)]
pub struct Policy {
    /// Minimum combined score to sort without asking.
    pub auto_min: f32,
    /// Any contradicting evidence at or above this weight forces review.
    pub conflict_max: f32,
    /// Below this, no year is suggested in the review grid.
    pub suggest_min: f32,
    /// "Now", for rejecting future dates (unix seconds).
    pub now_unix: i64,
}

impl Policy {
    pub fn new(now_unix: i64) -> Self {
        Self { auto_min: 0.9, conflict_max: 0.5, suggest_min: 0.25, now_unix }
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub decision: Decision,
    pub evidence: Vec<Evidence>,
    pub flags: Vec<String>,
}

fn ev(source: DateSource, date: PartialDate, weight: f32, note: impl Into<String>) -> Evidence {
    Evidence { source, date, weight, note: note.into() }
}

fn utc_date(t: i64) -> PartialDate {
    let (y, m, d, h, mi, s) = from_unix(t);
    let mut p = PartialDate::ymd_hms(y as i16, m, d, h, mi, s);
    p.utc_offset_min = Some(0);
    p
}

/// A UTC timestamp whose local year is unknown: if every real-world time zone
/// (UTC-12 … UTC+14) gives the same year, it is safe; otherwise it is not.
fn utc_evidence(source: DateSource, t: i64, safe_w: f32, what: &str) -> Evidence {
    let lo = from_unix(t - 12 * 3600).0;
    let hi = from_unix(t + 14 * 3600).0;
    if lo == hi {
        ev(source, utc_date(t), safe_w, format!("{what} (UTC)"))
    } else {
        ev(source, utc_date(t), 0.5, format!("{what} is UTC near New Year: local year could be {lo} or {hi}"))
    }
}

/// Camera clocks that were never set start at a default such as 2000-01-01 00:00.
fn is_reset_date(d: &PartialDate) -> bool {
    let jan1 = d.month == Some(1) && d.day == Some(1);
    let near_midnight = d.time.is_some_and(|(h, m, _)| h == 0 && m < 10);
    jan1 && (near_midnight || matches!(d.year, 1970 | 1980))
}

fn exif_evidence(f: &Facts, out: &mut Vec<Evidence>) {
    let Some(e) = &f.exif else { return };
    let scanner = cameras::is_scanner(e);
    let release = cameras::release_year(e);
    let items = [
        (DateSource::ExifOriginal, e.original, 0.95, "EXIF DateTimeOriginal"),
        (DateSource::ExifDigitized, e.digitized, 0.90, "EXIF CreateDate"),
        (DateSource::ExifModify, e.modify, 0.60, "EXIF ModifyDate"),
    ];
    for (src, date, base, label) in items {
        let Some(date) = date else { continue };
        let (w, note) = if scanner {
            (0.05, format!("{label}: scanner, so this is the scan date, not the photo date"))
        } else if is_reset_date(&date) {
            (0.1, format!("{label}: looks like an unset camera clock ({}-01-01)", date.year))
        } else if let Some(r) = release.filter(|r| date.year < *r) {
            (0.0, format!("{label}: impossible, this camera model was released in {r}"))
        } else {
            (base, label.to_string())
        };
        out.push(ev(src, date, w, note));
    }
}

/// Gather weighted evidence from one file's facts.
pub fn collect(f: &Facts) -> Vec<Evidence> {
    let mut out = Vec::new();
    exif_evidence(f, &mut out);
    // Apple's local capture time is the same moment as the UTC one, but in the
    // phone's own time zone: prefer it (the UTC day can differ, even the year).
    if let Some(d) = f.qt_local {
        out.push(ev(DateSource::QuickTime, d, 0.95, "video capture time (iPhone, local time)"));
    } else if let Some(t) = f.qt_created_utc {
        out.push(utc_evidence(DateSource::QuickTime, t, 0.92, "video creation time"));
    }
    if let Some(t) = f.takeout_taken_utc {
        out.push(utc_evidence(DateSource::TakeoutJson, t, 0.93, "Google Takeout photoTakenTime"));
    }
    for d in &f.xmp {
        out.push(ev(DateSource::XmpSidecar, *d, 0.92, "XMP sidecar date"));
    }
    for d in &f.xmp_embedded {
        out.push(ev(DateSource::XmpSidecar, *d, 0.9, "XMP date inside the photo"));
    }
    for n in &f.name_dates {
        let (w, what) = match n.kind {
            NameKind::DateTime => (0.92, "file name contains date and time"),
            NameKind::WhatsApp => (0.92, "WhatsApp file name (app strips EXIF)"),
            NameKind::Date => (0.90, "file name contains a date"),
        };
        out.push(ev(DateSource::FileName, n.date, w, what));
    }
    for fd in &f.folder_dates {
        let w = match fd.kind {
            FolderKind::YearMonthTree => 0.92,
            FolderKind::DatePrefix => 0.85,
            FolderKind::YearExact => 0.75,
            FolderKind::YearToken => 0.6,
        };
        out.push(ev(DateSource::FolderPath, fd.date, w, format!("folder \"{}\"", fd.folder)));
    }
    if let Some(m) = f.fs_modified {
        let preserved = f.fs_created.is_some_and(|c| m < c - 60);
        let (w, note) = if preserved {
            (0.3, "file modified date is older than its creation date (original date was kept)")
        } else {
            (0.1, "file modified date (often just the copy date)")
        };
        out.push(ev(DateSource::FsModified, utc_date(m), w, note));
    }
    if let Some(c) = f.fs_created {
        out.push(ev(DateSource::FsCreated, utc_date(c), 0.05, "file creation date (usually the copy date)"));
    }
    out
}

/// Decide the year for a group of files that travel together.
pub fn decide(group: &[&Facts], policy: &Policy) -> Outcome {
    let now_year = from_unix(policy.now_unix).0 as i16;
    let mut flags = Vec::new();
    let mut evidence: Vec<Evidence> = Vec::new();
    for f in group {
        for mut e in collect(f) {
            if e.date.year > now_year || !e.date.is_plausible() {
                e.weight = 0.0;
                e.note = format!("{}: date {} is in the future or impossible", e.note, e.date.year);
            }
            // Same fact from two members (shared folder, RAW+JPEG twins): keep the strongest.
            let dup = evidence.iter_mut().find(|x| x.source == e.source && x.date == e.date);
            match dup {
                Some(x) if x.weight >= e.weight => {}
                Some(x) => *x = e,
                None => evidence.push(e),
            }
        }
        for w in &f.warnings {
            if !flags.contains(w) {
                flags.push(w.clone());
            }
        }
    }
    let damaged: Vec<&String> = group.iter().filter_map(|f| f.damaged.as_ref()).collect();
    for d in &damaged {
        flags.push((*d).clone());
    }

    // Noisy-OR per year: independent sources agreeing raise confidence.
    let mut miss: BTreeMap<i16, f32> = BTreeMap::new();
    for e in evidence.iter().filter(|e| e.weight > 0.0) {
        *miss.entry(e.date.year).or_insert(1.0) *= 1.0 - e.weight;
    }
    let best = miss.iter().map(|(y, m)| (*y, 1.0 - m)).max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
    evidence.sort_by(|a, b| b.weight.total_cmp(&a.weight));

    let Some((year, score)) = best else {
        let decision =
            Decision::Review { suggested: None, confidence: 0.0, reason: "no date information at all".into() };
        return Outcome { decision, evidence, flags };
    };
    let against = evidence.iter().filter(|e| e.date.year != year).map(|e| e.weight).fold(0.0f32, f32::max);
    let confidence = score * (1.0 - against);
    let suggested = (score >= policy.suggest_min).then_some(year);

    let decision = if let Some(d) = damaged.first() {
        Decision::Review { suggested, confidence, reason: (*d).clone() }
    } else if score >= policy.auto_min && against < policy.conflict_max {
        Decision::Auto { year, confidence }
    } else {
        let reason = if against >= policy.conflict_max {
            let other = evidence.iter().find(|e| e.date.year != year).map(|e| e.date.year).unwrap_or(year);
            format!("conflicting dates: {year} vs {other}")
        } else {
            // Explain: what was distrusted, and what the suggestion rests on.
            let mut parts: Vec<String> = Vec::new();
            for e in evidence.iter().filter(|e| e.weight <= 0.1 && !is_fs(e.source)) {
                // "EXIF CreateDate: looks like …" -> "EXIF: looks like …" (one line per reason)
                let why = e.note.split_once(": ").map_or(e.note.as_str(), |(_, w)| w);
                let line = if e.note.starts_with("EXIF") { format!("EXIF {why}") } else { e.note.clone() };
                if !parts.contains(&line) {
                    parts.push(line);
                }
            }
            let basis = evidence.iter().find(|e| e.date.year == year && e.weight > 0.0).map(|e| e.note.as_str());
            match (suggested, basis) {
                (Some(_), Some(b)) => parts.push(format!("best guess {year} from {b}")),
                _ => parts.push("no reliable date found".into()),
            }
            parts.join("; ")
        };
        Decision::Review { suggested, confidence, reason }
    };
    Outcome { decision, evidence, flags }
}

fn is_fs(s: DateSource) -> bool {
    matches!(s, DateSource::FsModified | DateSource::FsCreated)
}

/// Precision of the strongest evidence for the decided year (for the UI).
pub fn best_precision(o: &Outcome) -> Option<Precision> {
    let y = match &o.decision {
        Decision::Auto { year, .. } => *year,
        Decision::Review { suggested: Some(y), .. } => *y,
        _ => return None,
    };
    o.evidence.iter().find(|e| e.date.year == y).map(|e| e.date.precision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cs_meta::ExifInfo;
    use cs_model::civil::unix;

    fn now() -> Policy {
        Policy::new(unix(2026, 11, 20, 0, 0, 0))
    }

    fn exif(orig: &str, model: &str) -> Facts {
        Facts {
            exif: Some(ExifInfo {
                model: Some(model.into()),
                original: cs_meta::parse_exif_datetime(orig),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn plain_exif_is_auto() {
        let o = decide(&[&exif("2005:03:12 14:22:07", "Canon EOS 300D DIGITAL")], &now());
        assert!(matches!(o.decision, Decision::Auto { year: 2005, .. }));
    }

    #[test]
    fn anachronism_and_future_are_rejected() {
        let o = decide(&[&exif("2015:06:01 12:00:00", "iPhone 12")], &now());
        assert!(matches!(o.decision, Decision::Review { suggested: None, .. }));
        let o = decide(&[&exif("2099:01:01 08:00:00", "Canon EOS 300D DIGITAL")], &now());
        assert!(matches!(o.decision, Decision::Review { suggested: None, .. }));
    }

    #[test]
    fn iphone_local_time_settles_new_year() {
        // 23:30 on 31 Dec 2010 in Sydney is already 2011 in local time; UTC says 2010.
        let local = cs_meta::parse_exif_datetime("2011:01:01 09:30:00").map(|mut d| {
            d.utc_offset_min = Some(600);
            d
        });
        let f = Facts { qt_created_utc: Some(unix(2010, 12, 31, 23, 30, 0)), qt_local: local, ..Default::default() };
        assert!(matches!(decide(&[&f], &now()).decision, Decision::Auto { year: 2011, .. }));
    }

    #[test]
    fn utc_new_year_is_ambiguous() {
        let f = Facts { qt_created_utc: Some(unix(2010, 12, 31, 23, 30, 0)), ..Default::default() };
        assert!(matches!(decide(&[&f], &now()).decision, Decision::Review { suggested: Some(2010), .. }));
        let f = Facts { qt_created_utc: Some(unix(2012, 7, 4, 18, 0, 0)), ..Default::default() };
        assert!(matches!(decide(&[&f], &now()).decision, Decision::Auto { year: 2012, .. }));
    }

    #[test]
    fn nothing_known() {
        assert!(matches!(decide(&[&Facts::default()], &now()).decision, Decision::Review { suggested: None, .. }));
    }
}
