//! Rename that refuses to replace an existing destination.
//!
//! `std::fs::rename` silently overwrites on every platform, so it must never
//! be used for placing files into the sorted tree.

use std::io;
use std::path::Path;

/// Atomically rename `from` to `to`, failing with `ErrorKind::AlreadyExists`
/// if `to` already exists. Both paths should be on the same volume.
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    imp::rename_no_replace(from, to)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::space::win_path;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let f = win_path(from)?;
        let t = win_path(to)?;
        // No MOVEFILE_REPLACE_EXISTING => fails with ERROR_ALREADY_EXISTS (183),
        // which std maps to ErrorKind::AlreadyExists. No MOVEFILE_COPY_ALLOWED
        // => never silently turns into a cross-volume copy.
        // SAFETY: both buffers are NUL-terminated UTF-16 that outlive the call.
        let ok = unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let f = CString::new(from.as_os_str().as_bytes())?;
        let t = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: valid NUL-terminated C strings for the duration of the call.
        let r =
            unsafe { libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE) };
        if r == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            // Filesystem or kernel without RENAME_NOREPLACE support.
            Some(libc::EINVAL) | Some(libc::ENOSYS) => super::link_fallback(from, to),
            _ => Err(e),
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let f = CString::new(from.as_os_str().as_bytes())?;
        let t = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: valid NUL-terminated C strings for the duration of the call.
        let r = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
        if r == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::ENOTSUP) | Some(libc::EINVAL) => super::link_fallback(from, to),
            _ => Err(e),
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    use super::*;
    pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        super::link_fallback(from, to)
    }
}

/// `link()` fails if the target exists, so link + unlink is a no-replace
/// rename (not atomic: for a moment both names exist, which is harmless).
#[cfg(not(windows))]
#[allow(dead_code)]
fn link_fallback(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_when_free() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.part");
        let b = d.path().join("b.jpg");
        std::fs::write(&a, b"new").unwrap();
        rename_no_replace(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(&b).unwrap(), b"new");
    }

    #[test]
    fn never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.part");
        let b = d.path().join("b.jpg");
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&b, b"precious").unwrap();
        let e = rename_no_replace(&a, &b).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&b).unwrap(), b"precious");
        assert_eq!(std::fs::read(&a).unwrap(), b"new");
    }

    #[test]
    fn long_paths() {
        let d = tempfile::tempdir().unwrap();
        let mut deep = d.path().to_path_buf();
        for i in 0..8 {
            deep.push(format!("{i}_{}", "x".repeat(40)));
        }
        std::fs::create_dir_all(&deep).unwrap();
        assert!(deep.as_os_str().len() > 300);
        let a = deep.join("a.part");
        let b = deep.join("final_name_that_is_long.jpg");
        std::fs::write(&a, b"x").unwrap();
        rename_no_replace(&a, &b).unwrap();
        assert!(b.exists());
    }
}
