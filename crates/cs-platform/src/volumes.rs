//! Drives / volumes and the user's standard folders, for the File browser's tree.
//!
//! Nothing here reads file contents. Empty card readers and optical drives
//! must not hang or pop up "insert a disk" dialogs, so volume details are only
//! asked for drives that report media, and Windows' critical-error dialogs are
//! switched off for the calling thread while we look.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Ram,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct Volume {
    /// Root to browse: `C:\`, `/`, `/media/anna/USB`, `/Volumes/Card`.
    pub path: PathBuf,
    /// Volume label ("Photos"), empty if none.
    pub label: String,
    pub kind: VolumeKind,
    /// `None` when there is no media (empty card reader) or it could not be read.
    pub total: Option<u64>,
    pub free: Option<u64>,
}

/// A standard folder of the current user (Pictures, Desktop …).
#[derive(Debug, Clone, Serialize)]
pub struct KnownFolder {
    /// Stable id: "pictures", "desktop", "downloads", "videos", "documents", "home".
    pub id: &'static str,
    pub path: PathBuf,
}

/// All volumes of this computer, in a natural order (C:, D: … or / first).
pub fn volumes() -> Vec<Volume> {
    imp::volumes()
}

/// The user's standard folders that exist on this computer.
pub fn known_folders() -> Vec<KnownFolder> {
    imp::known_folders().into_iter().filter(|k| k.path.is_dir()).collect()
}

fn home_dir() -> Option<PathBuf> {
    let v = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") };
    v.map(PathBuf::from).filter(|p| p.is_absolute())
}

#[allow(dead_code)]
fn sizes(p: &Path) -> (Option<u64>, Option<u64>) {
    match total_and_free(p) {
        Some((t, f)) => (Some(t), Some(f)),
        None => (None, None),
    }
}

#[cfg(unix)]
fn total_and_free(p: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(p.as_os_str().as_bytes()).ok()?;
    // SAFETY: zeroed statvfs is a valid out-buffer; `c` is NUL-terminated.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)]
    Some((st.f_blocks as u64 * st.f_frsize as u64, st.f_bavail as u64 * st.f_frsize as u64))
}

