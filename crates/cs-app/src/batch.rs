//! Batch tools in the file browser: convert / resize many pictures at once
//! (copies in a new folder, originals untouched) and rename many files with a
//! pattern (with a preview, refused when any name would clash; one Undo).
//!
//! Progress: `batch-progress` events `{ done, total, name }`; `batch_cancel` stops
//! after the file being worked on.

use crate::fileops::{self, Op, Undo};
use crate::{Res, err};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static CANCEL: AtomicBool = AtomicBool::new(false);
static BUSY: AtomicBool = AtomicBool::new(false);

/// Progress of a batch: files done, of how many, the file just done.
#[derive(Clone, Serialize, Debug, Default)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub name: String,
}

/// Stop the running batch after the current file.
pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

struct BusyGuard;
impl BusyGuard {
    fn take() -> Res<BusyGuard> {
        if BUSY.swap(true, Ordering::SeqCst) {
            return Err("Another batch is still running.".into());
        }
        CANCEL.store(false, Ordering::SeqCst);
        Ok(BusyGuard)
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

// ===================================================================== convert

/// A copy that was made: its path, size and modified time (what Undo checks).
type Made = (PathBuf, u64, Option<std::time::SystemTime>);
/// Year, month, day, hour, minute, second.
type Civil = (i64, u8, u8, u8, u8, u8);

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Resize {
    None,
    /// Longest side at most `px`.
    Long {
        px: u32,
    },
    /// Percent of the original size.
    Percent {
        pct: u32,
    },
    /// Fit inside `w` × `h`.
    Fit {
        w: u32,
        h: u32,
    },
}

impl Resize {
    /// New size for a `w` × `h` picture, or `None` to keep it. Never enlarges.
    pub fn target(self, w: u32, h: u32) -> Option<(u32, u32)> {
        let s = match self {
            Resize::None => return None,
            Resize::Long { px } => f64::from(px) / f64::from(w.max(h)),
            Resize::Percent { pct } => f64::from(pct) / 100.0,
            Resize::Fit { w: fw, h: fh } => (f64::from(fw) / f64::from(w)).min(f64::from(fh) / f64::from(h)),
        };
        if !(s.is_finite() && s > 0.0 && s < 1.0) {
            return None;
        }
        Some((((f64::from(w) * s).round() as u32).max(1), ((f64::from(h) * s).round() as u32).max(1)))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOpts {
    /// "keep" or a name from `cs_image::save_formats` ("JPEG", "PNG", "TIFF", "WebP").
    pub format: String,
    pub quality: u8,
    pub resize: Resize,
    /// Folder for the copies; `None` = a new "Converted" folder next to the pictures.
    pub folder: Option<String>,
    /// Frame / watermark to add (the user's last settings), with the watermark picture.
    pub effects: Option<cs_adjust::effects::Effects>,
    pub watermark_png: Option<Vec<u8>>,
    /// …or a logo picture file.
    pub watermark_path: Option<String>,
}

#[derive(Serialize, Default)]
pub struct ConvertReport {
    pub done: usize,
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
    pub folder: String,
}

/// Open any picture ChronoSort can read; RAW files give their camera JPEG.
pub(crate) use crate::open_any;

fn convert_one(src: &Path, dir: &Path, opts: &ConvertOpts, layer: Option<&cs_adjust::effects::Layer>) -> Res<Made> {
    let photo = open_any(src)?;
    let formats = cs_image::save_formats();
    let (fmt, ext) = if opts.format == "keep" {
        formats.iter().find(|f| f.0 == photo.format).map_or((cs_image::Format::Jpeg, "jpg"), |f| (f.0, f.2))
    } else {
        formats
            .iter()
            .find(|f| f.1.eq_ignore_ascii_case(&opts.format))
            .map(|f| (f.0, f.2))
            .ok_or_else(|| format!("Unknown format {}", opts.format))?
    };
    let mut img = photo.image;
    let (w, h) = img.dimensions();
    if let Some((nw, nh)) = opts.resize.target(w, h) {
        img = cs_image::apply(img, cs_image::Edit::Resize { w: nw, h: nh })?;
    }
    if let Some(fx) = &opts.effects
        && !fx.is_none(layer.is_some())
    {
        img = cs_adjust::effects::apply(&img, fx, layer)?;
    }
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "picture".into());
    let target = fileops::free_name(dir, &format!("{stem}.{ext}"));
    cs_image::save(&img, fmt, cs_image::SaveOptions { quality: opts.quality }, &target)?;
    // Keep the original's dates, so sorting by date still works.
    if let (Ok(t), Ok(f)) = (cs_platform::read_times(src), std::fs::File::options().write(true).open(&target)) {
        let _ = cs_platform::apply_times(&f, &t);
    }
    let m = std::fs::metadata(&target).map_err(err)?;
    Ok((target, m.len(), m.modified().ok()))
}

/// Convert / resize pictures into copies. Never changes or overwrites an original.
/// Blocking: run it on a worker thread. `progress` is called after each file (from several threads).
pub fn convert(files: Vec<PathBuf>, opts: ConvertOpts, progress: impl Fn(Progress) + Sync) -> Res<ConvertReport> {
    let files: Vec<PathBuf> = files.into_iter().filter(|p| p.is_file()).collect();
    if files.is_empty() {
        return Err("None of these files can be found any more. Press F5 to refresh.".into());
    }
    let guard = BusyGuard::take()?;
    {
        let _guard = guard;
        let layer = match (&opts.effects, &opts.watermark_png, &opts.watermark_path) {
            (Some(fx), Some(png), _) if fx.watermark.on => Some(cs_adjust::effects::Layer::from_png(png)?),
            (Some(fx), None, Some(path)) if fx.watermark.on => {
                let logo = cs_image::open(Path::new(path)).map_err(|e| format!("The logo could not be opened: {e}"))?;
                Some(cs_adjust::effects::Layer::from_image(&logo.image)?)
            }
            _ => None,
        };
        let base = files[0].parent().ok_or("no folder")?.to_path_buf();
        let (dir, created) = match &opts.folder {
            Some(f) => {
                let d = PathBuf::from(f);
                let new = !d.exists();
                std::fs::create_dir_all(&d).map_err(err)?;
                (d, new)
            }
            None => {
                let d = fileops::free_name(&base, "Converted");
                std::fs::create_dir(&d).map_err(err)?;
                (d, true)
            }
        };
        fileops::guard(&dir.join("x"))?;
        let total = files.len();
        let done = AtomicUsize::new(0);
        let results: Vec<(PathBuf, Res<Made>)> = {
            use rayon::prelude::*;
            // A few at a time: big photos need a lot of memory each.
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4))
                .build()
                .map_err(err)?;
            pool.install(|| {
                files
                    .par_iter()
                    .map(|src| {
                        if CANCEL.load(Ordering::SeqCst) {
                            return (src.clone(), Err("cancelled".to_string()));
                        }
                        let r = convert_one(src, &dir, &opts, layer.as_ref());
                        let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                        let name = src.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        progress(Progress { done: n, total, name });
                        (src.clone(), r)
                    })
                    .collect()
            })
        };
        let mut rep = ConvertReport { folder: dir.to_string_lossy().into_owned(), ..Default::default() };
        let mut made = Vec::new();
        for (src, r) in results {
            match r {
                Ok(f) => {
                    rep.done += 1;
                    made.push(f);
                }
                Err(e) if e == "cancelled" => rep.cancelled = true,
                Err(e) => {
                    rep.failed.push((src.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), e))
                }
            }
        }
        if made.is_empty() && created {
            let _ = std::fs::remove_dir(&dir);
        } else if !made.is_empty() {
            fileops::push(Op {
                label: format!(
                    "Convert {} pictures into “{}”",
                    made.len(),
                    dir.file_name().unwrap_or_default().to_string_lossy()
                ),
                undo: Undo::Copied { files: made, dirs: if created { vec![dir.clone()] } else { vec![] } },
            });
        }
        Ok(rep)
    }
}

