use super::*;
use crate::{admin, service};
impl Desktop {
    pub(super) fn open_settings(&mut self) {
        match Config::load(&service::root().join("settings.json")) {
            Ok(cfg) => self.config = cfg,
            Err(e) => {
                self.message = format!(
                    "Native service settings are unavailable: {e:#}. Install/register the Rust service first."
                )
            }
        }
        self.settings = true;
    }
    fn operation(&mut self, work: impl FnOnce() -> Result<String> + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.operation = Some(rx);
        std::thread::spawn(move || {
            let result = work().map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
    }
    fn administer(&mut self, arguments: Vec<String>) {
        self.message = "Waiting for administrator approval and service action…".into();
        self.operation(move || admin::elevate(&arguments));
    }
    pub(super) fn poll_operations(&mut self) {
        if let Some(worker) = &self.operation {
            match worker.try_recv() {
                Ok(result) => {
                    self.message = result.unwrap_or_else(|e| format!("Action failed: {e}"));
                    self.operation = None;
                    if self.settings
                        && let Ok(cfg) = Config::load(&service::root().join("settings.json"))
                    {
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
                        Ok(report) => self.report = Some(report),
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
    pub(super) fn change_source(&mut self, native: bool) {
        let root = if native {
            service::root()
        } else {
            crate::config::machine_root()
        };
        self.config_path = root.join("settings.json");
        self.database = root.join("data/history.sqlite3");
        match Config::load(&self.config_path) {
            Ok(cfg) => self.config = cfg,
            Err(e) => self.message = format!("Service settings unavailable: {e:#}"),
        };
        self.selected.clear();
        self.history.clear();
        self.report = None;
        self.report_worker = None;
        self.refresh();
        if self.comparison {
            self.request_report();
        }
    }
    pub(super) fn settings_panel(&mut self, ctx: &egui::Context) {
        let mut visible = self.settings;
        egui::Window::new("Settings · Native Windows service").open(&mut visible).default_width(1060.).vscroll(true).show(ctx,|ui|{
            ui.label("Applies to the Rust Windows service. Every configured server is checked and logged.");
            let state=crate::ownership::WindowsServices.snapshot(service::NAME).map(|s|if s.exists{format!("Native service: {} · Clock mode: {}",if s.running{"Running"}else{"Stopped"},self.config.clock.mode)}else{"Native service is not registered".into()}).unwrap_or_else(|e|format!("Service state unavailable: {e:#}"));
            ui.strong(state);
            ui.add_enabled_ui(self.operation.is_none(),|ui|{
                ui.horizontal(|ui|{
                    if ui.button("Start service").clicked(){self.administer(vec!["--start-service".into()]);}
                    if ui.add_enabled(self.config.clock.mode=="observe",egui::Button::new("Stop service")).on_disabled_hover_text("Restore previous clocks before stopping an active controller").clicked(){self.administer(vec!["--stop-service".into()]);}
                    if ui.button("Import current NetTime Next settings").clicked(){self.administer(vec!["--import-settings".into()]);}
                    if ui.button("Reload settings").clicked(){self.open_settings();}
                });
                ui.separator();
                egui::ScrollArea::horizontal().id_salt("source_settings_scroll").show(ui,|ui|{egui::Grid::new("settings_sources").striped(true).show(ui,|ui|{
                    for title in ["Server","NTP port","Security","NTS-KE port","Independent source group",""]{ui.strong(title);}ui.end_row();
                    let mut remove=None;
                    for(i,s)in self.config.servers.iter_mut().enumerate(){
                        ui.add(egui::TextEdit::singleline(&mut s.host).desired_width(190.));ui.add(egui::DragValue::new(&mut s.port).range(1..=65535));
                        egui::ComboBox::from_id_salt(("security",i)).selected_text(if s.security=="nts_required"{"NTS required"}else{"Plain NTP"}).show_ui(ui,|ui|{ui.selectable_value(&mut s.security,"plain_ntp".into(),"Plain NTP");ui.selectable_value(&mut s.security,"nts_required".into(),"NTS required");});
                        ui.add_enabled(s.security=="nts_required",egui::DragValue::new(&mut s.nts_ke_port).range(1..=65535));
                        ui.add(egui::TextEdit::singleline(&mut s.source_group).desired_width(150.));if ui.button("Remove").clicked(){remove=Some(i);}ui.end_row();
                    }
                    if let Some(i)=remove{self.config.servers.remove(i);}
                });});
                if self.config.servers.len()<32&&ui.button("Add server").clicked(){self.config.servers.push(Server::default());}
                ui.label("Sources in the same group are measured separately and contribute one timing vote. Assign groups according to independent upstream clocks.");
                ui.separator();
                egui::Grid::new("native_service_options").num_columns(2).show(ui,|ui|{
                    for(label,value,low,high)in [
                        ("Check every (seconds)",&mut self.config.poll_seconds,64.,131072.),
                        ("Keep history (days)",&mut self.config.retention_days,7.,3650.),
                        ("Response timeout (seconds)",&mut self.config.timeout_seconds,0.1,30.),
                        ("Maximum source distance (ms)",&mut self.config.max_distance_ms,1.,10000.),
                        ("Maximum automatic offset (ms)",&mut self.config.clock.max_offset_ms,1.,128.),
                        ("Maximum slew rate (ppm)",&mut self.config.clock.max_slew_ppm,1.,500.),
                        ("Maximum holdover (seconds)",&mut self.config.clock.holdover_seconds,64.,3600.),
                    ]{ui.label(label);ui.add(egui::DragValue::new(value).range(low..=high));ui.end_row();}
                    ui.label("Minimum independent source groups");ui.add(egui::DragValue::new(&mut self.config.clock.min_sources).range(3..=32));ui.end_row();
                });
                ui.checkbox(&mut self.config.clock.require_nts,"Require NTS-authenticated sources for clock control");
                ui.label("Settings are validated and applied with administrator approval. A running native service restarts; failed updates restore the previous configuration. Restore previous clocks before editing an active controller.");
                if ui.add_enabled(self.config.clock.mode=="observe",egui::Button::new("Apply settings to native service…")).clicked(){
                    let request=(||->Result<PathBuf>{let mut cfg=self.config.clone();cfg.validate()?;cfg.clock.mode="observe".into();let folder=PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or_else(||anyhow::anyhow!("Local application data is unavailable"))?).join("NetTimeNextNativePreview");std::fs::create_dir_all(&folder)?;let path=folder.join("settings-request.json");crate::windows::atomic_write(&path,&serde_json::to_vec_pretty(&cfg)?)?;Ok(path)})();
                    match request{Ok(path)=>self.administer(vec!["--apply-config".into(),path.to_string_lossy().into_owned()]),Err(e)=>self.message=format!("Settings invalid: {e:#}")}
                }
                ui.separator();ui.heading("Clock ownership");
                ui.label("Takeover requires fresh agreement within the offset limit. It saves the current service states, stops competing clock services and enables bounded gradual correction. Large offsets and lost agreement suspend correction. Clock steps are not performed.");
                ui.horizontal(|ui|{
                    if ui.button("Take over clock…").clicked(){self.administer(vec!["--takeover".into()]);}
                    if ui.button("Restore previous clocks…").clicked(){self.administer(vec!["--rollback".into()]);}
                });
            });
            if !self.message.is_empty(){ui.separator();ui.label(&self.message);}
        });
        self.settings = visible;
    }
    pub(super) fn comparison_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Compare recorded evidence over");
            let old = self.report_days;
            egui::ComboBox::from_id_salt("comparison_period")
                .selected_text(if self.report_days == 1. {
                    "24 hours"
                } else if self.report_days == 7. {
                    "7 days"
                } else {
                    "All retained"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.report_days, 1., "24 hours");
                    ui.selectable_value(&mut self.report_days, 7., "7 days");
                    ui.selectable_value(
                        &mut self.report_days,
                        self.config.retention_days.max(7.),
                        "All retained",
                    );
                });
            if (old != self.report_days
                || ui
                    .add_enabled(
                        self.report_worker.is_none(),
                        egui::Button::new("Refresh comparison"),
                    )
                    .clicked())
                && self.report_worker.is_none()
            {
                self.request_report();
            }
            if self.report_worker.is_some() {
                ui.spinner();
            }
            if ui
                .add_enabled(
                    self.report.is_some() && self.operation.is_none(),
                    egui::Button::new("Export CSV and JSON"),
                )
                .clicked()
            {
                let report = self.report.clone().unwrap();
                let database = self.database.clone();
                self.operation(move || {
                    let folder = report_folder()?;
                    comparison::export(&report, &folder)?;
                    let _ = database;
                    Ok(format!("Comparison exported to {}", folder.display()))
                });
            }
            if ui
                .add_enabled(
                    self.operation.is_none(),
                    egui::Button::new("Export full evidence"),
                )
                .clicked()
            {
                let database = self.database.clone();
                self.operation(move || {
                    let folder = report_folder()?;
                    Store::open(&database, true)?.export(&folder)?;
                    Ok(format!("Full evidence exported to {}", folder.display()))
                });
            }
        });
        let Some(report) = &self.report else {
            ui.label("Refresh to build a comparison from recorded evidence.");
            return;
        };
        ui.label(format!(
            "{} sources · {} selection changes · {:.1} days requested · gaps remain visible",
            report.servers.len(),
            report.selection_changes.len(),
            report.requested_days
        ));
        egui::ScrollArea::both()
            .id_salt("comparison_scroll")
            .show(ui, |ui| {
                egui::Grid::new("comparison_table")
                    .striped(true)
                    .min_col_width(95.)
                    .show(ui, |ui| {
                        for title in [
                            "Server",
                            "Valid attempts %",
                            "Coverage h",
                            "Median offset ms",
                            "P95 |offset| ms",
                            "Median RTT ms",
                            "P95 RTT ms",
                            "PC offset SD ms",
                            "Stratum range",
                            "Stratum changes",
                            "NTS valid %",
                            "Selected h",
                        ] {
                            ui.strong(title);
                        }
                        ui.end_row();
                        for s in &report.servers {
                            if ui
                                .selectable_label(self.report_peer == s.peer, &s.peer)
                                .clicked()
                            {
                                self.report_peer = s.peer.clone();
                            }
                            ui.label(format_number(s.valid_percent));
                            ui.label(format!("{:.2}", s.observed_hours));
                            for v in [
                                s.median_offset_ms,
                                s.p95_abs_offset_ms,
                                s.median_rtt_ms,
                                s.p95_rtt_ms,
                                s.offset_stddev_ms,
                            ] {
                                ui.label(format_number(v));
                            }
                            ui.label(
                                s.min_stratum
                                    .map(|v| format!("{}–{}", v, s.max_stratum.unwrap_or(v)))
                                    .unwrap_or_else(|| "—".into()),
                            );
                            ui.label(s.stratum_changes.to_string());
                            ui.label(format_number(s.authenticated_percent));
                            ui.label(format!("{:.2}", s.selected_hours));
                            ui.end_row();
                        }
                    });
                ui.separator();
                if report.offset_series.values().any(|points|!points.is_empty()) {
                    use egui_plot::{Plot,Points,PlotPoints,Legend};
                    ui.strong("Source offsets · recorded measurements");
                    Plot::new("comparison_offsets").height(260.).legend(Legend::default()).y_axis_label("Offset (ms)").x_axis_label("UTC").x_axis_formatter(|mark,_|crate::windows::utc_label(mark.value)).show(ui,|plot|{
                        for(peer,points)in &report.offset_series {
                            let points:PlotPoints=points.iter().copied().collect();
                            plot.points(Points::new(peer,points).radius(1.5_f32));
                        }
                    });
                    ui.label("Gaps contain no plotted measurements; hover or zoom to inspect recorded offsets.");
                }
                if let Some(s) = report.servers.iter().find(|s| s.peer == self.report_peer) {
                    ui.strong(&s.peer);
                    ui.label(format!(
                        "Valid {} / {} attempts · Longest gap {:.0}s · {} selection entries",
                        s.valid, s.attempts, s.longest_attempt_gap_seconds, s.selection_entries
                    ));
                    ui.label(format!(
                        "Recorded failures: {}",
                        serde_json::to_string(&s.failures).unwrap_or_default()
                    ));
                    for (reason, count) in &s.selection_reasons {
                        ui.label(format!("{count} decisions: {reason}"));
                    }
                }
                ui.separator();
                ui.strong("Selection changes (latest 100)");
                for c in report.selection_changes.iter().rev().take(100) {
                    ui.label(format!(
                        "{}   {} → {}",
                        utc_label(c.utc),
                        c.previous.as_deref().unwrap_or("none"),
                        c.selected.as_deref().unwrap_or("none")
                    ));
                    ui.label(&c.reason);
                    ui.add_space(5.);
                }
                ui.separator();
                for note in &report.notes {
                    ui.label(note);
                }
            });
    }
}
fn format_number(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.3}")).unwrap_or_else(|| "—".into())
}
fn utc_label(utc: f64) -> String {
    crate::windows::utc_label(utc)
}
fn report_folder() -> Result<PathBuf> {
    let root = std::env::var_os("USERPROFILE")
        .ok_or_else(|| anyhow::anyhow!("User profile directory unavailable"))?;
    Ok(PathBuf::from(root)
        .join("Documents/NetTime Next Reports")
        .join(format!(
            "report-{}",
            (crate::protocol::utc() * 1000.) as u64
        )))
}
