//! Installed memory (RAM), used to suggest AI models that fit this PC.

/// Total physical memory in bytes, if the OS reports it.
pub fn total_ram() -> Option<u64> {
    imp::total_ram()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    pub fn total_ram() -> Option<u64> {
        // SAFETY: zeroed struct with dwLength set is the documented way to call this.
        let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        let ok = unsafe { GlobalMemoryStatusEx(&mut m) };
        (ok != 0).then_some(m.ullTotalPhys)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn total_ram() -> Option<u64> {
        // SAFETY: sysconf has no preconditions.
        let (pages, size) = unsafe { (libc::sysconf(libc::_SC_PHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
        (pages > 0 && size > 0).then(|| pages as u64 * size as u64)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn total_ram() -> Option<u64> {
        let mut v: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: valid NUL-terminated name and correctly sized out-buffer.
        let r = unsafe {
            libc::sysctlbyname(c"hw.memsize".as_ptr(), (&mut v as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0)
        };
        (r == 0 && v > 0).then_some(v)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    pub fn total_ram() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_plausible_ram() {
        let r = super::total_ram().expect("RAM size");
        assert!(r >= 512 << 20, "at least 512 MB, got {r}");
    }
}
