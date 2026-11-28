//! ChronoSort desktop app (Tauri 2). Thin layer over the same crates as the CLI.
//!
//! Security model: the web UI has no filesystem access. It can only call the
//! typed commands below, open the native folder picker, and load thumbnails
//! through the `thumb://` scheme, which serves cached previews by content hash (and `preview://`, the same at 1280 px)
//! of files that were part of the current analysis, and nothing else.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cs_model::{Decision, ItemReport, MediaKind, TransferMode};
use cs_plan::{MonthPolicy, Naming, Plan, PlanOptions, Skipped};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};

mod batch;
mod browse;
mod create;
mod desktop;
mod edit;
mod fileops;
mod histogram;
mod meta;
mod people;
mod places;
mod portable;
mod privacy;
mod rating;
mod review;
mod slideshow;
mod thumbcache;
mod vision;

#[derive(Default)]
struct Session {
    source: Option<PathBuf>,
    items: Vec<ItemReport>,
    /// hash -> (absolute path, format) for thumbnails
    by_hash: HashMap<String, (PathBuf, cs_scan::Format)>,
    plan: Option<(PathBuf, Plan)>,
    dups: Option<cs_core::dup::DupScan>,
}

struct AppState {
    session: Mutex<Session>,
    thumbs: cs_thumb::Cache,
    /// Pack id being downloaded (one at a time) and its cancel flag.
    download: Mutex<Option<String>>,
    cancel: std::sync::atomic::AtomicBool,
    /// The last face search (People screen).
    faces: people::Faces,
    /// The local vision model, started on first use.
    vision: vision::Vision,
    /// Files of recent File-browser listings (ids for `fthumb://`).
    browse: browse::Registry,
}

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Where ChronoSort keeps its own data (models, engine, people, logs,
/// remembered checks): `ChronoSortData` next to the program in portable mode
/// (a `portable.txt` beside ChronoSort.exe), else the user's app data folder.
pub fn data_dir(app: &tauri::AppHandle) -> Res<PathBuf> {
    match cs_ai::portable_root() {
        Some(p) => Ok(p),
        None => app.path().app_data_dir().map_err(err),
    }
}

/// Thumbnails cache (can be deleted at any time). It lives next to the program
/// (`ChronoSortData\cache`), so it travels with ChronoSort; only when that folder
/// cannot be written (e.g. under Program Files) the PC's own cache folder is used.
fn cache_dir(app: &tauri::AppHandle) -> Res<PathBuf> {
    if let Some(p) = cs_ai::portable_root() {
        return Ok(p.join("cache"));
    }
    if let Some(d) = thumbcache::beside_program() {
        return Ok(d);
    }
    app.path().app_cache_dir().map_err(err)
}

pub fn now_unix() -> i64 {
    cs_model::civil::system_to_unix(std::time::SystemTime::now())
}

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
    bytes: u64,
}

#[derive(Serialize)]
struct AnalyzeOut {
    total: usize,
    auto_by_year: BTreeMap<i16, usize>,
    /// Items that need the user (full detail, for the review grid).
    review: Vec<ItemReport>,
    junk: usize,
    duplicates: usize,
    errors: Vec<String>,
}

#[tauri::command]
async fn analyze(app: tauri::AppHandle, state: State<'_, AppState>, source: String) -> Res<AnalyzeOut> {
    let src = PathBuf::from(&source);
    let a = {
        let app = app.clone();
        let src = src.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let emit = |done, total| {
                let _ = app.emit("analyze-progress", Progress { done, total, bytes: 0 });
            };
            cs_core::analyze_with_progress(&src, &cs_core::DatePolicy::new(now_unix()), &emit)
        })
        .await
        .map_err(err)?
        .map_err(err)?
    };
    let mut out = AnalyzeOut {
        total: a.items.len(),
        auto_by_year: BTreeMap::new(),
        review: vec![],
        junk: 0,
        duplicates: 0,
        errors: a.errors.iter().map(|(p, e)| format!("{}: {e}", p.display())).collect(),
    };
    let mut by_hash = HashMap::new();
    for i in &a.items {
        if let Some(h) = &i.hash {
            let abs = cs_exec::join_rel(&src, &i.rel_path).map_err(err)?;
            by_hash.insert(h.clone(), (abs, cs_scan::Format::from_name(&i.format)));
        }
        out.duplicates += usize::from(i.duplicate_of.is_some());
        match &i.decision {
            Decision::Auto { year, .. } => *out.auto_by_year.entry(*year).or_default() += 1,
            Decision::Review { .. } if i.kind != MediaKind::Sidecar => out.review.push(i.clone()),
            Decision::Review { .. } => {}
            Decision::Skip { .. } => out.junk += 1,
        }
    }
    let mut s = state.session.lock().map_err(err)?;
    s.source = Some(src);
    s.items = a.items;
    s.by_hash.extend(by_hash);
    s.plan = None;
    Ok(out)
}

