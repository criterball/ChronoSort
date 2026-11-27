//! Keep the screen on (no sleep, no screen saver) while a slideshow plays.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);
static THREAD: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// Ask the OS to keep the display awake (`true`) or let it sleep again (`false`).
/// Safe to call repeatedly. On Windows a small helper thread tells the system
/// every 20 s that the display is in use; elsewhere it does nothing yet.
pub fn keep_awake(on: bool) {
    ON.store(on, Ordering::SeqCst);
    let mut t = THREAD.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        if t.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        *t = Some(std::thread::spawn(|| {
            while ON.load(Ordering::SeqCst) {
                imp::nudge();
                // Short sleeps so turning it off takes effect quickly.
                for _ in 0..40 {
                    if !ON.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
            imp::release();
        }));
    } else if let Some(h) = t.take() {
        let _ = h.join();
    }
}

/// True while [`keep_awake`] is on.
pub fn is_awake() -> bool {
    ON.load(Ordering::SeqCst)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };
    pub fn nudge() {
        // SAFETY: plain flags, no pointers.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED) };
    }
    pub fn release() {
        // SAFETY: as above; clears this thread's request.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn nudge() {}
    pub fn release() {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn on_and_off_again() {
        super::keep_awake(true);
        super::keep_awake(true);
        assert!(super::is_awake());
        super::keep_awake(false);
        assert!(!super::is_awake());
        super::keep_awake(false);
    }
}
