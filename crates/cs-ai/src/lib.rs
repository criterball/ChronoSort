//! `cs-ai`: which AI models fit this PC, and which ones the user has.
//!
//! Pure logic plus one small JSON file, no network. Hardware is read by
//! `cs-platform`; this crate turns it into a tier, a plain-language reason,
//! and a recommendation from the built-in catalogue (`catalogue.json`).
//!
//! Model files are never copied or changed: the user points ChronoSort at a
//! file they already have (or, later, an optional verified download), and only
//! its path, size and modified time are recorded in `models.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub mod download;
pub mod unpack;

const GIB: f64 = (1u64 << 30) as f64;

/// A file with this name next to the program switches on portable mode.
pub const PORTABLE_MARKER: &str = "portable.txt";
/// In portable mode everything ChronoSort keeps lives in this folder next to the program.
pub const PORTABLE_DIR: &str = "ChronoSortData";

/// Portable mode (e.g. a USB stick): `<program folder>/ChronoSortData` when
/// `portable.txt` sits next to the program.
pub fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join(PORTABLE_MARKER).is_file().then(|| dir.join(PORTABLE_DIR))
}

/// ChronoSort's data folder: the portable folder if there is one, else the
/// same folder the desktop app uses (Tauri `app_data_dir` for `org.chronosort.app`).
pub fn app_data_dir() -> Option<PathBuf> {
    if let Some(p) = portable_root() {
        return Some(p);
    }
    installed_data_dir()
}

fn installed_data_dir() -> Option<PathBuf> {
    const ID: &str = "org.chronosort.app";
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        env("APPDATA").map(|d| d.join(ID))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support").join(ID))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share"))).map(|d| d.join(ID))
    }
}

/// Where a downloaded pack's files live: `<app data>/models/<pack id>/`.
pub fn pack_dir(pack_id: &str) -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("models").join(pack_id))
}

// ------------------------------------------------------------------ tiers

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Too little memory for AI: dates from file data only.
    Metadata,
    Light,
    Standard,
    Max,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Hardware {
    pub ram_bytes: Option<u64>,
    /// Dedicated VRAM of the best graphics card (0 if none / integrated only).
    pub vram_bytes: u64,
    pub gpu_name: Option<String>,
}

