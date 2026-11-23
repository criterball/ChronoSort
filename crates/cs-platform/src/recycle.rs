//! Does deleting on this drive go to the Recycle Bin / Trash?
//!
//! This matters for safety: on Windows, "send to the Recycle Bin" silently
//! deletes for good when the bin is switched off for that drive, and USB
//! sticks, memory cards and network drives have no Recycle Bin at all. The
//! File browser asks here first and only offers "Delete permanently" (with a
//! clear warning) when there is no bin.

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BinStatus {
    /// Deleted files on this drive can be restored from the Recycle Bin / Trash.
    pub available: bool,
    /// Plain-language reason when it is not available.
    pub reason: Option<String>,
}

impl BinStatus {
    fn yes() -> Self {
        Self { available: true, reason: None }
    }
    fn no(r: &str) -> Self {
        Self { available: false, reason: Some(r.to_string()) }
    }
}

/// Recycle Bin status for the drive holding `path`.
pub fn recycle_bin(path: &Path) -> BinStatus {
    imp::recycle_bin(path)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::space::win_path;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW,
    };

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn reg_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
        let (k, v) = (wide(key), wide(value));
        let mut data: u32 = 0;
        let mut len: u32 = 4;
        // SAFETY: key/value are NUL-terminated; data/len point to a valid u32.
        let r = unsafe {
            RegGetValueW(
                root,
                k.as_ptr(),
                v.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut data as *mut u32).cast(),
                &mut len,
            )
        };
        (r == 0).then_some(data)
    }

    /// Mount point of the volume holding `p` ("C:\", "D:\Mount\Disk\") as a NUL-terminated wide string.
    fn volume_root(p: &Path) -> Option<Vec<u16>> {
        let mut q = std::path::absolute(p).ok()?;
        while !q.exists() {
            q = q.parent()?.to_path_buf();
        }
        let w = win_path(&q).ok()?;
        let mut root = [0u16; 1024];
        // SAFETY: `w` is NUL-terminated; buffer length is given.
        if unsafe { GetVolumePathNameW(w.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return None;
        }
        let n = root.iter().position(|&c| c == 0)?;
        let mut v = root[..n].to_vec();
        // Strip the \\?\ prefix again for GetDriveTypeW / the mount-point API.
        let pre: Vec<u16> = r"\\?\".encode_utf16().collect();
        if v.starts_with(&pre) && !v[pre.len()..].starts_with(&"UNC".encode_utf16().collect::<Vec<_>>()) {
            v.drain(..pre.len());
        }
        if v.last() != Some(&u16::from(b'\\')) {
            v.push(u16::from(b'\\'));
        }
        v.push(0);
        Some(v)
    }

    pub fn recycle_bin(path: &Path) -> BinStatus {
        const POL: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";
        if reg_dword(HKEY_CURRENT_USER, POL, "NoRecycleFiles") == Some(1)
            || reg_dword(HKEY_LOCAL_MACHINE, POL, "NoRecycleFiles") == Some(1)
        {
            return BinStatus::no("The Recycle Bin is switched off on this PC (by a policy).");
        }
        let s = path.to_string_lossy();
        if s.starts_with(r"\\") && !s.starts_with(r"\\?\") {
            return BinStatus::no("Network folders have no Recycle Bin.");
        }
        let Some(root) = volume_root(path) else {
            return BinStatus::no("Could not check this drive's Recycle Bin.");
        };
        // SAFETY: NUL-terminated root path.
        match unsafe { GetDriveTypeW(root.as_ptr()) } {
            DRIVE_REMOVABLE => return BinStatus::no("USB sticks and memory cards have no Recycle Bin."),
            DRIVE_REMOTE => return BinStatus::no("Network drives have no Recycle Bin."),
            DRIVE_CDROM => return BinStatus::no("This is a CD/DVD drive."),
            DRIVE_RAMDISK => return BinStatus::no("RAM disks have no Recycle Bin."),
            _ => {}
        }
        // Per-drive setting "Don't move files to the Recycle Bin" (Recycle Bin properties).
        let mut guid = [0u16; 64];
        // SAFETY: NUL-terminated mount point; buffer length is given.
        if unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), guid.as_mut_ptr(), guid.len() as u32) } != 0 {
            let n = guid.iter().position(|&c| c == 0).unwrap_or(0);
            let name = String::from_utf16_lossy(&guid[..n]); // \\?\Volume{GUID}\
            if let (Some(a), Some(b)) = (name.find('{'), name.find('}')) {
                let key =
                    format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{}", &name[a..=b]);
                if reg_dword(HKEY_CURRENT_USER, &key, "NukeOnDelete") == Some(1) {
                    return BinStatus::no("The Recycle Bin is switched off for this drive (Recycle Bin properties).");
                }
            }
        }
        BinStatus::yes()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    /// Linux (freedesktop Trash, also on USB drives via .Trash-UID) and macOS
    /// (~/.Trash and per-volume .Trashes) both have a trash on normal drives.
    pub fn recycle_bin(path: &Path) -> BinStatus {
        let s = path.to_string_lossy();
        if s.starts_with("/proc") || s.starts_with("/sys") || s.starts_with("/dev") {
            return BinStatus::no("This is a system area without a Trash.");
        }
        BinStatus::yes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_for_temp_dir() {
        let b = recycle_bin(&std::env::temp_dir());
        // Either answer is fine; a "no" must say why.
        assert!(b.available || b.reason.is_some());
    }

    #[test]
    fn missing_path_uses_existing_parent() {
        let b = recycle_bin(&std::env::temp_dir().join("not/there/yet.jpg"));
        assert!(b.available || b.reason.as_deref() != Some("Could not check this drive's Recycle Bin."));
    }
}

#[cfg(test)]
mod probe {
    /// `cargo test -p cs-platform print_bins -- --ignored --nocapture` shows the answer for every drive.
    #[test]
    #[ignore]
    fn print_bins() {
        for v in crate::volumes() {
            println!("{} {:?} -> {:?}", v.path.display(), v.kind, super::recycle_bin(&v.path));
        }
    }
}