#[derive(Serialize)]
struct PlanOut {
    ready: bool,
    month_folders: Vec<(String, usize)>,
    files: usize,
    total_bytes: u64,
    per_year: BTreeMap<i16, usize>,
    duplicates: usize,
    already_in_destination: usize,
    unresolved: usize,
    junk: usize,
    saved_bytes: u64,
    renamed: usize,
    /// Planned copies for the preview (renamed ones first, then the rest; at most 500).
    sample: Vec<PreviewRow>,
    /// Files skipped because the same photo is already in the destination (at most 300).
    already_there: Vec<AlreadyRow>,
    notes: Vec<String>,
}

#[derive(Serialize)]
struct PreviewRow {
    src: String,
    dst: String,
    /// For the thumbnail (thumb://hash).
    hash: Option<String>,
    /// The wanted name was taken: "destination" or "this_run".
    taken_by: Option<&'static str>,
}

#[derive(Serialize)]
struct AlreadyRow {
    src: String,
    existing: String,
    hash: Option<String>,
}

#[tauri::command]
fn make_plan(
    state: State<'_, AppState>,
    dest: String,
    months: String,
    naming: String,
    resolutions: HashMap<String, i16>,
) -> Res<PlanOut> {
    let mut s = state.session.lock().map_err(err)?;
    let opts = PlanOptions {
        months: match months.as_str() {
            "keep" => MonthPolicy::Keep,
            "flatten" => MonthPolicy::Flatten,
            _ => MonthPolicy::Ask,
        },
        naming: Naming::parse(&naming).map_err(err)?,
        accept_suggested: false,
        resolutions,
    };
    let dest = PathBuf::from(dest);
    let plan = cs_plan::build(&s.items, &dest, &opts).map_err(err)?;
    let count = |f: fn(&Skipped) -> bool| plan.skipped.iter().filter(|x| f(x)).count();
    let mut per_year = BTreeMap::new();
    for f in &plan.files {
        *per_year.entry(f.year).or_default() += 1;
    }
    let out = PlanOut {
        ready: plan.ready(),
        month_folders: plan.month_question.as_ref().map(|q| q.folders.clone()).unwrap_or_default(),
        files: plan.files.len(),
        total_bytes: plan.total_bytes,
        per_year,
        duplicates: count(|x| matches!(x, Skipped::Duplicate { .. })),
        already_in_destination: count(|x| matches!(x, Skipped::AlreadyInDestination { .. })),
        unresolved: count(|x| matches!(x, Skipped::Unresolved { .. })),
        junk: count(|x| matches!(x, Skipped::Junk { .. })),
        saved_bytes: plan.bytes_saved_by_dedup(&s.items),
        renamed: plan.files.iter().filter(|f| f.dst_rel.rsplit('/').next() != f.src_rel.rsplit('/').next()).count(),
        sample: plan
            .files
            .iter()
            .filter(|f| f.name_taken_by.is_some())
            .chain(plan.files.iter().filter(|f| f.name_taken_by.is_none()))
            .take(500)
            .map(|f| PreviewRow {
                src: f.src_rel.clone(),
                dst: f.dst_rel.clone(),
                hash: f.hash.clone(),
                taken_by: f.name_taken_by,
            })
            .collect(),
        already_there: {
            let hash_of: HashMap<&str, &Option<String>> =
                s.items.iter().map(|i| (i.rel_path.as_str(), &i.hash)).collect();
            plan.skipped
                .iter()
                .filter_map(|x| match x {
                    Skipped::AlreadyInDestination { src_rel, existing } => Some(AlreadyRow {
                        src: src_rel.clone(),
                        existing: existing.clone(),
                        hash: hash_of.get(src_rel.as_str()).and_then(|h| (*h).clone()),
                    }),
                    _ => None,
                })
                .take(300)
                .collect()
        },
        notes: plan.notes.clone(),
    };
    s.plan = Some((dest, plan));
    Ok(out)
}