impl Hardware {
    pub fn detect() -> Self {
        let best = cs_platform::gpus().into_iter().find(|g| g.useful_vram() > 0);
        Hardware {
            ram_bytes: cs_platform::total_ram(),
            vram_bytes: best.as_ref().map_or(0, |g| g.useful_vram()),
            gpu_name: best.map(|g| g.name),
        }
    }
    fn ram_gb(&self) -> f64 {
        self.ram_bytes.map_or(0.0, |b| b as f64 / GIB)
    }
    fn vram_gb(&self) -> f64 {
        self.vram_bytes as f64 / GIB
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Assessment {
    pub tier: Tier,
    /// One sentence for the user, e.g. "12 GB graphics memory and 64 GB RAM: …".
    pub reason: String,
}

/// Thresholds sit a little under the marketing sizes: a "16 GB" PC reports
/// ~15.7 GiB and a "6 GB" card ~5.9 GiB.
pub fn assess(hw: &Hardware) -> Assessment {
    let (ram, vram) = (hw.ram_gb(), hw.vram_gb());
    let gpu = match &hw.gpu_name {
        Some(n) if vram > 0.0 => format!("{} with {:.0} GB graphics memory", n, vram),
        _ => "no separate graphics card".to_string(),
    };
    let ram_txt = if hw.ram_bytes.is_some() { format!("{ram:.0} GB memory") } else { "unknown memory".into() };
    let (tier, why) = if vram >= 11.5 && ram >= 30.0 {
        (Tier::Max, "enough for the most accurate models, fully on the graphics card")
    } else if vram >= 5.5 && ram >= 14.0 {
        (Tier::Standard, "good for the standard models on the graphics card")
    } else if ram >= 7.5 {
        (Tier::Light, "the light models run on the processor: slower, but they only look at uncertain photos")
    } else {
        (Tier::Metadata, "too little memory for AI; ChronoSort uses the dates stored in the files")
    };
    Assessment { tier, reason: format!("{gpu}, {ram_txt}: {why}.") }
}

// ------------------------------------------------------------------ catalogue

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFile {
    pub role: String,
    pub name: String,
    pub approx_mb: f64,
    /// Exact size, when verified.
    #[serde(default)]
    pub bytes: Option<u64>,
    pub url: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    pub id: String,
    /// "faces", "vision" (models) or "engine" (a program, e.g. llama.cpp)
    pub purpose: String,
    /// Only for this OS ("windows", "linux", "macos"); None = everywhere.
    #[serde(default)]
    pub os: Option<String>,
    /// The single file is a zip to unpack (programs); `entry` is the main file inside.
    #[serde(default)]
    pub unpack: bool,
    #[serde(default)]
    pub entry: Option<String>,
    pub name: String,
    pub description: String,
    pub licence: String,
    /// The tier this pack is recommended for.
    pub tier: Tier,
    /// Graphics memory needed to run fully on the card.
    pub vram_gb: f64,
    /// System memory needed to run on the processor.
    pub ram_gb: f64,
    /// Sources, sizes and checksums checked. Downloads need this.
    pub verified: bool,
    pub files: Vec<ModelFile>,
}

impl Pack {
    pub fn approx_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes.unwrap_or((f.approx_mb * 1_048_576.0) as u64)).sum()
    }
    /// Every file has a url and a checksum, and the entry has been reviewed.
    pub fn downloadable(&self) -> bool {
        self.verified && self.files.iter().all(|f| f.url.is_some() && f.sha256.as_deref().is_some_and(is_sha256))
    }
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    pub schema: u32,
    pub packs: Vec<Pack>,
}

/// The built-in catalogue, without packs meant for other operating systems.
pub fn catalogue() -> Catalogue {
    let mut c = full_catalogue();
    c.packs.retain(|p| p.os.as_deref().is_none_or(|o| o == std::env::consts::OS));
    c
}

fn full_catalogue() -> Catalogue {
    serde_json::from_str(include_str!("../catalogue.json")).expect("built-in catalogue is valid (tested)")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Runs fully on the graphics card.
    Gpu,
    /// Runs on the processor (slower).
    Cpu,
    TooBig,
}

pub fn fit(pack: &Pack, hw: &Hardware) -> Fit {
    if pack.vram_gb <= 0.0 && hw.ram_gb() >= pack.ram_gb {
        Fit::Cpu // tiny models (faces) run on the processor anyway
    } else if hw.vram_gb() + 0.25 >= pack.vram_gb && pack.vram_gb > 0.0 && hw.ram_gb() >= pack.ram_gb * 0.5 {
        Fit::Gpu
    } else if hw.ram_gb() >= pack.ram_gb {
        Fit::Cpu
    } else {
        Fit::TooBig
    }
}

/// Best pack per purpose for this tier: the highest tier not above the PC's.
/// Packs the user already has win among equals.
pub fn recommend(cat: &Catalogue, tier: Tier, have: &dyn Fn(&str) -> bool) -> BTreeMap<String, String> {
    let mut best: BTreeMap<String, &Pack> = BTreeMap::new();
    if tier == Tier::Metadata {
        return BTreeMap::new();
    }
    for p in cat.packs.iter().filter(|p| p.tier <= tier) {
        let better = match best.get(&p.purpose) {
            None => true,
            Some(b) => (p.tier, have(&p.id)) > (b.tier, have(&b.id)),
        };
        if better {
            best.insert(p.purpose.clone(), p);
        }
    }
    best.into_iter().map(|(k, p)| (k, p.id.clone())).collect()
}

// ------------------------------------------------------------------ the user's files

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedFile {
    pub path: PathBuf,
    pub size: u64,
    /// Unix seconds; a change means the file was replaced since it was linked.
    pub modified: i64,
    /// Size and SHA-256 matched the catalogue (the exact checked file).
    #[serde(default)]
    pub verified: bool,
}

