//! Native SCM host. Clock control requires a separate durable ownership transaction.
use crate::{config::Config, monitor::Monitor, storage::Store};
use anyhow::Result;
use std::{
    ffi::OsString,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
        ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};
pub const NAME: &str = "NetTimeNextNativePreview";
pub fn root() -> std::path::PathBuf {
    crate::config::machine_root().with_file_name("NetTimeNextNativePreview")
}
define_windows_service!(ffi_main, service_main);
pub fn dispatch() -> Result<()> {
    service_dispatcher::start(NAME, ffi_main)?;
    Ok(())
}
fn status(state: ServiceState, code: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(code),
        checkpoint: 0,
        wait_hint: Duration::from_secs(15),
        process_id: None,
    }
}
fn service_main(_args: Vec<OsString>) {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = match service_control_handler::register(NAME, move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            flag.store(true, Ordering::Relaxed);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    }) {
        Ok(h) => h,
        Err(_) => return,
    };
    let result = (|| -> Result<()> {
        handle.set_service_status(status(ServiceState::StartPending, 0))?;
        // A crashed process cannot run Drop. On SCM recovery, clear its former
        // rate before parsing configuration, but only with verified ownership.
        if crate::ownership::guard(&root(), &mut crate::ownership::WindowsServices).unwrap_or(false)
        {
            use crate::clock::Backend;
            crate::clock::WindowsClock::new()?.restore()?;
        }
        let cfg = Config::load(&root().join("settings.json"))?;
        let store = Store::open(&root().join("data/history.sqlite3"), false)?;
        let mut monitor = Monitor::for_service(cfg, store, &root())?;
        handle.set_service_status(status(ServiceState::Running, 0))?;
        monitor.run(stop)
    })();
    if let Err(e) = &result {
        let _ = std::fs::write(
            root().join("data/service-error.log"),
            format!("{} {e:#}", crate::protocol::utc()),
        );
    }
    let _ = handle.set_service_status(status(
        ServiceState::Stopped,
        if result.is_ok() { 0 } else { 1 },
    ));
}
pub fn install() -> Result<()> {
    crate::windows::require_admin()?;
    anyhow::ensure!(
        crate::ownership::journal(&root())?
            .is_none_or(|j| ["active", "rolled_back"].contains(&j.phase.as_str())),
        "Clock ownership restoration is incomplete"
    );
    let owned = crate::ownership::active(&root())?;
    if owned {
        anyhow::ensure!(
            crate::ownership::guard(&root(), &mut crate::ownership::WindowsServices)?
                && Config::load(&root().join("settings.json"))?.clock.mode == "slew",
            "Clock ownership conflict; service upgrade refused"
        );
    }
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: "NetTime Next Native".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?.with_file_name("NetTimeNextNativeService.exe"),
        launch_arguments: vec!["--service".into()],
        dependencies: vec![],
        account_name: Some(if owned {
            crate::ownership::account().into()
        } else {
            "NT AUTHORITY\\LocalService".into()
        }),
        account_password: None,
    };
    let service =
        match manager.open_service(NAME, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START) {
            Ok(service) => {
                service.change_config(&info)?;
                service
            }
            Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(1060) => manager
                .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?,
            Err(e) => return Err(e.into()),
        };
    service.set_description(
        "Native NTP/NTS measurement service with explicit bounded clock takeover and rollback.",
    )?;
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
        reboot_msg: Some("".into()),
        command: Some("".into()),
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::None,
                delay: Duration::ZERO,
            },
        ]),
    })?;
    // Deliberate safety stops must remain stopped for operator review.
    service.set_failure_actions_on_non_crash_failures(false)?;
    // A virtual account needs access to its data after a later explicit takeover.
    // Only this account is granted a data ACL; clock rights are not granted here.
    crate::windows::protect_service_files(&root(), &crate::ownership::account())?;
    Ok(())
}
pub fn action(command: &str) -> Result<String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let access = ServiceAccess::QUERY_STATUS
        | match command {
            "start" => ServiceAccess::START,
            "stop" => ServiceAccess::STOP,
            "remove" => ServiceAccess::DELETE,
            _ => ServiceAccess::empty(),
        };
    let service = manager.open_service(NAME, access)?;
    match command {
        "start" => {
            use crate::ownership::Services;
            crate::ownership::WindowsServices.change(NAME, true)?;
        }
        "stop" => {
            if service.query_status()?.current_state != ServiceState::Stopped {
                service.stop()?;
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                while service.query_status()?.current_state != ServiceState::Stopped {
                    anyhow::ensure!(
                        std::time::Instant::now() < deadline,
                        "Native service did not stop in time"
                    );
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
        "remove" => {
            crate::ownership::rollback(&root(), &mut crate::ownership::WindowsServices)?;
            use crate::ownership::Services;
            crate::ownership::WindowsServices.change(NAME, false)?;
            service.delete()?;
        }
        _ => {}
    }
    Ok(format!("{:?}", service.query_status()?.current_state))
}