#[tauri::command]
async fn run(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: String,
    threads: usize,
) -> Res<cs_exec::ExecReport> {
    let (src, dest, plan) = {
        let s = state.session.lock().map_err(err)?;
        let (dest, plan) = s.plan.clone().ok_or("make a plan first")?;
        (s.source.clone().ok_or("analyse a folder first")?, dest, plan)
    };
    let mode = if mode == "move" { TransferMode::Move } else { TransferMode::Copy };
    let app2 = app.clone();
    let progress: cs_exec::ProgressFn = Arc::new(move |done, total, bytes| {
        let _ = app2.emit("run-progress", Progress { done, total, bytes });
    });
    let opts =
        cs_exec::ExecOptions { mode, threads: threads.clamp(1, 32), progress: Some(progress), ..Default::default() };
    tauri::async_runtime::spawn_blocking(move || cs_exec::execute(&src, &dest, &plan, &opts))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
async fn undo(dest: String) -> Res<cs_exec::UndoReport> {
    tauri::async_runtime::spawn_blocking(move || cs_exec::undo(&PathBuf::from(dest), None))
        .await
        .map_err(err)?
        .map_err(err)
}

// ---------------------------------------------------------------- naming

#[derive(Serialize)]
struct NamingCheck {
    ok: bool,
    error: Option<String>,
    /// (current name, new name) examples from the analysed files.
    examples: Vec<(String, String)>,
}

#[tauri::command]
fn check_naming(state: State<'_, AppState>, naming: String) -> Res<NamingCheck> {
    let s = state.session.lock().map_err(err)?;
    Ok(match Naming::parse(&naming) {
        Ok(n) => NamingCheck { ok: true, error: None, examples: cs_plan::naming_preview(&s.items, &n, 6) },
        Err(e) => NamingCheck { ok: false, error: Some(e.to_string()), examples: vec![] },
    })
}

// ---------------------------------------------------------------- duplicates

#[tauri::command]
async fn find_duplicates(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    roots: Vec<String>,
    include_all: bool,
    similar: Option<bool>,
) -> Res<serde_json::Value> {
    let similar = similar.unwrap_or(false);
    let roots: Vec<PathBuf> = roots.into_iter().filter(|r| !r.trim().is_empty()).map(PathBuf::from).collect();
    if roots.is_empty() {
        return Err("choose at least one folder".into());
    }
    let app2 = app.clone();
    let scan = tauri::async_runtime::spawn_blocking(move || {
        let emit = |done, total| {
            let _ = app2.emit("dup-progress", Progress { done, total, bytes: 0 });
        };
        // Near-duplicates: look at each photo through the thumbnail cache (upright,
        // HEIC / RAW previews included; cached for next time).
        let st = app2.state::<AppState>();
        let look = |f: &cs_core::dup::DupFile, hash: &str| -> Option<cs_core::dup::Look> {
            let fmt = cs_scan::Format::from_name(&f.format);
            let thumb = st.thumbs.get_sized(hash, &f.abs, fmt, cs_thumb::DEFAULT_SIZE).ok()?;
            let g = image::open(thumb).ok()?.to_luma8();
            let (w, h) = g.dimensions();
            Some(cs_core::dup::Look {
                gray: cs_core::dup::Gray { w, h, px: g.into_raw() },
                pixels: browse::image_size_of(&f.abs),
            })
        };
        cs_core::dup::find_duplicates_ext(&roots, include_all, similar.then_some(&look as cs_core::dup::LookFn), &emit)
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    let mut s = state.session.lock().map_err(err)?;
    for g in &scan.groups {
        if let Some(f) = g.files.first() {
            s.by_hash.insert(g.hash.clone(), (f.abs.clone(), cs_scan::Format::from_name(&f.format)));
        }
        // Near-duplicates are different files: each copy gets its own preview.
        for f in g.files.iter().filter(|_| g.similar) {
            if let Some(h) = &f.hash {
                s.by_hash.insert(h.clone(), (f.abs.clone(), cs_scan::Format::from_name(&f.format)));
            }
        }
    }
    let out = serde_json::to_value(&scan).map_err(err)?;
    s.dups = Some(scan);
    Ok(out)
}

/// `keep`: group hash -> index of the copy to keep. Only groups listed are acted on.
#[tauri::command]
async fn resolve_duplicates(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    keep: HashMap<String, usize>,
    mode: String,
    target: Option<String>,
) -> Res<cs_exec::DupReport> {
    let (actions, roots) = {
        let s = state.session.lock().map_err(err)?;
        let scan = s.dups.as_ref().ok_or("find duplicates first")?;
        let mut actions = Vec::new();
        for g in scan.groups.iter().filter(|g| keep.contains_key(&g.hash)) {
            let k = keep[&g.hash];
            let kept = g.files.get(k).ok_or("invalid choice")?;
            // Identical sets share one hash; near-duplicates each have their own.
            let hash_of = |f: &cs_core::dup::DupFile| -> Res<String> {
                if g.similar {
                    f.hash.clone().ok_or_else(|| "a file was not read completely".to_string())
                } else {
                    Ok(g.hash.clone())
                }
            };
            let keep_hash = hash_of(kept)?;
            let extras = g
                .files
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != k)
                .map(|(_, f)| {
                    // Keep the folder structure under a folder named after the checked folder.
                    let root_name = scan.roots[f.root].file_name().map(|n| n.to_string_lossy().into_owned());
                    Ok((
                        f.abs.clone(),
                        format!("{}/{}", root_name.unwrap_or_else(|| "folder".into()), f.rel),
                        hash_of(f)?,
                    ))
                })
                .collect::<Res<Vec<_>>>()?;
            actions.push(cs_exec::DupAction { keep: kept.abs.clone(), hash: keep_hash, extras });
        }
        (actions, scan.roots.clone())
    };
    let journal = data_dir(&app)?.join("journal");
    let mode = match (mode.as_str(), target) {
        ("move", Some(t)) if !t.trim().is_empty() => cs_exec::DupMode::MoveTo(PathBuf::from(t)),
        ("trash", _) => cs_exec::DupMode::Trash,
        _ => return Err("choose where to put the extra copies".into()),
    };
    // "Recycle Bin" must never turn into a permanent delete: on Windows a drive
    // whose bin is switched off (or a USB stick) deletes for good.
    if matches!(mode, cs_exec::DupMode::Trash) {
        for a in &actions {
            for (p, _, _) in &a.extras {
                let b = cs_platform::recycle_bin(p);
                if !b.available {
                    return Err(format!(
                        "{} So the extra copies cannot go to the Recycle Bin. Choose “Move them to a folder” instead.",
                        b.reason.unwrap_or_default()
                    ));
                }
            }
        }
    }
    let journal_dir = match &mode {
        cs_exec::DupMode::MoveTo(d) => d.clone(),
        cs_exec::DupMode::Trash => journal,
    };
    tauri::async_runtime::spawn_blocking(move || {
        cs_exec::resolve_duplicates(&actions, &mode, &roots, &journal_dir, cs_exec::os_trash)
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
fn save_duplicate_report(state: State<'_, AppState>, path: String) -> Res<()> {
    let s = state.session.lock().map_err(err)?;
    let scan = s.dups.as_ref().ok_or("find duplicates first")?;
    cs_core::dup::write_csv(scan, &PathBuf::from(path)).map_err(err)
}

#[tauri::command]
async fn undo_duplicates(target: String) -> Res<cs_exec::UndoReport> {
    tauri::async_runtime::spawn_blocking(move || cs_exec::undo(&PathBuf::from(target), None))
        .await
        .map_err(err)?
        .map_err(err)
}

// ---------------------------------------------------------------- AI models
// Hardware check + the built-in catalogue. The only network use in ChronoSort is
// downloading a model the user asked for (cs_ai::download): photos never leave the PC.

#[derive(Serialize)]
struct PackView {
    #[serde(flatten)]
    pack: cs_ai::Pack,
    approx_bytes: u64,
    fit: cs_ai::Fit,
    status: cs_ai::PackStatus,
    downloadable: bool,
    /// role -> linked path
    linked: BTreeMap<String, String>,
    /// role -> the linked file is the exact checked one
    verified: BTreeMap<String, bool>,
    /// Some linked files live in ChronoSort's own models folder (can be deleted).
    owns_files: bool,
}

#[derive(Serialize)]
struct AiOverview {
    ram_bytes: Option<u64>,
    gpus: Vec<cs_platform::Gpu>,
    assessment: cs_ai::Assessment,
    /// purpose ("faces" / "vision") -> pack id
    recommended: BTreeMap<String, String>,
    packs: Vec<PackView>,
    models_dir: String,
    free_bytes: Option<u64>,
    downloading: Option<String>,
}

fn models_json(app: &tauri::AppHandle) -> Res<PathBuf> {
    Ok(data_dir(app)?.join("models.json"))
}

/// Where downloaded models go: <app data>/models/<pack id>/<file>.
fn models_dir(app: &tauri::AppHandle) -> Res<PathBuf> {
    Ok(data_dir(app)?.join("models"))
}

/// Where downloaded programs (llama.cpp) are unpacked: <app data>/tools/<pack id>/app/.
fn tools_dir(app: &tauri::AppHandle) -> Res<PathBuf> {
    Ok(data_dir(app)?.join("tools"))
}

/// Folder a pack's own files live in (downloads), models or programs.
fn pack_home(app: &tauri::AppHandle, p: &cs_ai::Pack) -> Res<PathBuf> {
    Ok(if p.unpack { tools_dir(app)? } else { models_dir(app)? }.join(&p.id))
}

/// First file called `name` below `dir` (archives may or may not have a top folder).
fn find_file(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            let ft = e.file_type().ok()?;
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
                return Some(p);
            }
        }
    }
    None
}

#[derive(Serialize, Clone)]
struct ModelProgress {
    pack: String,
    file: String,
    /// "download" or "check"
    phase: &'static str,
    index: usize,
    files: usize,
    done: u64,
    total: u64,
}

/// Point the previewer at the FFmpeg helper if it is set up (else None).
fn refresh_ffmpeg(app: &tauri::AppHandle) {
    let path = models_json(app).ok().and_then(|f| cs_ai::Linked::load(&f).ok()).and_then(|l| {
        let cat = cs_ai::catalogue();
        let p = cat.packs.iter().find(|p| p.id == "ffmpeg")?;
        (l.status(p) == cs_ai::PackStatus::Ready)
            .then(|| l.packs.get("ffmpeg")?.get("archive").map(|f| f.path.clone()))?
    });
    cs_thumb::set_ffmpeg(path);
}

fn ai_view(app: &tauri::AppHandle) -> Res<AiOverview> {
    refresh_ffmpeg(app);
    let hw = cs_ai::Hardware::detect();
    let assessment = cs_ai::assess(&hw);
    let cat = cs_ai::catalogue();
    let linked = cs_ai::Linked::load(&models_json(app)?).map_err(err)?;
    let ready =
        |id: &str| cat.packs.iter().find(|p| p.id == id).is_some_and(|p| linked.status(p) == cs_ai::PackStatus::Ready);
    let recommended = cs_ai::recommend(&cat, assessment.tier, &ready);
    let mdir = models_dir(app)?;
    let tdir = tools_dir(app)?;
    let packs = cat
        .packs
        .iter()
        .map(|p| PackView {
            approx_bytes: p.approx_bytes(),
            fit: cs_ai::fit(p, &hw),
            status: linked.status(p),
            downloadable: p.downloadable(),
            linked: linked
                .packs
                .get(&p.id)
                .map(|m| m.iter().map(|(r, f)| (r.clone(), f.path.display().to_string())).collect())
                .unwrap_or_default(),
            verified: linked
                .packs
                .get(&p.id)
                .map(|m| m.iter().map(|(r, f)| (r.clone(), f.verified)).collect())
                .unwrap_or_default(),
            owns_files: linked
                .packs
                .get(&p.id)
                .is_some_and(|m| m.values().any(|f| f.path.starts_with(&mdir) || f.path.starts_with(&tdir))),
            pack: p.clone(),
        })
        .collect();
    let _ = std::fs::create_dir_all(&mdir);
    let downloading = app.state::<AppState>().download.lock().ok().and_then(|d| d.clone());
    Ok(AiOverview {
        ram_bytes: hw.ram_bytes,
        gpus: cs_platform::gpus(),
        assessment,
        recommended,
        packs,
        free_bytes: cs_platform::free_space(&mdir).ok(),
        models_dir: mdir.display().to_string(),
        downloading,
    })
}

#[tauri::command]
async fn ai_overview(app: tauri::AppHandle) -> Res<AiOverview> {
    tauri::async_runtime::spawn_blocking(move || ai_view(&app)).await.map_err(err)?
}

/// Remember a model file the user already has. The file is only read (header, size).
#[tauri::command]
async fn link_model_file(app: tauri::AppHandle, pack: String, role: String, path: String) -> Res<AiOverview> {
    tauri::async_runtime::spawn_blocking(move || {
        let cat = cs_ai::catalogue();
        let p = cat.packs.iter().find(|p| p.id == pack).ok_or("unknown model")?;
        let file = models_json(&app)?;
        let mut linked = cs_ai::Linked::load(&file).map_err(err)?;
        let total_files = p.files.len();
        let index = p.files.iter().position(|f| f.role == role).unwrap_or(0);
        let name = p.files.get(index).map(|f| f.name.clone()).unwrap_or_default();
        let emit = |done, total| {
            let _ = app.emit(
                "model-progress",
                ModelProgress {
                    pack: pack.clone(),
                    file: name.clone(),
                    phase: "check",
                    index,
                    files: total_files,
                    done,
                    total,
                },
            );
        };
        linked.link(p, &role, &PathBuf::from(path), &emit).map_err(err)?;
        linked.save(&file).map_err(err)?;
        ai_view(&app)
    })
    .await
    .map_err(err)?
}

/// Download every missing file of a pack (verified by size + SHA-256), then link it.
/// Only one download runs at a time; `cancel_download` stops it (resumable).
#[tauri::command]
async fn download_pack(app: tauri::AppHandle, pack: String) -> Res<AiOverview> {
    use std::sync::atomic::Ordering;
    {
        let st = app.state::<AppState>();
        let mut cur = st.download.lock().map_err(err)?;
        if let Some(other) = cur.as_ref() {
            return Err(format!("Another model is downloading ({other}). Please wait or cancel it."));
        }
        *cur = Some(pack.clone());
        st.cancel.store(false, Ordering::Relaxed);
    }
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Res<()> {
        let app = app2;
        let cat = cs_ai::catalogue();
        let p = cat.packs.iter().find(|p| p.id == pack).ok_or("unknown model")?;
        if !p.downloadable() {
            return Err("This model cannot be downloaded (its source has not been checked yet).".into());
        }
        let file = models_json(&app)?;
        let mut linked = cs_ai::Linked::load(&file).map_err(err)?;
        let dir = pack_home(&app, p)?;
        std::fs::create_dir_all(&dir).map_err(err)?;

        // Free space: what is still missing, plus 1 GB to spare.
        let todo: Vec<_> = p
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| !linked.packs.get(&p.id).and_then(|m| m.get(&f.role)).is_some_and(|l| l.verified))
            .collect();
        let need: u64 = todo
            .iter()
            .map(|(_, f)| {
                let part = std::fs::metadata(cs_ai::download::part_path(&dir.join(&f.name))).map_or(0, |m| m.len());
                f.bytes.unwrap_or(0).saturating_sub(part)
            })
            .sum();
        if let Ok(free) = cs_platform::free_space(&dir)
            && free < need + (1 << 30)
        {
            return Err(format!(
                "Not enough free disk space: {:.1} GB needed, {:.1} GB free.",
                (need + (1 << 30)) as f64 / 1e9,
                free as f64 / 1e9
            ));
        }

        let st = app.state::<AppState>();
        for (i, f) in todo {
            let dest = dir.join(&f.name);
            let emit = |done, total| {
                let _ = app.emit(
                    "model-progress",
                    ModelProgress {
                        pack: p.id.clone(),
                        file: f.name.clone(),
                        phase: if done < total || total == 0 { "download" } else { "check" },
                        index: i,
                        files: p.files.len(),
                        done,
                        total,
                    },
                );
            };
            cs_ai::download::fetch(
                f.url.as_deref().ok_or("no url")?,
                &dest,
                f.bytes.ok_or("no size")?,
                f.sha256.as_deref().ok_or("no checksum")?,
                &emit,
                &st.cancel,
            )
            .map_err(err)?;
            let target = if p.unpack {
                // Verified program archive: unpack into our own folder, then drop the zip.
                let app_dir = dir.join("app");
                if app_dir.exists() {
                    std::fs::remove_dir_all(&app_dir).map_err(err)?;
                }
                cs_ai::unpack::extract_zip(&dest, &app_dir).map_err(err)?;
                let _ = std::fs::remove_file(&dest);
                let entry = p.entry.as_deref().ok_or("no entry")?;
                find_file(&app_dir, entry).ok_or_else(|| format!("{entry} is missing from the download"))?
            } else {
                dest
            };
            linked.link_verified(&p.id, &f.role, &target).map_err(err)?;
            linked.save(&file).map_err(err)?; // keep finished files even if a later one fails
        }
        Ok(())
    })
    .await
    .map_err(err)
    .and_then(|r| r);
    if let Ok(mut cur) = app.state::<AppState>().download.lock() {
        *cur = None;
    }
    result?;
    ai_view(&app)
}