impl LinkedFile {
    /// Same path and size, and the same time allowing for USB sticks: FAT
    /// stores times to 2 s and in local time, so a time-zone change shifts
    /// them by whole quarter hours.
    fn same_file(&self, other: &LinkedFile) -> bool {
        let d = (self.modified - other.modified).abs();
        self.path == other.path && self.size == other.size && (d <= 2 || (d % 900).min(900 - d % 900) <= 2)
    }
}

/// `models.json`: pack id -> role -> file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Linked {
    pub packs: BTreeMap<String, BTreeMap<String, LinkedFile>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackStatus {
    Missing,
    /// Some files linked, not all.
    Partial,
    Ready,
    /// A linked file is gone or was changed since.
    Changed,
}

impl Linked {
    /// Paths stored relative to the file's folder (models ChronoSort keeps
    /// itself) are made absolute again, so the folder can move (USB stick,
    /// another drive letter).
    pub fn load(path: &Path) -> io::Result<Self> {
        let mut l: Self = match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let base = path.parent().unwrap_or(Path::new("."));
        for f in l.packs.values_mut().flat_map(|m| m.values_mut()) {
            if f.path.is_relative() {
                f.path = base.join(&f.path);
            }
        }
        Ok(l)
    }

    /// Written to a temp file first, then renamed, so a crash never leaves half a file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let base = path.parent().unwrap_or(Path::new("."));
        let mut out = self.clone();
        for f in out.packs.values_mut().flat_map(|m| m.values_mut()) {
            if let Ok(rel) = f.path.strip_prefix(base) {
                f.path = rel.to_path_buf();
            }
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&out).map_err(io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    pub fn status(&self, pack: &Pack) -> PackStatus {
        let Some(have) = self.packs.get(&pack.id) else { return PackStatus::Missing };
        let mut n = 0;
        for f in &pack.files {
            if let Some(l) = have.get(&f.role) {
                if !stat(&l.path).is_ok_and(|now| now.same_file(l)) {
                    return PackStatus::Changed;
                }
                n += 1;
            }
        }
        match n {
            0 => PackStatus::Missing,
            n if n == pack.files.len() => PackStatus::Ready,
            _ => PackStatus::Partial,
        }
    }

    /// Check a file the user picked and remember it. The file is only read.
    ///
    /// If its size matches the catalogue exactly, the SHA-256 is computed too
    /// (`progress(done, total)`): a match marks it `verified`. A different but
    /// plausible file (another publisher's copy) is accepted as unverified;
    /// clear mistakes (wrong type, far off in size) are refused.
    pub fn link(
        &mut self,
        pack: &Pack,
        role: &str,
        path: &Path,
        progress: &dyn Fn(u64, u64),
    ) -> Result<LinkedFile, LinkError> {
        let spec = pack.files.iter().find(|f| f.role == role).ok_or(LinkError::UnknownRole)?;
        check_kind(&spec.name, path)?;
        let mut l = stat(path).map_err(LinkError::Io)?;
        let mb = l.size as f64 / 1_048_576.0;
        if spec.approx_mb >= 50.0 && (mb < spec.approx_mb * 0.5 || mb > spec.approx_mb * 2.0) {
            return Err(LinkError::Size { expected_mb: spec.approx_mb, found_mb: mb });
        }
        if let (Some(bytes), Some(want)) = (spec.bytes, spec.sha256.as_deref())
            && bytes == l.size
        {
            let got = download::sha256_file(path, progress).map_err(LinkError::Io)?;
            l.verified = got.eq_ignore_ascii_case(want);
        }
        self.packs.entry(pack.id.clone()).or_default().insert(role.to_string(), l.clone());
        Ok(l)
    }

    /// Record a file that was just downloaded and verified.
    pub fn link_verified(&mut self, pack_id: &str, role: &str, path: &Path) -> io::Result<()> {
        let mut l = stat(path)?;
        l.verified = true;
        self.packs.entry(pack_id.to_string()).or_default().insert(role.to_string(), l);
        Ok(())
    }

    /// All linked files of a pack match the catalogue exactly.
    pub fn verified(&self, pack: &Pack) -> bool {
        self.packs.get(&pack.id).is_some_and(|m| pack.files.iter().all(|f| m.get(&f.role).is_some_and(|l| l.verified)))
    }

    pub fn unlink(&mut self, pack_id: &str) {
        self.packs.remove(pack_id);
    }
}

fn stat(path: &Path) -> io::Result<LinkedFile> {
    let m = std::fs::metadata(path)?;
    if !m.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
    }
    let modified = m.modified()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    Ok(LinkedFile { path: path.to_path_buf(), size: m.len(), modified, verified: false })
}