// ====================================================================== rename

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RenameOpts {
    /// Pattern with {name} {num} {year} {month} {day} {hour} {minute} {second} {folder}.
    pub pattern: String,
    pub start: i64,
    pub step: i64,
    /// Digits of {num} (0 = as many as the last number needs).
    pub digits: u8,
    pub find: String,
    pub replace: String,
    /// "keep" | "lower" | "upper"
    pub ext_case: String,
    /// Also rename sidecar files (.xmp, .aae, .thm, …) that belong to a photo.
    pub sidecars: bool,
    /// The PC's offset from UTC in minutes (for file and video dates).
    pub tz_offset_min: i32,
}

#[derive(Serialize, Clone, Debug)]
pub struct RenameRow {
    pub id: u64,
    pub old: String,
    pub new: String,
    /// Where the date came from: "taken" | "file" | "" (pattern has no date).
    pub date_from: String,
    /// Why this name cannot be used, if so.
    pub problem: Option<String>,
}

const SIDECARS: &[&str] = &["xmp", "aae", "thm", "json"];

/// Year, month, day, hour, minute, second of when the photo was taken (or the file's date).
fn date_of(p: &Path, tz: i32) -> (Option<Civil>, &'static str) {
    let fmt = crate::real_format(p);
    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let entry = cs_scan::FileEntry {
        abs: p.to_path_buf(),
        rel: name.clone(),
        name,
        size,
        format: fmt,
        kind: fmt.kind(),
        ext_mismatch: false,
        is_raw: false,
        group: String::new(),
    };
    let f = cs_meta::read_facts(&entry);
    let full = |d: &cs_model::PartialDate| {
        Some((
            i64::from(d.year),
            d.month?,
            d.day?,
            d.time.map_or(0, |t| t.0),
            d.time.map_or(0, |t| t.1),
            d.time.map_or(0, |t| t.2),
        ))
    };
    let taken = f
        .exif
        .as_ref()
        .and_then(|e| e.original.as_ref().or(e.digitized.as_ref()))
        .and_then(full)
        .or_else(|| f.qt_local.as_ref().and_then(full))
        .or_else(|| f.qt_created_utc.map(|t| cs_model::civil::from_unix(t + i64::from(tz) * 60)));
    if taken.is_some() {
        return (taken, "taken");
    }
    (f.fs_modified.map(|t| cs_model::civil::from_unix(t + i64::from(tz) * 60)), "file")
}

