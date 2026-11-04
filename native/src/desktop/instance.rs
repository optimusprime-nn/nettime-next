//! One desktop per Windows session. Launching it again surfaces the running
//! window, including one hidden in the notification area, instead of opening
//! another copy.
use super::*;
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForSingleObject},
    UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow},
};

/// Session-local, so each signed-in user gets their own desktop.
pub(super) const NAME: &str = "Local\\NetTimeNextDesktopOpen";

/// The running desktop's wake event. Holding it keeps the name claimed.
pub(super) struct Wake(isize);

/// Claims the desktop for this session. If another copy already holds it,
/// asks that copy to show its window (unless `show` is false, as for the
/// sign-in tray start) and returns None.
pub(super) fn claim(name: &str, show: bool) -> Result<Option<Wake>> {
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // An auto-reset event: creating it is atomic, so of two copies started
    // together exactly one finds it new.
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, wide.as_ptr()) };
    anyhow::ensure!(
        !event.is_null(),
        "Desktop instance check failed: {}",
        std::io::Error::last_os_error()
    );
    if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return Ok(Some(Wake(event as isize)));
    }
    if show {
        unsafe {
            // This launch came from the user, so it may hand the running copy
            // permission to come to the foreground.
            AllowSetForegroundWindow(ASFW_ANY);
            SetEvent(event);
        }
    }
    unsafe { CloseHandle(event) };
    Ok(None)
}

impl Wake {
    /// Runs `open` each time another launch asks for the window.
    pub fn listen(self, open: impl Fn() + Send + 'static) {
        std::thread::spawn(move || {
            while unsafe { WaitForSingleObject(self.0 as _, INFINITE) } == WAIT_OBJECT_0 {
                open();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn second_launch_wakes_the_first_instead_of_running() {
        let name = format!("Local\\NetTimeNextDesktopTest-{}", std::process::id());
        let first = claim(&name, true).unwrap().expect("first launch runs");
        let (opened, woken) = mpsc::channel();
        first.listen(move || opened.send(()).unwrap());

        assert!(claim(&name, true).unwrap().is_none());
        woken
            .recv_timeout(Duration::from_secs(2))
            .expect("window asked to open");

        // A sign-in tray start must not pop the window up.
        assert!(claim(&name, false).unwrap().is_none());
        assert!(woken.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