#[derive(Debug)]
pub enum LinkError {
    UnknownRole,
    WrongType(&'static str),
    Size { expected_mb: f64, found_mb: f64 },
    Io(io::Error),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownRole => write!(f, "This model has no such part."),
            Self::WrongType(t) => write!(f, "This is not a {t} file."),
            Self::Size { expected_mb, found_mb } => write!(
                f,
                "This file is {} but about {} was expected. Is it the right file?",
                human_mb(*found_mb),
                human_mb(*expected_mb)
            ),
            Self::Io(e) => write!(f, "Could not read the file: {e}"),
        }
    }
}
impl std::error::Error for LinkError {}

fn human_mb(mb: f64) -> String {
    if mb >= 1024.0 { format!("{:.1} GB", mb / 1024.0) } else { format!("{mb:.0} MB") }
}

/// GGUF files start with the magic "GGUF". ONNX is protobuf without a magic,
/// so only the extension and a plausible first byte (field 1, varint) are checked.
fn check_kind(expected_name: &str, path: &Path) -> Result<(), LinkError> {
    let want = expected_name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let got = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut head = [0u8; 4];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).map_err(LinkError::Io)?;
    match want.as_str() {
        "gguf" if got == "gguf" && n == 4 && &head == b"GGUF" => Ok(()),
        "gguf" => Err(LinkError::WrongType("GGUF model")),
        "onnx" if got == "onnx" && n >= 1 && head[0] == 0x08 => Ok(()),
        "onnx" => Err(LinkError::WrongType("ONNX model")),
        _ => Err(LinkError::WrongType("model")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_model_paths_are_relative_so_the_folder_can_move() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::create_dir_all(a.join("models/p")).unwrap();
        std::fs::write(a.join("models/p/m.gguf"), b"GGUF....").unwrap();
        let mut l = Linked::default();
        l.link_verified("p", "model", &a.join("models/p/m.gguf")).unwrap();
        l.save(&a.join("models.json")).unwrap();
        assert!(std::fs::read_to_string(a.join("models.json")).unwrap().contains("\"models"), "stored relative");
        std::fs::rename(&a, &b).unwrap(); // "another drive letter"
        let l2 = Linked::load(&b.join("models.json")).unwrap();
        let f = &l2.packs["p"]["model"];
        assert_eq!(f.path, b.join("models").join("p").join("m.gguf"));
        assert!(stat(&f.path).unwrap().same_file(f));
    }

    #[test]
    fn usb_time_shifts_are_tolerated() {
        let f = |m| LinkedFile { path: "x".into(), size: 1, modified: m, verified: true };
        assert!(f(1000).same_file(&f(1002)), "FAT 2-second rounding");
        assert!(f(1000).same_file(&f(1000 + 9 * 3600 + 1)), "time-zone shift");
        assert!(f(1000).same_file(&f(1000 - 5 * 3600 - 1800)), "half-hour zone");
        assert!(!f(1000).same_file(&f(1000 + 100)), "a real change");
    }

    fn hw(ram_gb: f64, vram_gb: f64) -> Hardware {
        Hardware {
            ram_bytes: Some((ram_gb * GIB) as u64),
            vram_bytes: (vram_gb * GIB) as u64,
            gpu_name: (vram_gb > 0.0).then(|| "Test GPU".into()),
        }
    }

    #[test]
    fn catalogue_is_valid() {
        let c = full_catalogue();
        assert!(!c.packs.is_empty());
        let mut ids = std::collections::HashSet::new();
        for p in &c.packs {
            assert!(ids.insert(&p.id), "duplicate id {}", p.id);
            assert!(matches!(p.purpose.as_str(), "faces" | "vision" | "engine" | "helper"));
            if p.unpack {
                assert_eq!(p.files.len(), 1, "{}: one archive", p.id);
                assert!(p.files[0].name.ends_with(".zip") && p.entry.is_some(), "{}", p.id);
            }
            assert!(!p.files.is_empty());
            // Licence rule: commercial-safe and GPL-compatible only. GPL-3.0 (same
            // licence as ChronoSort) only for optional helper programs the user
            // downloads separately, never for models (docs/THIRD_PARTY_POLICY.md).
            for l in p.licence.split(" + ") {
                let ok =
                    matches!(l, "Apache-2.0" | "MIT" | "BSD-3-Clause") || (l == "GPL-3.0" && p.purpose == "helper");
                assert!(ok, "{}: {}", p.id, p.licence);
            }
            assert!(!p.name.to_lowercase().contains("insightface"));
            // Nothing is downloadable until it has been verified.
            if !p.verified {
                assert!(!p.downloadable());
            } else {
                assert!(p.downloadable(), "{} is verified but incomplete", p.id);
                for f in &p.files {
                    let url = f.url.as_deref().unwrap();
                    assert!(download::allowed_url(url), "{url}");
                    assert!(url.ends_with(&format!("/{}", f.name)), "{url}");
                    // Pinned: a 40-hex commit (Hugging Face) or a fixed build tag (GitHub), never /main/ or /latest/.
                    if let Some(rest) = url.split("/resolve/").nth(1) {
                        let rev = rest.split('/').next().unwrap();
                        assert!(rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()), "{url}");
                    } else {
                        let tag = url.split("/releases/download/").nth(1).unwrap().split('/').next().unwrap();
                        // A fixed build or version tag ("b10276", "9.1"), never "latest".
                        let fixed = (tag.starts_with('b') && tag[1..].bytes().all(|b| b.is_ascii_digit()))
                            || (!tag.is_empty() && tag.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
                        assert!(fixed, "{url}");
                    }
                    let b = f.bytes.expect("verified files have an exact size");
                    assert!((b as f64 / 1_048_576.0 - f.approx_mb).abs() < 2.0, "{} approx_mb", f.name);
                }
            }
        }
    }

    #[test]
    fn tiers_match_the_research_table() {
        assert_eq!(assess(&hw(63.7, 12.0)).tier, Tier::Max); // dev PC: RTX 3060 12 GB, 64 GB
        assert_eq!(assess(&hw(15.7, 5.9)).tier, Tier::Standard); // 16 GB / 6 GB laptop
        assert_eq!(assess(&hw(15.7, 12.0)).tier, Tier::Standard); // big card, little RAM
        assert_eq!(assess(&hw(15.7, 0.0)).tier, Tier::Light);
        assert_eq!(assess(&hw(7.8, 0.0)).tier, Tier::Light);
        assert_eq!(assess(&hw(3.9, 0.0)).tier, Tier::Metadata);
        assert_eq!(assess(&Hardware::default()).tier, Tier::Metadata);
    }

    #[test]
    fn reason_is_plain_language() {
        let a = assess(&hw(63.7, 12.0));
        assert!(a.reason.contains("12 GB graphics memory"), "{}", a.reason);
        assert!(a.reason.contains("64 GB memory"), "{}", a.reason);
    }

    #[test]
    fn recommendations_per_tier() {
        let c = catalogue();
        let none = |_: &str| false;
        let max = recommend(&c, Tier::Max, &none);
        assert_eq!(max["faces"], "faces-opencv");
        if cfg!(windows) {
            assert_eq!(max["engine"], "llama-cpp-vulkan");
            assert_eq!(recommend(&c, Tier::Light, &none)["engine"], "llama-cpp-cpu");
        }
        assert_eq!(max["vision"], "qwen3-vl-8b");
        assert_eq!(recommend(&c, Tier::Standard, &none)["vision"], "qwen3-vl-4b");
        assert_eq!(recommend(&c, Tier::Light, &none)["vision"], "qwen3-vl-2b");
        assert!(recommend(&c, Tier::Metadata, &none).is_empty());
        // A pack the user already has wins over an equal-tier one.
        let have = |id: &str| id == "qwen3.5-9b";
        assert_eq!(recommend(&c, Tier::Max, &have)["vision"], "qwen3.5-9b");
    }

    #[test]
    fn fit_levels() {
        let c = catalogue();
        let p8 = c.packs.iter().find(|p| p.id == "qwen3-vl-8b").unwrap();
        assert_eq!(fit(p8, &hw(64.0, 12.0)), Fit::Gpu);
        assert_eq!(fit(p8, &hw(32.0, 0.0)), Fit::Cpu);
        assert_eq!(fit(p8, &hw(8.0, 0.0)), Fit::TooBig);
        let faces = c.packs.iter().find(|p| p.id == "faces-opencv").unwrap();
        assert_eq!(fit(faces, &hw(4.0, 0.0)), Fit::Cpu);
    }

    #[test]
    fn link_checks_type_and_size_and_detects_changes() {
        let d = tempfile::tempdir().unwrap();
        let c = catalogue();
        let p = c.packs.iter().find(|p| p.id == "qwen3-vl-2b").unwrap();
        let mut l = Linked::default();

        // Wrong magic
        let bad = d.path().join("x.gguf");
        std::fs::write(&bad, b"nope").unwrap();
        assert!(matches!(l.link(p, "model", &bad, &|_, _| {}), Err(LinkError::WrongType(_))));

        // Right type, far too small for the 1.1 GB model
        let small = d.path().join("small.gguf");
        std::fs::write(&small, b"GGUF....").unwrap();
        assert!(matches!(l.link(p, "model", &small, &|_, _| {}), Err(LinkError::Size { .. })));

        // Plausible size (sparse file: no real disk use)
        let ok = d.path().join("model.gguf");
        let f = std::fs::File::create(&ok).unwrap();
        f.set_len(1100 * 1_048_576).unwrap();
        drop(f);
        let mut w = std::fs::OpenOptions::new().write(true).open(&ok).unwrap();
        std::io::Write::write_all(&mut w, b"GGUF").unwrap();
        drop(w);
        let lf = l.link(p, "model", &ok, &|_, _| {}).unwrap();
        assert!(!lf.verified); // plausible size, but not the checked bytes
        assert_eq!(l.status(p), PackStatus::Partial);

        let proj = d.path().join("mmproj.gguf");
        let f = std::fs::File::create(&proj).unwrap();
        f.set_len(800 * 1_048_576).unwrap();
        drop(f);
        let mut w = std::fs::OpenOptions::new().write(true).open(&proj).unwrap();
        std::io::Write::write_all(&mut w, b"GGUF").unwrap();
        drop(w);
        l.link(p, "projector", &proj, &|_, _| {}).unwrap();
        assert!(!l.verified(p));
        assert_eq!(l.status(p), PackStatus::Ready);

        // Save / load round trip
        let j = d.path().join("cfg").join("models.json");
        l.save(&j).unwrap();
        let l2 = Linked::load(&j).unwrap();
        assert_eq!(l2.status(p), PackStatus::Ready);

        // File removed -> Changed
        std::fs::remove_file(&proj).unwrap();
        assert_eq!(l2.status(p), PackStatus::Changed);
    }

    #[test]
    fn missing_models_json_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert!(Linked::load(&d.path().join("none.json")).unwrap().packs.is_empty());
    }

    /// Real hardware on the dev machine: never panics.
    #[test]
    fn detect_and_assess() {
        let a = assess(&Hardware::detect());
        assert!(!a.reason.is_empty());
    }
}