#[cfg(windows)]
fn total_and_free(p: &Path) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let w = crate::space::win_path(p).ok()?;
    let (mut avail, mut total) = (0u64, 0u64);
    // SAFETY: `w` is NUL-terminated; out-pointers are valid; the last may be null.
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some((total, avail))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX, SetThreadErrorMode,
    };

    // GetDriveTypeW results (winbase.h).
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;

    /// Switches off "There is no disk in the drive" dialogs for this thread
    /// and restores the old mode when dropped.
    struct QuietErrors(u32);
    impl QuietErrors {
        fn new() -> Self {
            let mut old = 0u32;
            // SAFETY: plain call with a valid out-pointer.
            unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX, &mut old) };
            Self(old)
        }
    }
    impl Drop for QuietErrors {
        fn drop(&mut self) {
            // SAFETY: restoring the previous mode.
            unsafe { SetThreadErrorMode(self.0, std::ptr::null_mut()) };
        }
    }

    pub fn volumes() -> Vec<Volume> {
        let _quiet = QuietErrors::new();
        // SAFETY: no arguments.
        let mask = unsafe { GetLogicalDrives() };
        let mut out = Vec::new();
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let root = format!("{}:\\", char::from(b'A' + i as u8));
            let w: Vec<u16> = std::ffi::OsStr::new(&root).encode_wide().chain([0]).collect();
            // SAFETY: `w` is a NUL-terminated root path.
            let kind = match unsafe { GetDriveTypeW(w.as_ptr()) } {
                DRIVE_FIXED => VolumeKind::Fixed,
                DRIVE_REMOVABLE => VolumeKind::Removable,
                DRIVE_REMOTE => VolumeKind::Network,
                DRIVE_CDROM => VolumeKind::Optical,
                DRIVE_RAMDISK => VolumeKind::Ram,
                _ => VolumeKind::Other,
            };
            let mut name = [0u16; 261];
            // SAFETY: buffers are valid for the lengths given; unused outputs are null.
            let has_media = unsafe {
                GetVolumeInformationW(
                    w.as_ptr(),
                    name.as_mut_ptr(),
                    name.len() as u32,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            } != 0;
            let label = if has_media {
                let n = name.iter().position(|&c| c == 0).unwrap_or(0);
                OsString::from_wide(&name[..n]).to_string_lossy().into_owned()
            } else {
                String::new()
            };
            let path = PathBuf::from(&root);
            let (total, free) = if has_media { sizes(&path) } else { (None, None) };
            out.push(Volume { path, label, kind, total, free });
        }
        out
    }

    pub fn known_folders() -> Vec<KnownFolder> {
        use windows_sys::Win32::System::Com::CoTaskMemFree;
        use windows_sys::Win32::UI::Shell::{
            FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Pictures, FOLDERID_Videos,
            SHGetKnownFolderPath,
        };
        let mut out = Vec::new();
        // Pictures first: that is where most people's photos are.
        for (id, guid) in [
            ("pictures", &FOLDERID_Pictures),
            ("desktop", &FOLDERID_Desktop),
            ("downloads", &FOLDERID_Downloads),
            ("documents", &FOLDERID_Documents),
            ("videos", &FOLDERID_Videos),
        ] {
            let mut p: windows_sys::core::PWSTR = std::ptr::null_mut();
            // SAFETY: valid GUID pointer and out-pointer; the buffer is freed below.
            let hr = unsafe { SHGetKnownFolderPath(guid, 0, std::ptr::null_mut(), &mut p) };
            if hr == 0 && !p.is_null() {
                // SAFETY: SHGetKnownFolderPath returns a NUL-terminated string.
                let n = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
                let s = OsString::from_wide(unsafe { std::slice::from_raw_parts(p, n) });
                out.push(KnownFolder { id, path: PathBuf::from(s) });
            }
            if !p.is_null() {
                // SAFETY: memory allocated by the shell with CoTaskMemAlloc.
                unsafe { CoTaskMemFree(p as *const _) };
            }
        }
        if let Some(h) = home_dir() {
            out.push(KnownFolder { id: "home", path: h });
        }
        out
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;

    /// Undo the octal escapes in /proc/mounts (`\040` = space).
    pub(super) fn unescape(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
                out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
                i += 4;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Mount points a person would recognise: the root, /home, and anything
    /// under /media, /run/media or /mnt (USB sticks, cards, other disks).
    pub(super) fn wanted(mnt: &str, fstype: &str) -> Option<VolumeKind> {
        const PSEUDO: [&str; 14] = [
            "proc",
            "sysfs",
            "tmpfs",
            "devtmpfs",
            "devpts",
            "cgroup",
            "cgroup2",
            "overlay",
            "squashfs",
            "securityfs",
            "debugfs",
            "tracefs",
            "autofs",
            "fuse.portal",
        ];
        if PSEUDO.contains(&fstype) {
            return None;
        }
        let net = matches!(fstype, "nfs" | "nfs4" | "cifs" | "smb3" | "sshfs" | "fuse.sshfs");
        let kind = if fstype == "iso9660" || fstype == "udf" {
            VolumeKind::Optical
        } else if net {
            VolumeKind::Network
        } else if mnt.starts_with("/media/") || mnt.starts_with("/run/media/") {
            VolumeKind::Removable
        } else {
            VolumeKind::Fixed
        };
        let ok = mnt == "/"
            || mnt == "/home"
            || mnt.starts_with("/media/")
            || mnt.starts_with("/run/media/")
            || mnt.starts_with("/mnt/");
        ok.then_some(kind)
    }

    pub fn volumes() -> Vec<Volume> {
        let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let mut out: Vec<Volume> = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 3 {
                continue;
            }
            let mnt = unescape(f[1]);
            let Some(kind) = wanted(&mnt, f[2]) else { continue };
            if out.iter().any(|v| v.path == Path::new(&mnt)) {
                continue;
            }
            let path = PathBuf::from(&mnt);
            let label = if mnt == "/" {
                String::new()
            } else {
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            };
            let (total, free) = sizes(&path);
            out.push(Volume { path, label, kind, total, free });
        }
        out.sort_by_key(|v| (v.path != Path::new("/"), v.path.clone()));
        out
    }

    /// XDG user folders (`~/.config/user-dirs.dirs`), falling back to English names.
    pub fn known_folders() -> Vec<KnownFolder> {
        let Some(home) = home_dir() else { return Vec::new() };
        let cfg = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).unwrap_or_default();
        let xdg = |key: &str, default: &str| -> PathBuf {
            for line in cfg.lines() {
                if let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
                    let v = v.trim().trim_matches('"');
                    let v = v.replacen("$HOME", &home.to_string_lossy(), 1);
                    return PathBuf::from(v);
                }
            }
            home.join(default)
        };
        vec![
            KnownFolder { id: "pictures", path: xdg("XDG_PICTURES_DIR", "Pictures") },
            KnownFolder { id: "desktop", path: xdg("XDG_DESKTOP_DIR", "Desktop") },
            KnownFolder { id: "downloads", path: xdg("XDG_DOWNLOAD_DIR", "Downloads") },
            KnownFolder { id: "documents", path: xdg("XDG_DOCUMENTS_DIR", "Documents") },
            KnownFolder { id: "videos", path: xdg("XDG_VIDEOS_DIR", "Videos") },
            KnownFolder { id: "home", path: home.clone() },
        ]
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    /// The start-up disk ("/") plus everything in /Volumes except the
    /// symlink that points back to "/".
    pub fn volumes() -> Vec<Volume> {
        let (total, free) = sizes(Path::new("/"));
        let mut out =
            vec![Volume { path: PathBuf::from("/"), label: String::new(), kind: VolumeKind::Fixed, total, free }];
        if let Ok(rd) = std::fs::read_dir("/Volumes") {
            let mut more: Vec<Volume> = rd
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)) // symlinks report as symlink, not dir
                .map(|e| {
                    let path = e.path();
                    let (total, free) = sizes(&path);
                    Volume {
                        label: e.file_name().to_string_lossy().into_owned(),
                        path,
                        kind: VolumeKind::Removable,
                        total,
                        free,
                    }
                })
                .collect();
            more.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
            out.extend(more);
        }
        out
    }

    pub fn known_folders() -> Vec<KnownFolder> {
        let Some(home) = home_dir() else { return Vec::new() };
        vec![
            KnownFolder { id: "pictures", path: home.join("Pictures") },
            KnownFolder { id: "desktop", path: home.join("Desktop") },
            KnownFolder { id: "downloads", path: home.join("Downloads") },
            KnownFolder { id: "documents", path: home.join("Documents") },
            KnownFolder { id: "videos", path: home.join("Movies") },
            KnownFolder { id: "home", path: home.clone() },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_at_least_one_volume() {
        let v = volumes();
        assert!(!v.is_empty(), "no volumes found");
        // The volume holding the temp dir has media and a size.
        let tmp = std::env::temp_dir();
        let holder = v
            .iter()
            .filter(|x| tmp.starts_with(&x.path))
            .max_by_key(|x| x.path.as_os_str().len())
            .expect("temp volume listed");
        assert!(holder.total.unwrap_or(0) > 0);
    }

    #[test]
    fn known_folders_exist() {
        for k in known_folders() {
            assert!(k.path.is_dir(), "{} -> {:?}", k.id, k.path);
        }
        assert!(known_folders().iter().any(|k| k.id == "home"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_mount_rules() {
        assert_eq!(imp::unescape(r"/media/anna/MY\040USB"), "/media/anna/MY USB");
        assert_eq!(imp::wanted("/", "ext4"), Some(VolumeKind::Fixed));
        assert_eq!(imp::wanted("/media/anna/CARD", "vfat"), Some(VolumeKind::Removable));
        assert_eq!(imp::wanted("/proc", "proc"), None);
        assert_eq!(imp::wanted("/boot/efi", "vfat"), None);
    }
}
