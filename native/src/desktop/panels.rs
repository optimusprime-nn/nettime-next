use super::*;
use crate::{admin, service};
impl Desktop {
    pub(super) fn open_settings(&mut self) {
        if self.dirty_settings {
            self.settings = true;
            return;
        }
        match Config::load(&service::root().join("settings.json")) {
            Ok(cfg) => {
                self.settings_config = cfg;
                self.dirty_settings = false;
            }
            Err(e) => {
                self.message = format!(
                    "Service settings are unavailable: {e:#}. Install/register the Windows service first."
                )
            }
        }
        self.settings = true;
    }
    pub(super) fn operation(&mut self, work: impl FnOnce() -> Result<String> + Send + 'static) {
        self.reload_after_operation = false;
        let (tx, rx) = std::sync::mpsc::channel();
        self.operation = Some(rx);
        std::thread::spawn(move || {
            let result = work().map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
    }
    pub(super) fn administer(&mut self, arguments: Vec<String>) {
        self.message = "Waiting for administrator approval and service action…".into();
        self.operation(move || admin::elevate(&arguments));
        self.reload_after_operation = true;
    }
    pub(super) fn poll_operations(&mut self) {
        if let Some(worker) = &self.operation {
            match worker.try_recv() {
                Ok(result) => {
                    let reload = self.reload_after_operation && result.is_ok();
                    self.message = result.unwrap_or_else(|e| format!("Action failed: {e}"));
                    self.operation = None;
                    if reload && let Ok(cfg) = Config::load(&service::root().join("settings.json"))
                    {
                        self.settings_config = cfg;
                        self.dirty_settings = false;
                    }
                    if let Ok(cfg) = Config::load(&self.config_path) {
                        self.config = cfg;
                    }
                    self.refresh();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.operation = None;
                    self.message = "The background operation ended unexpectedly.".into();
                }
                _ => {}
            }
        }
        if let Some(worker) = &self.report_worker {
            match worker.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(report) => {
                            if !report.servers.iter().any(|s| s.peer == self.report_peer) {
                                self.report_peer = report
                                    .servers
                                    .first()
                                    .map(|s| s.peer.clone())
                                    .unwrap_or_default();
                            }
                            self.report = Some(report);
                        }
                        Err(e) => self.message = format!("Comparison unavailable: {e}"),
                    };
                    self.report_worker = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.report_worker = None;
                    self.message = "Comparison task ended unexpectedly.".into();
                }
                _ => {}
            }
        }
    }
    pub(super) fn request_report(&mut self) {
        if self.report_worker.is_some() {
            return;
        }
        let database = self.database.clone();
        let days = self.report_days;
        let (tx, rx) = std::sync::mpsc::channel();
        self.report_worker = Some(rx);
        std::thread::spawn(move || {
            let report = Store::open(&database, true)
                .and_then(|s| comparison::build(&s, days, crate::protocol::utc()))
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(report);
        });
    }
}

pub(super) fn report_folder() -> Result<PathBuf> {
    let root = std::env::var_os("USERPROFILE")
        .ok_or_else(|| anyhow::anyhow!("User profile directory unavailable"))?;
    Ok(PathBuf::from(root)
        .join("Documents/NetTime Next Reports")
        .join(format!(
            "report-{}",
            (crate::protocol::utc() * 1000.) as u64
        )))
}
