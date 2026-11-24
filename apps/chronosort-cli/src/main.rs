//! ChronoSort command-line runner (headless). The Tauri app uses the same crates.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use cs_model::Decision;
use cs_plan::{MonthPolicy, Naming, Plan, PlanOptions, Skipped};
use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "chronosort-cli", version, about = "Offline photo & video sorter into year folders")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show platform capabilities and free space for a path.
    Info {
        /// Folder to check (default: current directory).
        path: Option<PathBuf>,
    },
    /// Find faces in a folder (read-only) and group them into people.
    Faces {
        /// Folder with photos.
        source: PathBuf,
        /// Folder with the face models (default: the app's downloaded faces pack).
        #[arg(long)]
        models: Option<PathBuf>,
        /// Write each group's aligned face crops into DIR/person_NN/ to check by eye.
        #[arg(long)]
        crops: Option<PathBuf>,
        /// Write the groups as JSON.
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Generate the synthetic edge-case corpus (Old_Photos + manifest.json).
    Testgen {
        /// Output folder; must not already contain Old_Photos.
        out: PathBuf,
    },
    /// Analyse a folder (read-only): decide a year for every photo/video.
    Analyze {
        /// Source folder, e.g. D:\Old_Photos
        source: PathBuf,
        /// Write the full per-file report (with evidence) as JSON.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Print every file, not just the summary.
        #[arg(long, short)]
        verbose: bool,
    },
    /// Show what `run` would do (dry run). Writes nothing.
    Plan {
        #[command(flatten)]
        args: SortArgs,
        /// Print every planned copy.
        #[arg(long, short)]
        verbose: bool,
    },
    /// Sort: analyse, plan, confirm, then copy (or move) with verification.
    Run {
        #[command(flatten)]
        args: SortArgs,
        /// Move instead of copy: originals go to the OS trash only after a verified copy.
        #[arg(long = "move")]
        move_files: bool,
        /// Do not ask for confirmation.
        #[arg(long, short)]
        yes: bool,
        /// Parallel copies of small files (1 = strictly one at a time).
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u16).range(1..=32))]
        threads: u16,
    },
    /// Reverse a run (default: the latest) in a destination folder.
    Undo {
        dest: PathBuf,
        #[arg(long)]
        run: Option<i64>,
        #[arg(long, short)]
        yes: bool,
    },
    /// Run the analysis on a generated corpus and compare with its manifest.
    Eval {
        /// Folder containing Old_Photos/ and manifest.json (from `testgen`).
        corpus: PathBuf,
    },
}

#[derive(clap::Args)]
struct SortArgs {
    /// Source folder (never modified in copy mode), e.g. D:\Old_Photos
    source: PathBuf,
    /// Destination folder, e.g. D:\Sorted_Photos
    dest: PathBuf,
    /// Source month folders (2005\03\...): keep them under the year, or flatten into the year.
    /// If not given you are asked when month folders are found.
    #[arg(long, value_parser = ["keep", "flatten"])]
    months: Option<String>,
    /// File names: keep (default), datetime (2005-03-12_14-22-07), or a template like
    /// "{year}-{month}-{day}_{original}". Clashes always get _1, _2, ... at the end.
    #[arg(long, default_value = "keep")]
    rename: String,
    /// Accept the suggested year for files that would need review
    /// (otherwise they are left for the review screen).
    #[arg(long)]
    accept_suggested: bool,
}

impl SortArgs {
    fn options(&self) -> Result<PlanOptions> {
        Ok(PlanOptions {
            months: match self.months.as_deref() {
                Some("keep") => MonthPolicy::Keep,
                Some("flatten") => MonthPolicy::Flatten,
                _ => MonthPolicy::Ask,
            },
            naming: Naming::parse(&self.rename).map_err(|e| anyhow::anyhow!("--rename: {e}"))?,
            accept_suggested: self.accept_suggested,
            ..Default::default()
        })
    }
}

fn ask(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    Ok(s.trim().to_lowercase())
}

