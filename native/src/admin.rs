//! Administrative commands live in the helper; desktop requests UAC and waits off-thread.
use crate::{
    config::Config,
    ownership::{self, WindowsServices},
    service,
    windows::{Handle, atomic_write, check, require_admin, wide},
};
use anyhow::{Result, ensure};
use std::path::Path;
use windows_sys::Win32::{
    Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0},
    System::Threading::{CreateMutexW, GetExitCodeProcess, ReleaseMutex, WaitForSingleObject},
    UI::{
        Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_HIDE,
    },
};

struct Lock(Handle);
impl Lock {
    fn new() -> Result<Self> {
        let handle = unsafe {
            CreateMutexW(
                std::ptr::null(),
                0,
                wide("Global\\NetTimeNextNativeAdministration").as_ptr(),
            )
        };
        ensure!(
            !handle.is_null(),
            "Cannot lock native service administration"
        );
        let handle = Handle(handle);
        let result = unsafe { WaitForSingleObject(handle.0, 0) };
        ensure!(
            [WAIT_OBJECT_0, WAIT_ABANDONED].contains(&result),
            "Another service administration action is in progress"
        );
        Ok(Self(handle))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0.0);
        }
    }
}
pub fn dispatch(args: &[String]) -> Result<bool> {
    let Some(command) = args.first().map(String::as_str).filter(|s| {
        [
            "--apply-config",
            "--takeover",
            "--rollback",
            "--import-settings",
            "--install-service",
            "--start-service",
            "--stop-service",
            "--remove-service",
            "--prepare-upgrade",
            "--finish-upgrade",
        ]
        .contains(s)
    }) else {
        return Ok(false);
    };
    require_admin()?;
    let _lock = Lock::new()?;
    if !["--prepare-upgrade", "--finish-upgrade", "--rollback"].contains(&command) {
        ensure!(
            !crate::upgrade::pending()?,
            "A service upgrade is in progress; finish it before changing settings"
        );
        crate::settings_update::recover(&service::root(), &mut WindowsServices)?;
    }
    let result = match command {
        "--takeover" => ownership::activate(&service::root(), &mut WindowsServices),
        "--rollback" => ownership::rollback(&service::root(), &mut WindowsServices),
        "--import-settings" => {
            ensure!(
                !ownership::active(&service::root())?,
                "Restore previous clocks before importing another service's settings"
            );
            apply_config(&crate::config::machine_root().join("settings.json"))
        }
        "--apply-config" => {
            ensure!(args.len() == 2, "Supply one settings request file");
            apply_config(Path::new(&args[1]))
        }
        "--install-service" => service::install(),
        "--start-service" => service::action("start").map(|_| ()),
        "--stop-service" => service::action("stop").map(|_| ()),
        "--remove-service" => service::action("remove").map(|_| ()),
        "--prepare-upgrade" => crate::upgrade::prepare(),
        "--finish-upgrade" => crate::upgrade::finish(),
        _ => unreachable!(),
    };
    let message = match &result {
        Ok(()) => format!("{command} completed"),
        Err(e) => format!("{command}: {e:#}"),
    };
    let _ = atomic_write(
        &service::root().join("admin-result.json"),
        &serde_json::to_vec(
            &serde_json::json!({"utc":crate::protocol::utc(),"command":command,"success":result.is_ok(),"message":message}),
        )?,
    );
    result?;
    Ok(true)
}
pub fn apply_config(source: &Path) -> Result<()> {
    ensure!(
        std::fs::metadata(source)?.len() <= 1_048_576,
        "Settings file exceeds 1 MB"
    );
    let request = Config::load(source)?;
    crate::settings_update::apply(&service::root(), source, &mut WindowsServices, |server| {
        crate::settings_update::probe(server, request.timeout_seconds, request.poll_seconds)
    })
}

fn quote(value: &str) -> String {
    // CommandLineToArgvW escaping, including backslashes before a closing quote.
    let mut result = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            result.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            result.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        result.push(c);
    }
    result.push_str(&"\\".repeat(slashes * 2));
    result.push('"');
    result
}
/// Called on a worker thread. Only Windows' UAC prompt requests elevation.
pub fn elevate(arguments: &[String]) -> Result<String> {
    let helper = std::env::current_exe()?.with_file_name("NetTimeNextNativeService.exe");
    ensure!(helper.exists(), "Native service helper is missing");
    let file = wide(&helper);
    let verb = wide("runas");
    let parameters = wide(
        arguments
            .iter()
            .map(|a| quote(a))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of_val(&info) as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_HIDE;
    let started = crate::protocol::utc();
    check(unsafe { ShellExecuteExW(&mut info) })?;
    ensure!(
        !info.hProcess.is_null(),
        "Administrator action did not start"
    );
    let process = Handle(info.hProcess);
    // The service action may take multiple stop/start waits; never block the UI.
    ensure!(
        unsafe { WaitForSingleObject(process.0, u32::MAX) } == WAIT_OBJECT_0,
        "Cannot wait for administrator action"
    );
    let mut code = 0;
    check(unsafe { GetExitCodeProcess(process.0, &mut code) })?;
    let message = std::fs::read_to_string(service::root().join("admin-result.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(|v| {
            v["utc"].as_f64().is_some_and(|utc| utc >= started - 1.)
                && v["command"].as_str() == arguments.first().map(String::as_str)
        })
        .and_then(|v| v["message"].as_str().map(String::from))
        .unwrap_or_else(|| format!("Service action exited with code {code}"));
    ensure!(code == 0, "{message}");
    Ok(message)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_argument_escaping() {
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("C:\\a\\"), "\"C:\\a\\\\\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
}
