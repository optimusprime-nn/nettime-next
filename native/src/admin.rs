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
    Foundation::{HANDLE, LocalFree, WAIT_ABANDONED, WAIT_OBJECT_0},
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, CreateWellKnownSid,
        SECURITY_ATTRIBUTES, WinBuiltinAdministratorsSid,
    },
    System::Threading::{
        AddSIDToBoundaryDescriptor, ClosePrivateNamespace, CreateBoundaryDescriptorW, CreateMutexW,
        CreatePrivateNamespaceW, DeleteBoundaryDescriptor, GetExitCodeProcess,
        OpenPrivateNamespaceW, ReleaseMutex, WaitForSingleObject,
    },
    UI::{
        Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_HIDE,
    },
};

/// Serialises administrator actions. The mutex lives in a private namespace
/// whose boundary requires the Administrators group, so a standard user cannot
/// create or hold it. A mutex in the Global namespace could be pre-created by
/// any user and held open to block every administrator action.
struct Lock {
    mutex: Option<Handle>,
    namespace: HANDLE,
    boundary: HANDLE,
}
impl Lock {
    fn new() -> Result<Self> {
        let mut lock = Self {
            mutex: None,
            namespace: std::ptr::null_mut(),
            boundary: std::ptr::null_mut(),
        };
        unsafe {
            lock.boundary =
                CreateBoundaryDescriptorW(wide("NetTimeNextNativeAdministration").as_ptr(), 0);
            ensure!(
                !lock.boundary.is_null(),
                "Cannot lock native service administration"
            );
            let mut sid = [0u8; 68];
            let mut size = sid.len() as u32;
            check(CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                std::ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut size,
            ))?;
            check(AddSIDToBoundaryDescriptor(
                &mut lock.boundary,
                sid.as_mut_ptr().cast(),
            ))?;
            let mut descriptor = std::ptr::null_mut();
            check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)").as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            ))?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let alias = wide("NetTimeNextNative");
            lock.namespace = CreatePrivateNamespaceW(&attributes, lock.boundary, alias.as_ptr());
            if lock.namespace.is_null() {
                lock.namespace = OpenPrivateNamespaceW(lock.boundary, alias.as_ptr());
            }
            let mutex = if lock.namespace.is_null() {
                std::ptr::null_mut()
            } else {
                CreateMutexW(
                    &attributes,
                    0,
                    wide("NetTimeNextNative\\Administration").as_ptr(),
                )
            };
            LocalFree(descriptor);
            ensure!(
                !mutex.is_null(),
                "Cannot lock native service administration: {}",
                std::io::Error::last_os_error()
            );
            let mutex = Handle(mutex);
            let result = WaitForSingleObject(mutex.0, 0);
            ensure!(
                [WAIT_OBJECT_0, WAIT_ABANDONED].contains(&result),
                "Another service administration action is in progress"
            );
            // Only an owned mutex is released on drop.
            lock.mutex = Some(mutex);
        }
        Ok(lock)
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            if let Some(mutex) = self.mutex.take() {
                ReleaseMutex(mutex.0);
            }
            if !self.namespace.is_null() {
                ClosePrivateNamespace(self.namespace, 0);
            }
            if !self.boundary.is_null() {
                DeleteBoundaryDescriptor(self.boundary);
            }
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
            "--secure-data-folder",
            "--abort-upgrade",
        ]
        .contains(s)
    }) else {
        return Ok(false);
    };
    require_admin()?;
    let _lock = Lock::new()?;
    if command == "--secure-data-folder" {
        // Nothing in the folder is read before it is checked.
        use ownership::Services;
        ensure!(
            !WindowsServices.snapshot(service::NAME)?.exists,
            "The service is registered; use the upgrade path"
        );
        if let Some(aside) = crate::windows::secure_data_root(&service::root())? {
            println!("Moved untrusted data folder to {}", aside.display());
        }
        return Ok(true);
    }
    if ![
        "--prepare-upgrade",
        "--finish-upgrade",
        "--abort-upgrade",
        "--rollback",
    ]
    .contains(&command)
    {
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
        "--abort-upgrade" => crate::upgrade::abort(),
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
    fn standard_users_cannot_take_the_administration_lock() {
        if require_admin().is_err() {
            // A non-elevated token lacks the boundary's Administrators SID.
            assert!(Lock::new().is_err());
        } else {
            let held = Lock::new().unwrap();
            // Mutexes are re-entrant for their owner; contend from another thread.
            assert!(std::thread::spawn(|| Lock::new().is_err()).join().unwrap());
            drop(held);
            Lock::new().unwrap();
        }
    }
    #[test]
    fn windows_argument_escaping() {
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("C:\\a\\"), "\"C:\\a\\\\\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
}