/// Analyse + plan; resolve the month question interactively when possible.
fn make_plan(a: &SortArgs, interactive: bool) -> Result<(Vec<cs_model::ItemReport>, Plan)> {
    let analysis = cs_core::analyze(&a.source, &cs_core::DatePolicy::new(now_unix()))
        .with_context(|| format!("analysing {}", a.source.display()))?;
    let mut opts = a.options()?;
    let mut plan = cs_plan::build(&analysis.items, &a.dest, &opts)?;
    if let Some(q) = &plan.month_question {
        println!("\nThe source is already organised into month folders:");
        for (folder, n) in q.folders.iter().take(12) {
            println!("  {folder}  ({n} files)");
        }
        if q.folders.len() > 12 {
            println!("  ... and {} more", q.folders.len() - 12);
        }
        if interactive && std::io::stdin().is_terminal() {
            loop {
                let a = ask(
                    "Keep the month folders inside the year folder (k), or put everything straight into the year folder (f)? [k/f] ",
                )?;
                match a.as_str() {
                    "k" | "keep" => break opts.months = MonthPolicy::Keep,
                    "f" | "flatten" => break opts.months = MonthPolicy::Flatten,
                    _ => {}
                }
            }
            plan = cs_plan::build(&analysis.items, &a.dest, &opts)?;
        } else {
            println!("Choose with --months keep  or  --months flatten");
        }
    }
    Ok((analysis.items, plan))
}

fn gib(b: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{b} bytes") } else { format!("{v:.1} {}", units[u]) }
}

fn print_plan(items: &[cs_model::ItemReport], plan: &Plan, verbose: bool) {
    let mut per_year: BTreeMap<i16, usize> = BTreeMap::new();
    for f in &plan.files {
        *per_year.entry(f.year).or_default() += 1;
        if verbose {
            println!("  {}  ->  {}", f.src_rel, f.dst_rel);
        }
    }
    let count = |pred: fn(&Skipped) -> bool| plan.skipped.iter().filter(|s| pred(s)).count();
    println!("\nPlan: {} files ({}) into {} year folders", plan.files.len(), gib(plan.total_bytes), per_year.len());
    for (y, n) in &per_year {
        println!("  {y}: {n}");
    }
    println!("  duplicates not copied:           {}", count(|s| matches!(s, Skipped::Duplicate { .. })));
    println!("  already in destination:          {}", count(|s| matches!(s, Skipped::AlreadyInDestination { .. })));
    println!("  space saved by de-duplication:   {}", gib(plan.bytes_saved_by_dedup(items)));
    println!("  waiting for review (not copied): {}", count(|s| matches!(s, Skipped::Unresolved { .. })));
    println!("  junk skipped:                    {}", count(|s| matches!(s, Skipped::Junk { .. })));
    let renamed = plan.files.iter().filter(|f| f.dst_rel.rsplit('/').next() != f.src_rel.rsplit('/').next()).count();
    if renamed > 0 {
        println!("  renamed (scheme or name clash):  {renamed}");
    }
    for n in &plan.notes {
        println!("  note: {n}");
    }
}

fn now_unix() -> i64 {
    cs_model::civil::system_to_unix(std::time::SystemTime::now())
}

fn line(i: &cs_model::ItemReport) -> String {
    let what = match &i.decision {
        Decision::Auto { year, confidence } => format!("AUTO   {year}  ({:.0}%)", confidence * 100.0),
        Decision::Review { suggested, reason, .. } => {
            format!("REVIEW {}  {reason}", suggested.map_or("????".into(), |y| y.to_string()))
        }
        Decision::Skip { reason } => format!("SKIP         {reason}"),
    };
    let dup = i.duplicate_of.as_ref().map(|d| format!("  [duplicate of {d}]")).unwrap_or_default();
    format!("{what:<60} {}{dup}", i.rel_path)
}

