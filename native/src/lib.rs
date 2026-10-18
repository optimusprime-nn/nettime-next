mod admin;
mod clock;
mod comparison;
pub mod config;
mod desktop;
mod monitor;
mod nts;
mod ownership;
pub mod protocol;
pub mod selection;
mod service;
mod storage;
mod validation;
mod windows;
use anyhow::{Result, ensure};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--ci-clock-response") {
        ensure!(args.len() == 2, "Supply a clock-response output file");
        return validation::clock_response(std::path::Path::new(&args[1]));
    }
    if admin::dispatch(&args)? {
        return Ok(());
    }
    if args.iter().any(|a| a == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help") {
        println!(
            "NetTime Next Native\n\nDesktop: native service history when installed, otherwise current NetTime Next history\n--config FILE --database FILE\n--once / --headless: observation only; requires --config and separate --data-dir\n--export FOLDER: export full evidence\n--compare-export FOLDER --days DAYS: export windowed CSV/JSON report\n--service: Windows service dispatcher\n--install-service / --start-service / --stop-service / --remove-service / --status-service\n--apply-config FILE / --import-settings / --takeover / --rollback: explicit administrator actions\n--health: current native service and ownership evidence\n\nOnly the authorized Windows service can control the clock. Install and settings apply default to observation."
        );
        return Ok(());
    }
    if args.iter().any(|a| a == "--service") {
        return service::dispatch();
    }
    if args.iter().any(|a| a == "--health") {
        use ownership::Services;
        let cfg = config::Config::load(&service::root().join("settings.json"))?;
        let decision =
            storage::Store::open(&service::root().join("data/history.sqlite3"), true)?.latest()?;
        println!(
            "{}",
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"service":ownership::WindowsServices.snapshot(service::NAME)?,"mode":cfg.clock.mode,"ownership":ownership::journal(&service::root())?,"clock_privilege":ownership::WindowsServices.privilege(None).ok(),"latest":decision})
        );
        return Ok(());
    }
    if args.iter().any(|a| a == "--status-service") {
        println!("{}", service::action("status")?);
        return Ok(());
    }
    let value = |key: &str| -> Result<Option<PathBuf>> {
        if let Some(i) = args.iter().position(|a| a == key) {
            ensure!(i + 1 < args.len(), "Missing value for {key}");
            Ok(Some(PathBuf::from(&args[i + 1])))
        } else {
            Ok(None)
        }
    };
    let config = value("--config")?.unwrap_or_else(|| {
        let native = service::root().join("settings.json");
        if native.exists() {
            native
        } else {
            config::machine_root().join("settings.json")
        }
    });
    let collector = args.iter().any(|a| a == "--once" || a == "--headless");
    let data = value("--data-dir")?;
    if collector {
        ensure!(
            data.is_some() && args.iter().any(|a| a == "--config"),
            "Native collector requires explicit --config and separate --data-dir"
        );
    }
    let database = value("--database")?.unwrap_or_else(|| {
        data.unwrap_or_else(|| {
            config
                .parent()
                .unwrap_or(&config::machine_root())
                .join("data")
        })
        .join("history.sqlite3")
    });
    if let Some(folder) = value("--export")? {
        return storage::Store::open(&database, true)?.export(&folder);
    }
    if let Some(folder) = value("--compare-export")? {
        let days = if let Some(i) = args.iter().position(|a| a == "--days") {
            args.get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("Missing --days value"))?
                .parse::<f64>()?
        } else {
            7.
        };
        let report = comparison::build(
            &storage::Store::open(&database, true)?,
            days,
            protocol::utc(),
        )?;
        return comparison::export(&report, &folder);
    }
    if collector {
        let cfg = config::Config::load(&config)?;
        for active in [
            config::machine_root().join("data/history.sqlite3"),
            service::root().join("data/history.sqlite3"),
        ] {
            if database.exists() {
                ensure!(
                    std::fs::canonicalize(&database)?
                        != std::fs::canonicalize(&active).unwrap_or(active),
                    "Native collector must use a separate database"
                );
            }
        }
        let mut monitor = monitor::Monitor::new(cfg, storage::Store::open(&database, false)?)?;
        if args.iter().any(|a| a == "--once") {
            println!("{}", serde_json::to_string_pretty(&monitor.collect()?)?);
            Ok(())
        } else {
            monitor.run(Arc::new(AtomicBool::new(false)))
        }
    } else {
        desktop::run(
            database,
            config,
            args.iter().any(|a| {
                ["--smoke-ui", "--smoke-settings", "--smoke-comparison"].contains(&a.as_str())
            }),
            if args.iter().any(|a| a == "--smoke-settings") {
                "settings"
            } else if args.iter().any(|a| a == "--smoke-comparison") {
                "comparison"
            } else {
                "live"
            }
            .into(),
        )
    }
}
