//! Free disk space, plus the Windows long-path helper.

use std::io;
use std::path::Path;

/// Bytes available to the current user on the volume holding `path`.
/// `path` may not exist yet; the nearest existing ancestor is used.
pub fn free_space(path: &Path) -> io::Result<u64> {
    let abs = std::path::absolute(path)?;
    let mut p = abs.as_path();
    while !p.exists() {
        p = p.parent().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no existing ancestor"))?;
    }
    imp::free_space(p)
}

/// Absolute, NUL-terminated UTF-16 path with the `\\?\` prefix so that Win32
/// calls are not limited to 260 characters.
#[cfg(windows)]
pub(crate) fn win_path(p: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    // `absolute` uses GetFullPathNameW: resolves `..`, turns `/` into `\`.
    let abs = std::path::absolute(p)?;
    let s: Vec<u16> = abs.as_os_str().encode_wide().collect();
    if s.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"));
    }
    let bs = u16::from(b'\\');
    let mut out: Vec<u16> = Vec::with_capacity(s.len() + 9);
    if s.starts_with(&[bs, bs, u16::from(b'?'), bs]) || s.starts_with(&[bs, bs, u16::from(b'.'), bs]) {
        out.extend_from_slice(&s); // already verbatim / device path
    } else if s.starts_with(&[bs, bs]) {
        out.extend("\\\\?\\UNC\\".encode_utf16()); // \\server\share -> \\?\UNC\server\share
        out.extend_from_slice(&s[2..]);
    } else {
        out.extend("\\\\?\\".encode_utf16());
        out.extend_from_slice(&s);
    }
    out.push(0);
    Ok(out)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    pub fn free_space(p: &Path) -> io::Result<u64> {
        let w = win_path(p)?;
        let mut avail: u64 = 0;
        // SAFETY: `w` is NUL-terminated; out-pointer is valid; others may be null.
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(avail) }
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    pub fn free_space(p: &Path) -> io::Result<u64> {
        let c = CString::new(p.as_os_str().as_bytes())?;
        // SAFETY: zeroed statvfs is a valid out-buffer; `c` is NUL-terminated.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        #[allow(clippy::unnecessary_cast)]
        Ok(st.f_bavail as u64 * st.f_frsize as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_free_space() {
        let d = tempfile::tempdir().unwrap();
        assert!(free_space(d.path()).unwrap() > 0);
        // Non-existent child falls back to the existing parent.
        assert!(free_space(&d.path().join("not/yet/made")).unwrap() > 0);
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_prefixes() {
        let s = |p: &str| String::from_utf16(&win_path(Path::new(p)).unwrap()).unwrap();
        assert_eq!(s(r"C:\a\..\b"), "\\\\?\\C:\\b\0");
        assert_eq!(s(r"\\server\share\x"), "\\\\?\\UNC\\server\\share\\x\0");
        assert_eq!(s(r"\\?\C:\x"), "\\\\?\\C:\\x\0");
    }
}