fn main() -> Result<()> {
    if let Some(code) = cs_thumb::video_helper_main() {
        std::process::exit(code);
    }
    match Cli::parse().cmd {
        Cmd::Info { path } => {
            let p = path.unwrap_or_else(|| PathBuf::from("."));
            let info = cs_platform::platform_info();
            let free = cs_platform::free_space(&p).with_context(|| format!("free space of {}", p.display()))?;
            let db = cs_db::Db::open_in_memory()?;
            println!("ChronoSort {}", env!("CARGO_PKG_VERSION"));
            println!("  os / arch          : {} / {}", info.os, info.arch);
            println!("  can set created    : {}", info.can_set_created);
            if let Some(r) = cs_platform::total_ram() {
                println!("  memory (RAM)       : {:.1} GiB", r as f64 / (1u64 << 30) as f64);
            }
            for g in cs_platform::gpus() {
                let vram =
                    g.vram_bytes.map_or("unknown".into(), |b| format!("{:.1} GiB", b as f64 / (1u64 << 30) as f64));
                let kind = if g.integrated { "integrated" } else { "dedicated" };
                println!("  graphics           : {} ({kind}, {vram})", g.name);
            }
            println!("  free space         : {:.1} GiB at {}", free as f64 / (1u64 << 30) as f64, p.display());
            println!("  db schema version  : {}", db.schema_version()?);
        }
        Cmd::Faces { source, models, crops, json } => faces_cmd(&source, models, crops, json)?,
        Cmd::Testgen { out } => {
            let m = cs_testgen::generate(&out).with_context(|| format!("generating into {}", out.display()))?;
            let count = |r| m.entries.iter().filter(|e| e.route == r).count();
            println!(
                "Wrote {} files to {}  (auto {}, review {}, skip {})",
                m.entries.len(),
                out.join(cs_testgen::SOURCE_DIR).display(),
                count(cs_testgen::Route::Auto),
                count(cs_testgen::Route::Review),
                count(cs_testgen::Route::Skip),
            );
            println!("Expected results: {}", out.join("manifest.json").display());
        }
        Cmd::Analyze { source, json, verbose } => {
            let t0 = std::time::Instant::now();
            let a = cs_core::analyze(&source, &cs_core::DatePolicy::new(now_unix()))
                .with_context(|| format!("analysing {}", source.display()))?;
            let mut years: BTreeMap<i16, usize> = BTreeMap::new();
            let (mut review, mut skip, mut dups) = (0, 0, 0);
            for i in &a.items {
                match i.decision {
                    Decision::Auto { year, .. } => *years.entry(year).or_default() += 1,
                    Decision::Review { .. } => review += 1,
                    Decision::Skip { .. } => skip += 1,
                }
                dups += usize::from(i.duplicate_of.is_some());
                if verbose {
                    println!("{}", line(i));
                }
            }
            println!("\n{} files analysed in {:.2?} (read-only)", a.items.len(), t0.elapsed());
            for (y, n) in &years {
                println!("  {y}: {n}");
            }
            println!("  needs review: {review}\n  skipped junk: {skip}\n  exact duplicates: {dups}");
            for (p, e) in &a.errors {
                println!("  ERROR {}: {e}", p.display());
            }
            for p in &a.skipped_links {
                println!("  not followed (link/junction): {}", p.display());
            }
            if let Some(out) = json {
                std::fs::write(&out, serde_json::to_vec_pretty(&a.items)?)?;
                println!("Report written to {}", out.display());
            }
        }
        Cmd::Plan { args, verbose } => {
            let (items, plan) = make_plan(&args, false)?;
            print_plan(&items, &plan, verbose);
            println!("\nDry run only: nothing was written.");
        }
        Cmd::Run { args, move_files, yes, threads } => {
            let (items, plan) = make_plan(&args, !yes)?;
            if !plan.ready() {
                bail!("the month-folder question must be answered (use --months keep|flatten)");
            }
            print_plan(&items, &plan, false);
            if plan.files.is_empty() {
                println!("\nNothing to do.");
                return Ok(());
            }
            let mode = if move_files { cs_model::TransferMode::Move } else { cs_model::TransferMode::Copy };
            let verb = if move_files { "MOVE (originals go to the trash after a verified copy)" } else { "COPY" };
            println!("\n{verb}: {} -> {}", args.source.display(), args.dest.display());
            if !yes && ask("Proceed? [y/N] ")? != "y" {
                println!("Cancelled. Nothing was written.");
                return Ok(());
            }
            let t0 = std::time::Instant::now();
            let opts = cs_exec::ExecOptions { mode, threads: threads.into(), ..Default::default() };
            let r = cs_exec::execute(&args.source, &args.dest, &plan, &opts)?;
            println!(
                "\nRun {}: {} files ({}) copied and verified in {:.1?}",
                r.run_id,
                r.copied,
                gib(r.bytes),
                t0.elapsed()
            );
            if r.trashed > 0 {
                println!("  originals moved to the trash: {}", r.trashed);
            }
            if r.already_present > 0 {
                println!("  already present (identical): {}", r.already_present);
            }
            for w in &r.warnings {
                println!("  warning: {w}");
            }
            for (p, e) in &r.failed {
                println!("  FAILED {p}: {e}");
            }
            println!("Undo with: chronosort-cli undo \"{}\"", args.dest.display());
            if !r.failed.is_empty() {
                bail!("{} files failed (the originals are untouched)", r.failed.len());
            }
        }
        Cmd::Undo { dest, run, yes } => {
            if !cs_exec::db_path(&dest).exists() {
                bail!("no ChronoSort runs found in {}", dest.display());
            }
            let which = run.map_or("latest".to_string(), |r| format!("#{r}"));
            if !yes && ask(&format!("Undo the {which} run in {}? [y/N] ", dest.display()))? != "y" {
                return Ok(());
            }
            let r = cs_exec::undo(&dest, run)?;
            println!(
                "Run {} undone: {} copies removed, {} originals restored, {} folders removed",
                r.run_id, r.removed, r.restored, r.dirs_removed
            );
            for k in &r.kept_modified {
                println!("  kept (changed since it was copied): {k}");
            }
            for (p, e) in &r.failed {
                println!("  FAILED {p}: {e}");
            }
        }
        Cmd::Eval { corpus } => {
            let m: cs_testgen::Manifest =
                serde_json::from_slice(&std::fs::read(corpus.join("manifest.json")).context("reading manifest.json")?)?;
            let a = cs_core::analyze(&corpus.join(&m.source_dir), &cs_core::DatePolicy::new(now_unix()))?;
            let r = cs_testgen::eval::evaluate(&m, &a.items);
            for (p, want, got) in &r.failures {
                println!("FAIL {p}\n     expected: {want}\n     got:      {got}");
            }
            println!("{} / {} files as expected", r.passed, r.checked);
            if !r.ok() {
                bail!("evaluation failed");
            }
        }
    }
    Ok(())
}

