//! `cs-plan`: decide the destination of every file *before* anything is written.
//!
//! * Layout: `<year>/<name>`; month sub-folders from the source are kept only
//!   if the user says so (the plan raises a [`MonthQuestion`] first).
//! * Names: keep the original by default; on a clash append `_1`, `_2`, …
//!   Optional renaming by date/time or a template. Groups (RAW+JPEG,
//!   sidecars, Live Photos) always keep matching names.
//! * Duplicates: identical content is planned once. Files already present in
//!   the destination (earlier runs) are not copied again.
//!
//! Pure planning: reads the destination, never writes anything.

use cs_model::{Decision, ItemReport, MediaKind, Precision};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

mod naming;
pub use naming::{Naming, NamingError};

/// Folder ChronoSort keeps inside the destination (journal, reports).
pub const STATE_DIR: &str = ".chronosort";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MonthPolicy {
    /// Raise a question if the source has month folders (default).
    Ask,
    /// Everything straight into the year folder.
    Flatten,
    /// Keep the source's month folder name under the year: `2005/March/…`.
    Keep,
}

#[derive(Debug, Clone)]
pub struct PlanOptions {
    pub months: MonthPolicy,
    pub naming: Naming,
    /// Accept the suggested year for review items (CLI/testing; the app uses `resolutions`).
    pub accept_suggested: bool,
    /// Years confirmed by the user, keyed by source rel path (any member of a group).
    pub resolutions: HashMap<String, i16>,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self { months: MonthPolicy::Ask, naming: Naming::Keep, accept_suggested: false, resolutions: HashMap::new() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedFile {
    pub src_rel: String,
    /// Destination path relative to the destination root, `/`-separated.
    pub dst_rel: String,
    pub size: u64,
    pub hash: Option<String>,
    pub group: String,
    pub year: i16,
    /// The wanted name was taken, so `_1`, `_2`, … was added: by a file that is
    /// already in the destination ("destination") or by another file of this
    /// run ("this_run"). None = got the wanted name.
    pub name_taken_by: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum Skipped {
    /// Identical content is already planned from another source path.
    Duplicate {
        src_rel: String,
        same_as: String,
    },
    /// Identical content already exists in the destination.
    AlreadyInDestination {
        src_rel: String,
        existing: String,
    },
    /// Needs a user decision first.
    Unresolved {
        src_rel: String,
        suggested: Option<i16>,
        reason: String,
    },
    Junk {
        src_rel: String,
    },
}

/// The source is organised in month folders: ask before flattening.
#[derive(Debug, Clone, Serialize)]
pub struct MonthQuestion {
    /// ("2005/03", number of files)
    pub folders: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub files: Vec<PlannedFile>,
    pub skipped: Vec<Skipped>,
    /// Must be answered (re-plan with Keep/Flatten) before executing.
    pub month_question: Option<MonthQuestion>,
    pub total_bytes: u64,
    pub notes: Vec<String>,
}

impl Plan {
    pub fn ready(&self) -> bool {
        self.month_question.is_none()
    }
    pub fn bytes_saved_by_dedup(&self, items: &[ItemReport]) -> u64 {
        let size: HashMap<&str, u64> = items.iter().map(|i| (i.rel_path.as_str(), i.size)).collect();
        self.skipped
            .iter()
            .filter_map(|s| match s {
                Skipped::Duplicate { src_rel, .. } | Skipped::AlreadyInDestination { src_rel, .. } => {
                    size.get(src_rel.as_str())
                }
                _ => None,
            })
            .sum()
    }
}

/// Comparison key for names: NFC + lower case, so `Zoë` (NFC) and `Zoë`
/// (NFD), or `IMG.JPG` and `img.jpg`, are treated as the same name. This
/// matches case-insensitive Windows/macOS filesystems and avoids clashes when
/// the sorted library is later copied to another OS.
pub fn name_key(rel: &str) -> String {
    rel.nfc().collect::<String>().to_lowercase()
}

/// Existing destination content: name keys and, for sizes that also occur
/// in the source, content hashes.
struct DestIndex {
    names: HashSet<String>,
    by_hash: HashMap<String, String>,
}

fn index_destination(dest: &Path, source_sizes: &HashSet<u64>) -> io::Result<DestIndex> {
    let mut idx = DestIndex { names: HashSet::new(), by_hash: HashMap::new() };
    if !dest.exists() {
        return Ok(idx);
    }
    let walk = walkdir::WalkDir::new(dest).follow_links(false).into_iter().filter_entry(|e| e.file_name() != STATE_DIR);
    for ent in walk {
        let ent = ent.map_err(io::Error::other)?;
        let rel = ent
            .path()
            .strip_prefix(dest)
            .map_err(io::Error::other)?
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if rel.is_empty() {
            continue;
        }
        idx.names.insert(name_key(&rel)); // folders too: a file may not take a folder's name
        if ent.file_type().is_file() {
            let len = ent.metadata().map_err(io::Error::other)?.len();
            if source_sizes.contains(&len) {
                idx.by_hash.insert(cs_core::hash_file(ent.path())?, rel);
            }
        }
    }
    Ok(idx)
}

/// The month folder below a `YYYY` folder in the source path, if the file
/// lives in a year/month tree for `year`.
fn month_folder(src_rel: &str, year: i16) -> Option<(String, String)> {
    let parts: Vec<&str> = src_rel.split('/').collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    let fd = cs_meta::names::folder_dates(src_rel)
        .into_iter()
        .find(|f| f.kind == cs_meta::FolderKind::YearMonthTree && f.date.year == year)?;
    let i = dirs.iter().position(|d| *d == fd.folder)?;
    let month = dirs.get(i + 1)?;
    Some((dirs[..=i + 1].join("/"), month.to_string()))
}

fn year_of(d: &Decision, accept_suggested: bool) -> Option<i16> {
    match d {
        Decision::Auto { year, .. } => Some(*year),
        Decision::Review { suggested: Some(y), .. } if accept_suggested => Some(*y),
        _ => None,
    }
}

/// Pick the group member whose name/date drives the group's new name.
fn main_member<'a>(members: &[&'a ItemReport]) -> &'a ItemReport {
    let rank = |i: &ItemReport| match (i.kind, i.format.as_str()) {
        (MediaKind::Image, "tiff" | "cr3") => 1,
        (MediaKind::Image, _) => 0,
        (MediaKind::Video, _) => 2,
        _ => 3,
    };
    members.iter().min_by_key(|i| (rank(i), i.rel_path.clone())).unwrap()
}

fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Split "DSC_0001.jpg" -> ("DSC_0001", ".jpg"). Dot-files keep their name as stem.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

pub fn build(items: &[ItemReport], dest: &Path, opts: &PlanOptions) -> io::Result<Plan> {
    let mut plan = Plan { files: vec![], skipped: vec![], month_question: None, total_bytes: 0, notes: vec![] };

    // ---- duplicates first (no redundant data) --------------------------------------
    let sizes: HashSet<u64> = items.iter().filter(|i| i.kind != MediaKind::Junk).map(|i| i.size).collect();
    let dest_idx = index_destination(dest, &sizes)?;
    let mut groups: BTreeMap<&str, Vec<&ItemReport>> = BTreeMap::new();
    for i in items {
        if i.kind == MediaKind::Junk {
            plan.skipped.push(Skipped::Junk { src_rel: i.rel_path.clone() });
        } else {
            groups.entry(i.group.as_str()).or_default().push(i);
        }
    }

    let mut taken: HashSet<String> = dest_idx.names.clone();
    let mut months_seen: BTreeMap<String, usize> = BTreeMap::new();
    // Deterministic order: by first source path in the group.
    let mut ordered: Vec<Vec<&ItemReport>> = groups.into_values().collect();
    ordered.sort_by(|a, b| a[0].rel_path.cmp(&b[0].rel_path));

    for members in ordered {
        // A group is skipped only if *every* member is redundant; if anything
        // differs (e.g. an edited sidecar) the whole group is kept together.
        let redundancy: Vec<Option<Skipped>> = members
            .iter()
            .map(|m| {
                if let Some(e) = m.hash.as_ref().and_then(|h| dest_idx.by_hash.get(h)) {
                    Some(Skipped::AlreadyInDestination { src_rel: m.rel_path.clone(), existing: e.clone() })
                } else {
                    m.duplicate_of
                        .as_ref()
                        .map(|d| Skipped::Duplicate { src_rel: m.rel_path.clone(), same_as: d.clone() })
                }
            })
            .collect();
        if redundancy.iter().all(Option::is_some) {
            plan.skipped.extend(redundancy.into_iter().flatten());
            continue;
        }

        // ---- year -------------------------------------------------------------------
        let main = main_member(&members);
        let resolved = members.iter().find_map(|m| opts.resolutions.get(&m.rel_path).copied());
        let Some(year) = resolved.or_else(|| year_of(&main.decision, opts.accept_suggested)) else {
            for m in &members {
                let (suggested, reason) = match &m.decision {
                    Decision::Review { suggested, reason, .. } => (*suggested, reason.clone()),
                    _ => (None, "no decision".into()),
                };
                plan.skipped.push(Skipped::Unresolved { src_rel: m.rel_path.clone(), suggested, reason });
            }
            continue;
        };

        // ---- folder -----------------------------------------------------------------
        let mut dir = year.to_string();
        if let Some((tree, month)) = month_folder(&main.rel_path, year) {
            *months_seen.entry(tree).or_default() += members.len();
            if opts.months == MonthPolicy::Keep {
                dir = format!("{year}/{}", naming::sanitize(&month));
            }
        }

        // ---- names ------------------------------------------------------------------
        let main_name = file_name(&main.rel_path);
        let (main_stem, _) = split_ext(main_name);
        let new_stem = opts.naming.stem_for(main, year).unwrap_or_else(|| main_stem.to_string());
        let member_names = |stem: &str| -> Vec<String> {
            members
                .iter()
                .map(|m| {
                    let name = file_name(&m.rel_path);
                    // Keep whatever follows the main stem: ".jpg", ".NEF", ".jpg.json", ".xmp"
                    let tail =
                        if name.len() >= main_stem.len() && name[..main_stem.len()].eq_ignore_ascii_case(main_stem) {
                            &name[main_stem.len()..]
                        } else {
                            split_ext(name).1
                        };
                    format!("{stem}{tail}")
                })
                .collect()
        };
        let mut names = member_names(&new_stem);
        let free = |names: &[String], taken: &HashSet<String>| {
            let keys: Vec<String> = names.iter().map(|n| name_key(&format!("{dir}/{n}"))).collect();
            let unique: HashSet<&String> = keys.iter().collect();
            unique.len() == keys.len() && keys.iter().all(|k| !taken.contains(k))
        };
        // Why the name had to change, for the preview.
        let taken_by = (!free(&names, &taken)).then(|| {
            let in_dest = names.iter().any(|nm| dest_idx.names.contains(&name_key(&format!("{dir}/{nm}"))));
            if in_dest { "destination" } else { "this_run" }
        });
        let mut n = 0;
        while !free(&names, &taken) {
            n += 1;
            names = member_names(&format!("{new_stem}_{n}"));
        }
        for (m, name) in members.iter().zip(names) {
            let dst_rel = format!("{dir}/{name}");
            taken.insert(name_key(&dst_rel));
            plan.total_bytes += m.size;
            plan.files.push(PlannedFile {
                src_rel: m.rel_path.clone(),
                dst_rel,
                size: m.size,
                hash: m.hash.clone(),
                group: m.group.clone(),
                year,
                name_taken_by: taken_by,
            });
        }
    }
    if !months_seen.is_empty() && opts.months == MonthPolicy::Ask {
        plan.month_question = Some(MonthQuestion { folders: months_seen.into_iter().collect() });
    }
    if matches!(opts.naming, Naming::DateTime | Naming::Template(_)) {
        plan.notes.push("files without a known date and time keep their original name".into());
    }
    Ok(plan)
}