const TOKENS: &[&str] = &["name", "num", "year", "month", "day", "hour", "minute", "second", "folder"];

/// Check a pattern; the message is for the user.
pub fn check_pattern(p: &str) -> Res<()> {
    if p.trim().is_empty() {
        return Err("Type a pattern, for example {name} or Holiday {num}.".into());
    }
    let mut rest = p;
    let mut any = false;
    while let Some(i) = rest.find('{') {
        let j = rest[i..].find('}').ok_or("Every { needs a matching }.")? + i;
        let tok = &rest[i + 1..j];
        if !TOKENS.contains(&tok) {
            return Err(format!(
                "{{{tok}}} is not a known part. Use {{name}}, {{num}}, {{year}}, {{month}}, {{day}}, {{hour}}, {{minute}}, {{second}} or {{folder}}."
            ));
        }
        any |= tok == "name" || tok == "num" || tok == "second";
        rest = &rest[j + 1..];
    }
    if rest.contains('}') {
        return Err("Every } needs a matching {.".into());
    }
    if !any {
        return Err("Include {name}, {num} or the full time down to {second}, so every file gets its own name.".into());
    }
    Ok(())
}

fn uses_date(p: &str) -> bool {
    ["{year}", "{month}", "{day}", "{hour}", "{minute}", "{second}"].iter().any(|t| p.contains(t))
}