#[tauri::command]
fn cancel_download(state: State<'_, AppState>) {
    state.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Delete a pack's files from ChronoSort's own models folder (downloads and
/// unfinished parts) and forget it. Files elsewhere (the user's own) are never touched.
#[tauri::command]
async fn delete_model_files(app: tauri::AppHandle, pack: String) -> Res<AiOverview> {
    tauri::async_runtime::spawn_blocking(move || {
        let cat = cs_ai::catalogue();
        let p = cat.packs.iter().find(|p| p.id == pack).ok_or("unknown model")?;
        if app.state::<AppState>().download.lock().map_err(err)?.as_deref() == Some(p.id.as_str()) {
            return Err("Cancel the download first.".into());
        }
        let dir = pack_home(&app, p)?;
        if p.unpack {
            // Programs: the whole folder is ours (pack ids come from the catalogue).
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", dir.display())),
            }
        }
        for f in &p.files {
            // Only names from the catalogue, only inside our own folder.
            for path in [dir.join(&f.name), cs_ai::download::part_path(&dir.join(&f.name))] {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("{}: {e}", path.display())),
                }
            }
        }
        let _ = std::fs::remove_dir(&dir); // only if empty
        let file = models_json(&app)?;
        let mut linked = cs_ai::Linked::load(&file).map_err(err)?;
        let (mdir, tdir) = (models_dir(&app)?, tools_dir(&app)?);
        // Forget only the files we deleted; links to the user's own files stay.
        if let Some(m) = linked.packs.get_mut(&p.id) {
            m.retain(|_, f| !f.path.starts_with(&mdir) && !f.path.starts_with(&tdir));
            if m.is_empty() {
                linked.unlink(&p.id);
            }
            linked.save(&file).map_err(err)?;
        }
        ai_view(&app)
    })
    .await
    .map_err(err)?
}

