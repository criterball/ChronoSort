//! `cs-platform`: the only crate in ChronoSort that is allowed to contain
//! OS-specific code. Everything else calls these portable functions.
//!
//! Guarantees:
//! * [`rename_no_replace`] never overwrites an existing file: it fails with
//!   [`std::io::ErrorKind::AlreadyExists`] instead.
//! * [`apply_times`] sets modified/accessed everywhere, and created where the OS
//!   allows it (Windows, macOS). [`CAN_SET_CREATED`] says which.

use std::fs::{File, FileTimes};
use std::io;
use std::path::Path;
use std::time::SystemTime;

mod child;
mod gpu;
mod memory;
mod recycle;
mod rename;
mod space;
mod volumes;

pub use child::{ChildGuard, end_with_us, prepare_child};
pub use gpu::{Gpu, gpus};
pub use memory::total_ram;
pub use recycle::{BinStatus, recycle_bin};
pub use rename::rename_no_replace;
pub use space::free_space;
pub use volumes::{KnownFolder, Volume, VolumeKind, known_folders, volumes};

/// Timestamps of a file as read from the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Times {
    pub modified: SystemTime,
    pub accessed: Option<SystemTime>,
    /// Birth/creation time. `None` when the filesystem does not record it.
    pub created: Option<SystemTime>,
}

/// Can this OS set a file's creation (birth) time?
/// Linux has no user-space API for it; the original value must then be
/// preserved elsewhere (journal + XMP sidecar).
pub const CAN_SET_CREATED: bool = cfg!(any(windows, target_os = "macos"));

/// Read the timestamps of `path` (follows symlinks).
pub fn read_times(path: &Path) -> io::Result<Times> {
    let md = std::fs::metadata(path)?;
    Ok(Times { modified: md.modified()?, accessed: md.accessed().ok(), created: md.created().ok() })
}

/// Which timestamps [`apply_times`] actually set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub modified: bool,
    pub accessed: bool,
    pub created: bool,
}

/// Apply `t` to an open file. The file must be opened with write access.
pub fn apply_times(file: &File, t: &Times) -> io::Result<Applied> {
    let mut ft = FileTimes::new().set_modified(t.modified);
    if let Some(a) = t.accessed {
        ft = ft.set_accessed(a);
    }
    let created = CAN_SET_CREATED && t.created.is_some();
    #[cfg(windows)]
    if let Some(c) = t.created {
        use std::os::windows::fs::FileTimesExt;
        ft = ft.set_created(c);
    }
    #[cfg(target_os = "macos")]
    if let Some(c) = t.created {
        use std::os::macos::fs::FileTimesExt;
        ft = ft.set_created(c);
    }
    file.set_times(ft)?;
    Ok(Applied { modified: true, accessed: t.accessed.is_some(), created })
}

/// Make a completed rename durable. On Unix the directory entry must be
/// fsync'ed; on Windows `MoveFileExW(MOVEFILE_WRITE_THROUGH)` already did it.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Show a folder in the system file manager (Explorer / Finder / Files).
pub fn open_in_file_manager(dir: &Path) -> io::Result<()> {
    if !dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "folder not found"));
    }
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(dir).spawn().map(|_| ())
}

/// Show a file in the system file manager with the file selected
/// (Explorer `/select`, Finder `-R`; other systems open the parent folder).
pub fn show_file_in_file_manager(file: &Path) -> io::Result<()> {
    if !file.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "file not found"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Windows paths cannot contain '"', so quoting is safe here.
        let arg = format!("/select,\"{}\"", file.display());
        return std::process::Command::new("explorer").raw_arg(arg).spawn().map(|_| ());
    }
    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("open").arg("-R").arg(file).spawn().map(|_| ());
    }
    #[allow(unreachable_code)]
    open_in_file_manager(file.parent().unwrap_or(file))
}

/// Facts about the running platform, shown in `chronosort info`.
#[derive(Debug, Clone)]
pub struct PlatformInfo {
    pub os: &'static str,
    pub arch: &'static str,
    pub can_set_created: bool,
}

pub fn platform_info() -> PlatformInfo {
    PlatformInfo { os: std::env::consts::OS, arch: std::env::consts::ARCH, can_set_created: CAN_SET_CREATED }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn times_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.bin");
        std::fs::write(&p, b"x").unwrap();
        let want = Times {
            modified: at(1_110_621_727), // 2005-03-12
            accessed: Some(at(1_200_000_000)),
            created: Some(at(1_000_000_000)), // 2001-09-09
        };
        let f = File::options().write(true).open(&p).unwrap();
        let applied = apply_times(&f, &want).unwrap();
        drop(f);
        let got = read_times(&p).unwrap();
        assert_eq!(got.modified, want.modified);
        assert_eq!(applied.created, CAN_SET_CREATED);
        if CAN_SET_CREATED {
            assert_eq!(got.created, want.created);
        }
    }

    #[test]
    fn pre_1970_mtime() {
        // Scanned family photos are sometimes stamped with very old dates.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("old.bin");
        std::fs::write(&p, b"x").unwrap();
        let old = SystemTime::UNIX_EPOCH - Duration::from_secs(10 * 365 * 86_400);
        let f = File::options().write(true).open(&p).unwrap();
        let t = Times { modified: old, accessed: None, created: None };
        apply_times(&f, &t).unwrap();
        drop(f);
        assert_eq!(read_times(&p).unwrap().modified, old);
    }
}