fn faces_cmd(
    source: &std::path::Path,
    models: Option<PathBuf>,
    crops: Option<PathBuf>,
    json: Option<PathBuf>,
) -> anyhow::Result<()> {
    let dir = models
        .or_else(|| cs_ai::pack_dir("faces-opencv"))
        .ok_or_else(|| anyhow::anyhow!("no models folder; pass --models"))?;
    let det = dir.join("face_detection_yunet_2023mar.onnx");
    let rec = dir.join("face_recognition_sface_2021dec.onnx");
    if !det.is_file() || !rec.is_file() {
        anyhow::bail!(
            "face models not found in {}.
Download them in the app (Settings -> AI models -> Faces) or pass --models.",
            dir.display()
        );
    }
    let t = std::time::Instant::now();
    let m = cs_faces::FaceModels::load(&det, &rec)?;
    let progress = |done: usize, total: usize| {
        if done.is_multiple_of(25) || done == total {
            eprint!(
                "
  {done}/{total} photos"
            );
        }
    };
    let scan = cs_faces::folder::scan_folder(source, &m, &progress)?;
    eprintln!();
    println!(
        "{} photos, {} with faces, {} faces, {} groups ({:.1?})",
        scan.photos,
        scan.photos_with_faces,
        scan.faces.len(),
        scan.groups.len(),
        t.elapsed()
    );
    // Contact sheet: one row per group (first 14 faces), 112 px crops.
    const COLS: u32 = 14;
    let multi: Vec<&Vec<usize>> = scan.groups.iter().filter(|g| g.len() > 1).collect();
    let mut sheet = crops.as_ref().map(|_| {
        image::RgbImage::from_pixel(COLS * 116 + 4, multi.len().max(1) as u32 * 116 + 4, image::Rgb([40, 40, 40]))
    });
    let mut row = 0u32;
    for (gi, g) in scan.groups.iter().enumerate() {
        let photos: std::collections::BTreeSet<&str> = g.iter().map(|&i| scan.faces[i].rel.as_str()).collect();
        println!("person {:02}: {} faces in {} photos", gi + 1, g.len(), photos.len());
        for p in photos.iter().take(6) {
            println!("    {p}");
        }
        if photos.len() > 6 {
            println!("    ... and {} more", photos.len() - 6);
        }
        if let Some(c) = &crops {
            let d = c.join(format!("person_{:02}", gi + 1));
            std::fs::create_dir_all(&d)?;
            for &i in g {
                let f = &scan.faces[i];
                let abs = source.join(&f.rel);
                let fmt = cs_scan::Format::from_name(&f.format);
                if let Ok(img) = cs_thumb::load(&abs, fmt, cs_faces::folder::LOAD_SIDE) {
                    let name = f.rel.replace('/', "__");
                    let crop = cs_faces::align(&img.to_rgb8(), &f.face.landmarks);
                    crop.save(d.join(format!("{i:04}_{name}.png")))?;
                    let k = g.iter().position(|&x| x == i).unwrap_or(0) as u32;
                    if let Some(sh) = sheet.as_mut()
                        && g.len() > 1
                        && k < COLS
                    {
                        image::imageops::replace(sh, &crop, (4 + k * 116) as i64, (4 + row * 116) as i64);
                    }
                }
            }
        }
        row += u32::from(g.len() > 1);
    }
    if let (Some(sh), Some(c)) = (sheet, &crops) {
        sh.save(c.join("groups.png"))?;
        println!("contact sheet: {}", c.join("groups.png").display());
    }
    for (p, e) in &scan.errors {
        println!("could not read {p}: {e}");
    }
    if let Some(j) = json {
        std::fs::write(&j, serde_json::to_vec_pretty(&scan)?)?;
    }
    Ok(())
}