/// Forget a linked model. The model files themselves are never deleted.
#[tauri::command]
async fn unlink_model(app: tauri::AppHandle, pack: String) -> Res<AiOverview> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = models_json(&app)?;
        let mut linked = cs_ai::Linked::load(&file).map_err(err)?;
        linked.unlink(&pack);
        linked.save(&file).map_err(err)?;
        ai_view(&app)
    })
    .await
    .map_err(err)?
}

// ---------------------------------------------------------------- misc

#[tauri::command]
fn open_folder(path: String) -> Res<()> {
    cs_platform::open_in_file_manager(&PathBuf::from(path)).map_err(err)
}

/// Show one copy of a duplicate group in Explorer, selected. Only files from
/// the current duplicate scan can be shown.
#[tauri::command]
fn show_duplicate(state: State<'_, AppState>, hash: String, index: usize) -> Res<()> {
    let s = state.session.lock().map_err(err)?;
    let scan = s.dups.as_ref().ok_or("find duplicates first")?;
    let g = scan.groups.iter().find(|g| g.hash == hash).ok_or("unknown group")?;
    let f = g.files.get(index).ok_or("unknown copy")?;
    cs_platform::show_file_in_file_manager(&f.abs).map_err(err)
}

/// One version of a duplicate set, for Compare side by side (`fthumb://` ids).
#[derive(Serialize)]
struct CompareItem {
    id: u64,
    name: String,
    folder: String,
    size: u64,
    modified: i64,
}

