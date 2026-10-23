use super::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsIconic, IsWindow, PostMessageW, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindowAsync,
    WM_CLOSE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Open,
    Settings,
    Exit,
}

#[derive(Clone, Copy)]
pub(super) struct Window(isize);
impl Window {
    pub fn from_context(cc: &eframe::CreationContext<'_>) -> Result<Self> {
        match cc.window_handle()?.as_raw() {
            RawWindowHandle::Win32(handle) => Ok(Self(handle.hwnd.get())),
            _ => anyhow::bail!("The desktop requires a Windows window handle"),
        }
    }
    fn handle(self) -> windows_sys::Win32::Foundation::HWND {
        self.0 as _
    }
    pub fn restore(self) {
        // A hidden/minimized renderer need not produce another egui frame.
        // Restore the actual window from the tray event instead of relying
        // on viewport commands being consumed by App::update first.
        unsafe {
            let hwnd = self.handle();
            if IsWindow(hwnd) != 0 {
                ShowWindowAsync(
                    hwnd,
                    if IsIconic(hwnd) != 0 {
                        SW_RESTORE
                    } else {
                        SW_SHOW
                    },
                );
                SetForegroundWindow(hwnd);
            }
        }
    }
    pub fn close(self) {
        unsafe {
            PostMessageW(self.handle(), WM_CLOSE, 0, 0);
        }
    }
}

#[derive(Clone)]
pub(super) struct Control {
    pub window: Window,
    pub queue: Arc<Mutex<Vec<Action>>>,
    pub context: egui::Context,
}
impl Control {
    pub fn request(&self, action: Action) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(action);
        // Exit also wakes the hidden window so its final frame can cleanly
        // close eframe and drop the tray icon. The Windows service is separate.
        self.window.restore();
        self.context.request_repaint();
    }
}

pub(super) fn opens(event: &TrayIconEvent) -> bool {
    matches!(
        event,
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } | TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
}

// Exercise real window visibility and the production action path, without
// changing service configuration or synthesizing timing evidence.
pub(super) fn test_lifecycle(control: Control, destination: PathBuf) {
    std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindowVisible, SW_MINIMIZE};
            let wait = |condition: &dyn Fn() -> bool| -> Result<()> {
                let deadline = Instant::now() + Duration::from_secs(8);
                while Instant::now() < deadline {
                    if condition() {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                anyhow::bail!("Tray lifecycle condition timed out")
            };
            let visible = || unsafe {
                IsWindowVisible(control.window.handle()) != 0
                    && IsIconic(control.window.handle()) == 0
            };
            let hidden = || unsafe { IsWindowVisible(control.window.handle()) == 0 };
            wait(&visible)?;
            for action in [Action::Open, Action::Settings] {
                control.window.close(); // Same WM_CLOSE as the title-bar X.
                wait(&hidden)?;
                control.request(action);
                wait(&visible)?;
                wait(&|| {
                    control
                        .queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty()
                })?;
            }
            unsafe {
                ShowWindowAsync(control.window.handle(), SW_MINIMIZE);
            }
            wait(&|| unsafe {
                IsWindowVisible(control.window.handle()) != 0
                    && IsIconic(control.window.handle()) != 0
            })?;
            control.request(Action::Open);
            wait(&visible)?;
            wait(&|| {
                control
                    .queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_empty()
            })?;
            control.window.close();
            wait(&hidden)?;
            Ok(())
        })();
        let report = match &result {
            Ok(()) => {
                serde_json::json!({"passed":true,"checks":["X hides","Open restores after X","Settings restores after X","Minimize keeps taskbar window","Open restores after minimize","Exit requested while hidden"]})
            }
            Err(e) => serde_json::json!({"passed":false,"error":format!("{e:#}")}),
        };
        if let Err(e) = std::fs::write(&destination, serde_json::to_vec_pretty(&report).unwrap()) {
            eprintln!("Tray test report could not be written: {e}");
        }
        control.request(Action::Exit);
    });
}
