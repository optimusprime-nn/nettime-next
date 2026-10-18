pub mod config;
mod desktop;
mod monitor;
mod nts;
pub mod protocol;
pub mod selection;
mod service;
mod storage;
use anyhow::{Result, ensure};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help") {
        println!(
            "NetTime Next Rust migration\n\nDesktop: no options (reads current service history)\n--config FILE --database FILE\n--once / --headless: native collector; requires --config and --data-dir\n--export FOLDER: export history\n--service: Windows migration service dispatcher\n--install-service / --start-service / --stop-service / --remove-service / --status-service\n\nMigration collectors never adjust the clock or change the active service."
        );
        return Ok(());
    }
    if args.iter().any(|a| a == "--service") {
        return service::dispatch();
    }
    if args.iter().any(|a| a == "--install-service") {
        return service::install();
    }
    for action in ["start", "stop", "remove", "status"] {
        if args.iter().any(|a| a == &format!("--{action}-service")) {
            println!("{}", service::action(action)?);
            return Ok(());
        }
    }
    let value = |key: &str| -> Result<Option<PathBuf>> {
        if let Some(i) = args.iter().position(|a| a == key) {
            ensure!(i + 1 < args.len(), "Missing value for {key}");
            Ok(Some(PathBuf::from(&args[i + 1])))
        } else {
            Ok(None)
        }
    };
    let config = value("--config")?.unwrap_or_else(|| config::machine_root().join("settings.json"));
    let collector = args.iter().any(|a| a == "--once" || a == "--headless");
    let data = value("--data-dir")?;
    if collector {
        ensure!(
            data.is_some() && args.iter().any(|a| a == "--config"),
            "Native collector requires explicit --config and separate --data-dir"
        );
    }
    let database = value("--database")?.unwrap_or_else(|| {
        data.unwrap_or_else(|| config::machine_root().join("data"))
            .join("history.sqlite3")
    });
    if let Some(folder) = value("--export")? {
        return storage::Store::open(&database, true)?.export(&folder);
    }
    if collector {
        let cfg = config::Config::load(&config)?;
        let active = config::machine_root().join("data/history.sqlite3");
        if database.exists() {
            ensure!(
                std::fs::canonicalize(&database)?
                    != std::fs::canonicalize(&active).unwrap_or(active),
                "Native collector must use a separate database"
            );
        }
        let mut monitor = monitor::Monitor::new(cfg, storage::Store::open(&database, false)?)?;
        if args.iter().any(|a| a == "--once") {
            println!("{}", serde_json::to_string_pretty(&monitor.collect()?)?);
            Ok(())
        } else {
            monitor.run(Arc::new(AtomicBool::new(false)))
        }
    } else {
        desktop::run(database, config, args.iter().any(|a| a == "--smoke-ui"))
    }
}