/// Viewer ids for the versions of one duplicate set (current scan only), so
/// Compare can show them at full size. Ids, never paths, go to `fthumb://`.
#[tauri::command]
fn duplicate_items(state: State<'_, AppState>, hash: String) -> Res<Vec<CompareItem>> {
    let s = state.session.lock().map_err(err)?;
    let scan = s.dups.as_ref().ok_or("find duplicates first")?;
    let g = scan.groups.iter().find(|g| g.hash == hash).ok_or("unknown group")?;
    Ok(g.files
        .iter()
        .map(|f| CompareItem {
            id: state.browse.id_for(&f.abs),
            name: f.abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            folder: f.abs.parent().map(|p| p.display().to_string()).unwrap_or_default(),
            size: f.size,
            modified: f.mtime.unwrap_or(0),
        })
        .collect())
}

#[tauri::command]
fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[derive(Serialize)]
struct Info {
    os: &'static str,
    can_set_created: bool,
    version: &'static str,
    /// Installed memory in bytes (None if unknown). Read locally, no network.
    ram_bytes: Option<u64>,
    cpu_threads: usize,
}

#[tauri::command]
fn platform_info() -> Info {
    let i = cs_platform::platform_info();
    Info {
        os: i.os,
        can_set_created: i.can_set_created,
        version: env!("CARGO_PKG_VERSION"),
        ram_bytes: cs_platform::total_ram(),
        cpu_threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
    }
}

