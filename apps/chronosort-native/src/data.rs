//! Where ChronoSort keeps its own files: ALWAYS `ChronoSortData` beside the program.
//!
//! The rule (user, 2026-11-27): ChronoSort is always portable. Settings, people, AI parts, the
//! thumbnail cache — everything lives inside the ChronoSort folder, nothing in AppData. If that
//! folder cannot be written (a locked stick, Program Files), the app says so instead of quietly
//! falling back to the PC's own folders.

use std::path::PathBuf;

pub const DATA_DIR: &str = "ChronoSortData";

pub struct DataDirs {
    pub root: PathBuf,
    pub cache: PathBuf,
}

/// The data folder, created and proven writable; `Err` is a message for the user.
pub fn data_dirs() -> Result<DataDirs, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find where ChronoSort is running from: {e}"))?;
    let dir = exe.parent().ok_or("Cannot find the ChronoSort folder.")?;
    let root = dir.join(DATA_DIR);
    let cache = root.join("cache");
    let not_writable = |e: std::io::Error| {
        format!(
            "ChronoSort keeps all its files in its own folder, but it cannot write to\n{}\n({e}).\n\n\
             Copy the ChronoSort folder to a place you can write to (for example a USB stick, \
             Documents or D:\\) and start it from there.",
            root.display()
        )
    };
    std::fs::create_dir_all(&cache).map_err(not_writable)?;
    let probe = root.join(format!(".write_test_{}", std::process::id()));
    std::fs::write(&probe, b"ok").map_err(not_writable)?;
    let _ = std::fs::remove_file(&probe);
    Ok(DataDirs { root, cache })
}