/// Up to `n` examples of (current name, new name) for a naming choice, taken
/// from files with a confident year (for the live preview in the app).
pub fn naming_preview(items: &[ItemReport], naming: &Naming, n: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in items.iter().filter(|i| matches!(i.kind, MediaKind::Image | MediaKind::Video)) {
        let Decision::Auto { year, .. } = i.decision else { continue };
        let name = file_name(&i.rel_path);
        let (stem, ext) = split_ext(name);
        let new_stem = naming.stem_for(i, year).unwrap_or_else(|| stem.to_string());
        out.push((name.to_string(), format!("{new_stem}{ext}")));
        if out.len() == n {
            break;
        }
    }
    out
}

/// The date/time of the strongest evidence for `year` (used for renaming).
pub(crate) fn best_datetime(item: &ItemReport, year: i16) -> Option<&cs_model::PartialDate> {
    item.evidence
        .iter()
        .filter(|e| e.date.year == year && e.weight > 0.0)
        .max_by(|a, b| a.weight.total_cmp(&b.weight).then(a.date.precision.cmp(&b.date.precision)))
        .map(|e| &e.date)
        .filter(|d| d.precision >= Precision::Day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cs_model::{DateSource, Evidence, PartialDate};

    fn item(rel: &str, year: i16, hash: &str) -> ItemReport {
        ItemReport {
            rel_path: rel.into(),
            kind: MediaKind::Image,
            format: "jpeg".into(),
            size: 10,
            group: rel.into(),
            hash: Some(hash.into()),
            duplicate_of: None,
            decision: Decision::Auto { year, confidence: 0.95 },
            evidence: vec![Evidence {
                source: DateSource::ExifOriginal,
                date: PartialDate::ymd_hms(year, 3, 12, 14, 22, 7),
                weight: 0.95,
                note: String::new(),
            }],
            flags: vec![],
        }
    }

    fn dsts(p: &Plan) -> Vec<&str> {
        p.files.iter().map(|f| f.dst_rel.as_str()).collect()
    }

    #[test]
    fn keeps_names_and_numbers_clashes() {
        let items = vec![
            item("A/DSC_0001.jpg", 2005, "a"),
            item("B/DSC_0001.jpg", 2005, "b"),
            item("C/dsc_0001.JPG", 2005, "c"),
        ];
        let d = tempfile::tempdir().unwrap();
        let p = build(&items, d.path(), &PlanOptions::default()).unwrap();
        assert_eq!(dsts(&p), ["2005/DSC_0001.jpg", "2005/DSC_0001_1.jpg", "2005/dsc_0001_2.JPG"]);
        let why: Vec<_> = p.files.iter().map(|f| f.name_taken_by).collect();
        assert_eq!(why, [None, Some("this_run"), Some("this_run")]);
    }

    #[test]
    fn says_when_a_name_is_taken_in_the_destination() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("2005")).unwrap();
        std::fs::write(d.path().join("2005/DSC_0001.jpg"), b"someone else's photo").unwrap();
        let p = build(&[item("A/DSC_0001.jpg", 2005, "a")], d.path(), &PlanOptions::default()).unwrap();
        assert_eq!(dsts(&p), ["2005/DSC_0001_1.jpg"], "the existing file is never overwritten");
        assert_eq!(p.files[0].name_taken_by, Some("destination"));
    }

    #[test]
    fn unicode_forms_clash() {
        let items = vec![item("U/Zo\u{eb}.jpg", 2008, "a"), item("U/Zoe\u{308}.jpg", 2008, "b")];
        let d = tempfile::tempdir().unwrap();
        let p = build(&items, d.path(), &PlanOptions::default()).unwrap();
        let keys: HashSet<String> = p.files.iter().map(|f| name_key(&f.dst_rel)).collect();
        assert_eq!(keys.len(), 2, "visually identical names must not collide");
        assert_eq!(p.files.iter().filter(|f| f.dst_rel.ends_with("_1.jpg")).count(), 1);
    }

    #[test]
    fn duplicates_and_existing_destination() {
        let mut dup = item("Backup/DSC_0001.jpg", 2005, "a");
        dup.duplicate_of = Some("A/DSC_0001.jpg".into());
        let items = vec![item("A/DSC_0001.jpg", 2005, "a"), dup];
        let d = tempfile::tempdir().unwrap();
        let p = build(&items, d.path(), &PlanOptions::default()).unwrap();
        assert_eq!(p.files.len(), 1);
        assert!(matches!(&p.skipped[0], Skipped::Duplicate { same_as, .. } if same_as == "A/DSC_0001.jpg"));
    }

    #[test]
    fn month_trees_ask_then_obey() {
        let items = vec![item("2005/03/a.jpg", 2005, "a"), item("2005/March/b.jpg", 2005, "b")];
        let d = tempfile::tempdir().unwrap();
        let p = build(&items, d.path(), &PlanOptions::default()).unwrap();
        assert!(!p.ready());
        assert_eq!(p.month_question.as_ref().unwrap().folders.len(), 2);
        let keep = PlanOptions { months: MonthPolicy::Keep, ..Default::default() };
        assert_eq!(dsts(&build(&items, d.path(), &keep).unwrap()), ["2005/03/a.jpg", "2005/March/b.jpg"]);
        let flat = PlanOptions { months: MonthPolicy::Flatten, ..Default::default() };
        let p = build(&items, d.path(), &flat).unwrap();
        assert!(p.ready());
        assert_eq!(dsts(&p), ["2005/a.jpg", "2005/b.jpg"]);
    }

    #[test]
    fn groups_share_names_when_renamed() {
        let mut jpg = item("Raw/DSC_0700.JPG", 2013, "a");
        let mut nef = item("Raw/DSC_0700.NEF", 2013, "b");
        nef.format = "tiff".into();
        jpg.group = "raw/dsc_0700".into();
        nef.group = "raw/dsc_0700".into();
        let mut xmp = item("Raw/DSC_0700.xmp", 2013, "c");
        xmp.kind = MediaKind::Sidecar;
        xmp.group = "raw/dsc_0700".into();
        let opts = PlanOptions { naming: Naming::DateTime, ..Default::default() };
        let d = tempfile::tempdir().unwrap();
        let p = build(&[jpg, nef, xmp], d.path(), &opts).unwrap();
        let mut got = dsts(&p);
        got.sort();
        assert_eq!(
            got,
            ["2013/2013-03-12_14-22-07.JPG", "2013/2013-03-12_14-22-07.NEF", "2013/2013-03-12_14-22-07.xmp"]
        );
    }
}