/// Command-line start options: `--source DIR --dest DIR [--analyze]`
/// (e.g. for an Explorer "Sort with ChronoSort" entry). Debug builds also
/// accept `--goto plan` to jump ahead for UI screenshots.
#[derive(Serialize, Default)]
struct StartupArgs {
    source: Option<String>,
    dest: Option<String>,
    analyze: bool,
    goto: Option<String>,
}

#[tauri::command]
fn startup_args() -> StartupArgs {
    let mut a = StartupArgs::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--source" => a.source = it.next(),
            "--dest" => a.dest = it.next(),
            "--analyze" => a.analyze = true,
            "--goto" if cfg!(debug_assertions) => a.goto = it.next(),
            _ => {}
        }
    }
    a
}

fn thumb_response(app: &tauri::AppHandle, path: &str, size: u32) -> tauri::http::Response<Vec<u8>> {
    let hash = path.trim_start_matches('/');
    let state = app.state::<AppState>();
    let entry = state.session.lock().ok().and_then(|s| s.by_hash.get(hash).cloned());
    let result = entry.ok_or_else(|| "unknown".to_string()).and_then(|(abs, fmt)| {
        let p = state.thumbs.get_sized(hash, &abs, fmt, size).map_err(|e| e.to_string())?;
        std::fs::read(p).map_err(|e| e.to_string())
    });
    let b = tauri::http::Response::builder().header("Cache-Control", "max-age=31536000, immutable");
    match result {
        Ok(bytes) => b.header("Content-Type", "image/jpeg").status(200).body(bytes),
        Err(e) => b.header("Content-Type", "text/plain").status(404).body(e.into_bytes()),
    }
    .expect("static response")
}

/// Write panics to `<data>/logs/crash.log` (the window app has no console),
/// so a problem can be found later. Nothing leaves the PC.
fn install_crash_log() {
    let dir = cs_ai::app_data_dir().map(|d| d.join("logs"));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(d.join("crash.log")) {
                use std::io::Write;
                let thread = std::thread::current();
                let _ = writeln!(
                    f,
                    "[{}] ChronoSort {} panicked in thread '{}': {info}
{}
",
                    now_unix(),
                    env!("CARGO_PKG_VERSION"),
                    thread.name().unwrap_or("?"),
                    std::backtrace::Backtrace::force_capture()
                );
            }
        }
        default(info);
    }));
}

