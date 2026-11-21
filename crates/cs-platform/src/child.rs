//! Tie a helper process (the AI engine) to ChronoSort's life: if ChronoSort
//! ends in any way, even a crash, the helper ends too and frees its memory.
//!
//! Windows: a job object with KILL_ON_JOB_CLOSE; the OS closes our handle when
//! we exit, which ends every process in the job. Linux: the child asks the
//! kernel for SIGKILL when its parent dies (`prepare_child`, PR_SET_PDEATHSIG).
//! macOS has no equivalent; there the engine is stopped on a normal exit.

/// Call on the command before spawning it (Linux: parent-death signal).
#[cfg(target_os = "linux")]
pub fn prepare_child(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: only async-signal-safe calls (prctl, getppid, kill) between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may already be gone (race between fork and prctl).
            if libc::getppid() == 1 {
                libc::kill(libc::getpid(), libc::SIGKILL);
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub fn prepare_child(_cmd: &mut std::process::Command) {}

/// Keep this alive as long as the child should run. Dropping it ends the child.
pub struct ChildGuard {
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

// The handle is only used to close the job; that is safe from any thread.
unsafe impl Send for ChildGuard {}
unsafe impl Sync for ChildGuard {}

#[cfg(windows)]
pub fn end_with_us(child: &std::process::Child) -> std::io::Result<ChildGuard> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // SAFETY: plain Win32 calls with valid arguments; every failure path closes the job handle.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const info).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0
            && AssignProcessToJobObject(job, child.as_raw_handle()) != 0;
        if !ok {
            let e = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(e);
        }
        Ok(ChildGuard { job })
    }
}

#[cfg(not(windows))]
pub fn end_with_us(_child: &std::process::Child) -> std::io::Result<ChildGuard> {
    Ok(ChildGuard {})
}

#[cfg(windows)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed exactly once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.job) };
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn dropping_the_guard_ends_the_child() {
        let mut child = std::process::Command::new("cmd").args(["/C", "ping -n 30 127.0.0.1 >NUL"]).spawn().unwrap();
        let g = super::end_with_us(&child).unwrap();
        assert!(child.try_wait().unwrap().is_none(), "still running");
        drop(g);
        let t = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(t.elapsed().as_secs() < 5, "the child should have been ended");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