/// The new names, in the given order. Pure, so it can be tested.
fn plan(files: &[(u64, PathBuf)], o: &RenameOpts) -> Res<Vec<RenameRow>> {
    check_pattern(&o.pattern)?;
    let need_date = uses_date(&o.pattern);
    let last = o.start + o.step * (files.len() as i64 - 1).max(0);
    let digits = if o.digits > 0 { usize::from(o.digits) } else { last.max(o.start).abs().to_string().len() };
    let mut rows = Vec::with_capacity(files.len());
    for (i, (id, p)) in files.iter().enumerate() {
        let old = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = p.extension().map(|s| s.to_string_lossy().into_owned());
        let folder =
            p.parent().and_then(|d| d.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (date, from) = if need_date { date_of(p, o.tz_offset_min) } else { (None, "") };
        let n = o.start + o.step * i as i64;
        let num = if n < 0 { format!("-{:0digits$}", -n) } else { format!("{n:0digits$}") };
        let mut s = o.pattern.replace("{name}", &stem).replace("{num}", &num).replace("{folder}", &folder);
        let mut problem = None;
        if need_date {
            match date {
                Some((y, mo, d, h, mi, se)) => {
                    s = s
                        .replace("{year}", &format!("{y:04}"))
                        .replace("{month}", &format!("{mo:02}"))
                        .replace("{day}", &format!("{d:02}"))
                        .replace("{hour}", &format!("{h:02}"))
                        .replace("{minute}", &format!("{mi:02}"))
                        .replace("{second}", &format!("{se:02}"));
                }
                None => problem = Some("no date found".to_string()),
            }
        }
        if !o.find.is_empty() {
            s = s.replace(&o.find, &o.replace);
        }
        let s = s.trim().to_string();
        let ext = ext.map(|e| match o.ext_case.as_str() {
            "lower" => e.to_lowercase(),
            "upper" => e.to_uppercase(),
            _ => e,
        });
        let new = match ext {
            Some(e) => format!("{s}.{e}"),
            None => s,
        };
        if problem.is_none()
            && let Err(e) = fileops::check_name(&new)
        {
            problem = Some(e);
        }
        rows.push(RenameRow { id: *id, old, new, date_from: from.into(), problem });
    }
    // Two files may not get the same name, and a new name may not hit a file that stays.
    let lower = |s: &str| s.to_lowercase();
    let moving: std::collections::HashSet<(PathBuf, String)> = files
        .iter()
        .map(|(_, p)| {
            (
                p.parent().unwrap_or(Path::new("")).to_path_buf(),
                lower(&p.file_name().unwrap_or_default().to_string_lossy()),
            )
        })
        .collect();
    let mut seen: std::collections::HashMap<(PathBuf, String), usize> = std::collections::HashMap::new();
    for (i, (_, p)) in files.iter().enumerate() {
        let dir = p.parent().unwrap_or(Path::new("")).to_path_buf();
        let key = (dir.clone(), lower(&rows[i].new));
        if let Some(&j) = seen.get(&key) {
            let other = rows[j].old.clone();
            rows[i].problem.get_or_insert_with(|| format!("same new name as {other}"));
        } else {
            seen.insert(key.clone(), i);
        }
        if rows[i].problem.is_none()
            && lower(&rows[i].new) != lower(&rows[i].old)
            && dir.join(&rows[i].new).exists()
            && !moving.contains(&key)
        {
            rows[i].problem = Some("a file with this name is already there".into());
        }
    }
    Ok(rows)
}

/// The files with their position as id (the rows of a preview refer to them by it).
fn sorted_files(files: &[PathBuf]) -> Res<Vec<(u64, PathBuf)>> {
    let v: Vec<(u64, PathBuf)> =
        files.iter().enumerate().map(|(i, p)| (i as u64, p.clone())).filter(|(_, p)| p.is_file()).collect();
    if v.is_empty() {
        return Err("None of these files can be found any more. Press F5 to refresh.".into());
    }
    Ok(v)
}

/// What the names would become (first `limit` rows; problems are always included).
pub fn rename_preview(files: &[PathBuf], opts: &RenameOpts) -> Res<Vec<RenameRow>> {
    plan(&sorted_files(files)?, opts)
}

#[derive(Serialize, Default, Debug)]
pub struct RenameReport {
    pub renamed: usize,
    pub unchanged: usize,
    pub sidecars: usize,
}

/// Sidecars of `p` in the same folder: `stem.xmp`, `name.ext.xmp` etc. → their new names.
fn sidecars_of(p: &Path, new_name: &str) -> Vec<(PathBuf, PathBuf)> {
    let (Some(dir), Some(stem), Some(name)) = (p.parent(), p.file_stem(), p.file_name()) else { return vec![] };
    let (stem, name) = (stem.to_string_lossy().to_string(), name.to_string_lossy().to_string());
    let new_stem = Path::new(new_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let f = e.file_name().to_string_lossy().into_owned();
        let Some((base, ext)) = f.rsplit_once('.') else { continue };
        if !SIDECARS.contains(&ext.to_lowercase().as_str()) || !e.path().is_file() {
            continue;
        }
        let new = if base.eq_ignore_ascii_case(&stem) {
            format!("{new_stem}.{ext}")
        } else if base.eq_ignore_ascii_case(&name) {
            format!("{new_name}.{ext}")
        } else {
            continue;
        };
        out.push((e.path(), dir.join(new)));
    }
    out
}

/// Rename `files` as planned; returns the report and the (from, to) steps for Undo.
fn do_rename(
    files: &[(u64, PathBuf)],
    opts: &RenameOpts,
    mut emit: impl FnMut(Progress),
) -> Res<(RenameReport, Vec<(PathBuf, PathBuf)>)> {
    let rows = plan(files, opts)?;
    if let Some(r) = rows.iter().find(|r| r.problem.is_some()) {
        return Err(format!("Nothing was renamed: “{}” → {}", r.old, r.problem.as_deref().unwrap_or("")));
    }
    for (_, p) in files {
        fileops::guard(p)?;
    }
    // Pairs (from, to): the sidecars, then the files.
    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut unchanged = 0;
    let mut side = 0;
    for ((_, p), r) in files.iter().zip(&rows) {
        if r.new == r.old {
            unchanged += 1;
            continue;
        }
        if opts.sidecars {
            for sc in sidecars_of(p, &r.new) {
                if !files.iter().any(|(_, f)| f == &sc.0) {
                    side += 1;
                    pairs.push(sc);
                }
            }
        }
        pairs.push((p.clone(), p.with_file_name(&r.new)));
    }
    // Step 1: everything to a temporary name (so swaps like 1→2, 2→1 work); step 2: the new names.
    // Any failure puts back what was already done.
    let run = crate::now_unix();
    let total = pairs.len();
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    let fail = |done: &mut Vec<(PathBuf, PathBuf)>, msg: String| -> String {
        for (a, b) in done.drain(..).rev() {
            let _ = cs_platform::rename_no_replace(&b, &a);
        }
        format!("{msg} Nothing was renamed.")
    };
    let mut temps = Vec::with_capacity(total);
    for (i, (from, _)) in pairs.iter().enumerate() {
        let tmp = from.with_file_name(format!(".cs-rename-{run}-{i}.tmp"));
        if let Err(e) = cs_platform::rename_no_replace(from, &tmp) {
            return Err(fail(&mut done, format!("“{}” could not be renamed: {e}.", from.display())));
        }
        done.push((from.clone(), tmp.clone()));
        temps.push(tmp);
    }
    for (i, ((_, to), tmp)) in pairs.iter().zip(&temps).enumerate() {
        if let Err(e) = cs_platform::rename_no_replace(tmp, to) {
            return Err(fail(&mut done, format!("“{}” could not be given its new name: {e}.", to.display())));
        }
        done.push((tmp.clone(), to.clone()));
        if i % 20 == 0 || i + 1 == total {
            emit(Progress { done: i + 1, total, name: to.file_name().unwrap_or_default().to_string_lossy().into() });
        }
    }
    Ok((RenameReport { renamed: files.len() - unchanged, unchanged, sidecars: side }, done))
}

/// Rename the files as previewed. Refused if any row has a problem. One Undo puts all names back.
/// Blocking.
pub fn rename(files: &[PathBuf], opts: &RenameOpts, progress: impl Fn(Progress)) -> Res<RenameReport> {
    let files = sorted_files(files)?;
    let guard = BusyGuard::take()?;
    {
        let _guard = guard;
        let (rep, done) = do_rename(&files, opts, progress)?;
        if !done.is_empty() {
            let names = rep.renamed + rep.sidecars;
            fileops::push(Op {
                label: format!("Rename {} files", rep.renamed),
                undo: Undo::RenamedMany { steps: done, names },
            });
        }
        Ok(rep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(p: &str) -> RenameOpts {
        RenameOpts {
            pattern: p.into(),
            start: 1,
            step: 1,
            digits: 0,
            find: String::new(),
            replace: String::new(),
            ext_case: "keep".into(),
            sidecars: true,
            tz_offset_min: 0,
        }
    }

    #[test]
    fn resize_never_enlarges() {
        assert_eq!(Resize::Long { px: 1000 }.target(4000, 3000), Some((1000, 750)));
        assert_eq!(Resize::Long { px: 5000 }.target(4000, 3000), None);
        assert_eq!(Resize::Percent { pct: 50 }.target(4000, 3000), Some((2000, 1500)));
        assert_eq!(Resize::Percent { pct: 100 }.target(40, 30), None);
        assert_eq!(Resize::Fit { w: 1920, h: 1080 }.target(3000, 4000), Some((810, 1080)));
        assert_eq!(Resize::None.target(1, 1), None);
        assert_eq!(Resize::Long { px: 0 }.target(10, 10), None);
    }

    #[test]
    fn patterns_are_checked() {
        assert!(check_pattern("{name}").is_ok());
        assert!(check_pattern("Holiday {num}").is_ok());
        assert!(check_pattern("{year}-{month}-{day} {hour}.{minute}.{second}").is_ok());
        assert!(check_pattern("Holiday").is_err());
        assert!(check_pattern("{year}").is_err());
        assert!(check_pattern("{nam}").is_err());
        assert!(check_pattern("{name").is_err());
        assert!(check_pattern("name}").is_err());
        assert!(check_pattern("  ").is_err());
    }

    #[test]
    fn plan_numbers_cases_and_clashes() {
        let d = tempfile::tempdir().unwrap();
        let mk = |n: &str| {
            let p = d.path().join(n);
            std::fs::write(&p, b"x").unwrap();
            p
        };
        let files: Vec<(u64, PathBuf)> =
            ["b.JPG", "a.jpg", "c.png"].iter().enumerate().map(|(i, n)| (i as u64, mk(n))).collect();
        mk("Trip 2.JPG"); // a file that is not renamed
        let mut o = opts("Trip {num}");
        o.ext_case = "lower".into();
        let r = plan(&files, &o).unwrap();
        assert_eq!(r.iter().map(|x| x.new.as_str()).collect::<Vec<_>>(), ["Trip 1.jpg", "Trip 2.jpg", "Trip 3.png"]);
        assert!(r[0].problem.is_none());
        assert_eq!(r[1].problem.as_deref(), Some("a file with this name is already there"));
        o.digits = 3;
        o.start = 9;
        let r = plan(&files, &o).unwrap();
        assert_eq!(r[2].new, "Trip 011.png");
        // Same names inside the batch.
        let r = plan(&files, &opts("{folder}{num}")).unwrap();
        assert!(r.iter().all(|x| x.problem.is_none()), "{r:?}");
        let mut o = opts("{name}");
        o.find = "b".into();
        o.replace = "a".into();
        let r = plan(&files, &o).unwrap();
        assert_eq!(r[0].new, "a.JPG");
        assert!(r[0].problem.is_none());
        assert_eq!(r[1].problem.as_deref(), Some("same new name as b.JPG"));
        // Bad characters.
        let mut o = opts("{name}");
        o.find = "a".into();
        o.replace = "a:b".into();
        assert!(plan(&files, &o).unwrap()[1].problem.is_some());
    }

    #[test]
    fn date_pattern_falls_back_to_the_file_date() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.txt");
        std::fs::write(&p, b"x").unwrap();
        let r = plan(&[(1, p)], &opts("{year}-{month}-{day}_{hour}{minute}{second}")).unwrap();
        assert_eq!(r[0].date_from, "file");
        assert!(r[0].problem.is_none());
        assert_eq!(r[0].new.len(), "2026-11-27_120000.txt".len());
    }

    #[test]
    fn swap_rename_with_sidecar_and_undo() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let w = |n: &str, c: &str| std::fs::write(d.path().join(n), c).unwrap();
        w("1.jpg", "one");
        w("2.jpg", "two");
        w("2.xmp", "two-side");
        let files = vec![(1, d.path().join("2.jpg")), (2, d.path().join("1.jpg"))];
        let (rep, done) = do_rename(&files, &opts("{num}"), |_| {}).unwrap();
        assert_eq!((rep.renamed, rep.unchanged, rep.sidecars), (2, 0, 1));
        let r = |n: &str| std::fs::read_to_string(d.path().join(n)).unwrap();
        assert_eq!((r("1.jpg").as_str(), r("2.jpg").as_str(), r("1.xmp").as_str()), ("two", "one", "two-side"));
        assert!(!d.path().join("2.xmp").exists());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 3, "no temporary files left");
        assert_eq!(
            crate::fileops::undo_op(Undo::RenamedMany { steps: done, names: 3 }).unwrap(),
            "3 old names put back."
        );
        assert_eq!((r("1.jpg").as_str(), r("2.jpg").as_str(), r("2.xmp").as_str()), ("one", "two", "two-side"));
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 3);
        // A clash refuses the whole batch and changes nothing.
        w("Trip 1.jpg", "x");
        let files = vec![(1, d.path().join("1.jpg")), (2, d.path().join("2.jpg"))];
        assert!(do_rename(&files, &opts("Trip {num}"), |_| {}).unwrap_err().starts_with("Nothing was renamed"));
        assert_eq!(r("1.jpg"), "one");
    }

    #[test]
    fn sidecars_follow_their_photo() {
        let d = tempfile::tempdir().unwrap();
        for n in ["IMG_1.CR2", "IMG_1.xmp", "IMG_1.CR2.xmp", "IMG_10.xmp", "IMG_1.txt"] {
            std::fs::write(d.path().join(n), b"x").unwrap();
        }
        let mut s: Vec<String> = sidecars_of(&d.path().join("IMG_1.CR2"), "Trip 1.CR2")
            .into_iter()
            .map(|(a, b)| {
                format!("{} -> {}", a.file_name().unwrap().to_string_lossy(), b.file_name().unwrap().to_string_lossy())
            })
            .collect();
        s.sort();
        assert_eq!(s, ["IMG_1.CR2.xmp -> Trip 1.CR2.xmp", "IMG_1.xmp -> Trip 1.xmp"]);
    }
}