fn main() {
    // Started as the video-poster helper: do that and end (no window).
    if let Some(code) = cs_thumb::video_helper_main() {
        std::process::exit(code);
    }
    install_crash_log();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cache = cache_dir(app.handle())?.join("thumbs");
            // The window is made here (not from the config alone) so that in
            // portable mode its browser profile (settings, theme) also lives
            // on the stick instead of on this PC.
            let cfg = app.config().app.windows.first().cloned().ok_or("no window in tauri.conf.json")?;
            let mut win = tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?;
            if let Some(p) = cs_ai::portable_root() {
                win = win.data_directory(p.join("webview"));
            }
            win.build()?;
            refresh_ffmpeg(app.handle());
            app.manage(AppState {
                session: Mutex::new(Session::default()),
                thumbs: cs_thumb::Cache::new(cache),
                download: Mutex::new(None),
                cancel: std::sync::atomic::AtomicBool::new(false),
                faces: Mutex::new(None),
                vision: vision::Vision::default(),
                browse: browse::Registry::default(),
            });
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("thumb", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let path = req.uri().path().to_string();
            std::thread::spawn(move || responder.respond(thumb_response(&app, &path, cs_thumb::DEFAULT_SIZE)));
        })
        // Same rules as thumb://, larger image (only files of the current analysis / duplicate scan).
        .register_asynchronous_uri_scheme_protocol("preview", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let path = req.uri().path().to_string();
            std::thread::spawn(move || responder.respond(thumb_response(&app, &path, cs_thumb::PREVIEW_SIZE)));
        })
        // The picture being edited in the viewer (the number only defeats caching).
        .register_asynchronous_uri_scheme_protocol("edit", |_ctx, req, responder| {
            let raw = req.uri().path().ends_with("-raw");
            std::thread::spawn(move || responder.respond(edit::edit_response(raw)));
        })
        // File browser thumbnails: only files handed out by recent listings (ids, never paths).
        .register_asynchronous_uri_scheme_protocol("fthumb", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let path = req.uri().path().to_string();
            std::thread::spawn(move || responder.respond(browse::fthumb_response(&app, &path)));
        })
        // Videos and music for the slideshow, in byte ranges (only ids the app handed out).
        .register_asynchronous_uri_scheme_protocol("media", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let path = req.uri().path().to_string();
            let range = req.headers().get("range").and_then(|v| v.to_str().ok()).map(str::to_string);
            std::thread::spawn(move || responder.respond(slideshow::media_response(&app, &path, range.as_deref())));
        })
        // Face crops of the current face search, and stored pictures of named people.
        .register_asynchronous_uri_scheme_protocol("face", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let path = req.uri().path().to_string();
            std::thread::spawn(move || responder.respond(people::face_response(&app, &path)));
        })
        .register_asynchronous_uri_scheme_protocol("person", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let path = req.uri().path().to_string();
            std::thread::spawn(move || responder.respond(people::person_response(&app, &path)));
        })
        .invoke_handler(tauri::generate_handler![
            analyze,
            make_plan,
            run,
            undo,
            platform_info,
            startup_args,
            check_naming,
            find_duplicates,
            resolve_duplicates,
            save_duplicate_report,
            undo_duplicates,
            open_folder,
            show_duplicate,
            duplicate_items,
            rating::marks,
            rating::set_rating,
            rating::set_tagged,
            ai_overview,
            link_model_file,
            unlink_model,
            download_pack,
            cancel_download,
            delete_model_files,
            browse::browse_roots,
            browse::list_dir,
            browse::list_subdirs,
            browse::folder_samples,
            browse::reveal_item,
            browse::image_size,
            fileops::fs_bin_status,
            fileops::fs_check_name,
            fileops::fs_new_folder,
            fileops::fs_rename,
            fileops::fs_transfer,
            fileops::fs_delete,
            fileops::fs_cancel,
            fileops::fs_last_undo,
            fileops::fs_undo,
            edit::edit_open,
            edit::edit_apply,
            edit::edit_step_back,
            edit::edit_close,
            edit::edit_adjust,
            edit::edit_auto,
            edit::edit_controls,
            edit::edit_set_dpi,
            edit::edit_effects,
            histogram::image_histogram,
            desktop::set_wallpaper,
            privacy::jpeg_lossless_crop,
            privacy::strip_meta,
            thumbcache::thumb_cache_info,
            thumbcache::thumb_cache_clear,
            thumbcache::thumb_cache_trim,
            thumbcache::thumb_cache_clear_old,
            thumbcache::thumb_cache_prescan,
            thumbcache::thumb_cache_cancel,
            create::create_sizes,
            create::create_sheet,
            create::create_pdf,
            create::create_zip,
            batch::batch_convert,
            batch::batch_rename,
            batch::batch_rename_preview,
            batch::batch_cancel,
            slideshow::slideshow_music,
            slideshow::slideshow_awake,
            slideshow::slideshow_playable,
            edit::edit_watermark,
            edit::edit_retouch,
            edit::edit_save,
            edit::edit_formats,
            meta::meta_read,
            meta::meta_write,
            people::find_faces,
            people::suggest_person,
            people::save_person,
            people::list_people,
            people::rename_person,
            people::forget_person,
            people::merge_people,
            people::forget_everyone,
            people::set_birth_year,
            people::show_face_photo,
            portable::portable_info,
            review::save_review,
            places::places_scan,
            places::places_search,
            places::places_apply,
            places::places_cancel,
            places::places_undo,
            review::load_review,
            portable::make_portable,
            vision::ai_date,
            vision::ai_cancel,
            vision::ai_stop,
            exit_app
        ])
        .build(tauri::generate_context!())
        .expect("error while building ChronoSort")
        .run(|app, ev| {
            // Never leave the AI engine running after ChronoSort closes.
            if let tauri::RunEvent::Exit = ev
                && let Some(st) = app.try_state::<AppState>()
            {
                st.vision.stop();
            }
        });
}
