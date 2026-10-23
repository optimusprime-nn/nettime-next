use super::*;
use style::{AMBER, BLUE, badge, card, section};
impl Desktop {
    pub(super) fn save_appearance(&mut self, ctx: &egui::Context) {
        style::apply(ctx, self.dark);
        ctx.set_zoom_factor(self.zoom);
        if let Err(e) = (style::Preferences {
            dark: self.dark,
            zoom: self.zoom,
        })
        .save()
        {
            self.message = format!("Appearance changed for this session; could not save: {e:#}");
        }
    }
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.weak("Configure the Windows service. Changes are applied only when you choose Save and apply.");
        ui.add_space(8.);
        ui.horizontal_wrapped(|ui| {
            for (i, title) in [
                "Time sources",
                "Collection & history",
                "Clock control",
                "Windows service",
                "Appearance",
            ]
            .iter()
            .enumerate()
            {
                ui.selectable_value(&mut self.settings_tab, i, *title);
            }
        });
        ui.add_space(12.);
        let before = serde_json::to_vec(&self.settings_config).unwrap_or_default();
        let busy = self.operation.is_some();
        card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.add_enabled_ui(!busy, |ui| match self.settings_tab {
                0 => self.source_settings(ui),
                1 => self.collection_settings(ui),
                2 => self.clock_settings(ui),
                3 => self.service_settings(ui),
                _ => self.appearance_settings(ui, ctx),
            });
        });
        if before != serde_json::to_vec(&self.settings_config).unwrap_or_default() {
            self.dirty_settings = true;
        }
        if self.settings_tab < 3 {
            ui.add_space(12.);
            card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.dirty_settings { badge(ui, "UNSAVED CHANGES", AMBER); }
                    else { ui.weak("Settings loaded from the service."); }
                    if ui.add_enabled(!busy, egui::Button::new("Save and apply…").fill(ui.visuals().selection.bg_fill)).on_disabled_hover_text("Changes are checked before the collector restarts; existing clock ownership is preserved.").clicked() {
                        self.apply_settings();
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Discard changes / reload")).clicked() { self.dirty_settings = false; self.open_settings(); }
                });
                ui.small("Saving requests administrator approval. During clock control, changed sources are tested first. The service restarts with fresh filters, retains history and keeps ownership; failed updates restore the previous settings.");
            });
        }
        if self.confirm_retention {
            let mut apply = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("shorter_retention")).show(ctx, |ui| {
                ui.heading("Delete older history?");
                ui.label(format!("You are reducing retention from {:.0} to {:.0} days.", self.config.retention_days, self.settings_config.retention_days));
                ui.label(format!("Saving will permanently delete measurements, decisions and events older than {:.0} days, including compressed records. This cannot be undone.", self.settings_config.retention_days));
                ui.label("Export your history first if you want to keep it. Clock ownership and current measurements are preserved.");
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked();
                    apply = ui.button("Save and delete older history…").clicked();
                });
            });
            if cancel {
                self.confirm_retention = false;
            }
            if apply {
                self.confirm_retention = false;
                self.save_confirmed_settings();
            }
        }
    }
    fn source_settings(&mut self, ui: &mut egui::Ui) {
        section(
            ui,
            "Time sources",
            "All listed endpoints are checked. NTP uses UDP; NTS adds authenticated time and TLS key exchange.",
        );
        let mut remove = None;
        let mut move_to = None;
        let source_count = self.settings_config.servers.len();
        for (index, source) in self.settings_config.servers.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                egui::Frame::group(ui.style()).inner_margin(12).corner_radius(8).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(format!("Source {}", index + 1));
                        if ui.add_enabled(index > 0, egui::Button::new("Move up").small()).clicked() { move_to = Some((index, index - 1)); }
                        if ui.add_enabled(index + 1 < source_count, egui::Button::new("Move down").small()).clicked() { move_to = Some((index, index + 1)); }
                        ui.label("Host / IP");
                        ui.add(egui::TextEdit::singleline(&mut source.host).desired_width(230.));
                        ui.label("NTP port");
                        ui.add(egui::DragValue::new(&mut source.port).range(1..=65535));
                        if ui.small_button("Remove").clicked() { remove = Some(index); }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Security");
                        egui::ComboBox::from_id_salt("security").selected_text(if source.security == "nts_required" { "NTS required" } else { "Plain NTP · unauthenticated" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut source.security, "plain_ntp".into(), "Plain NTP · unauthenticated");
                                ui.selectable_value(&mut source.security, "nts_required".into(), "NTS required");
                            });
                        if source.security == "nts_required" {
                            ui.label("NTS-KE port");
                            ui.add(egui::DragValue::new(&mut source.nts_ke_port).range(1..=65535));
                        }
                        ui.label("Independent source group");
                        ui.add(egui::TextEdit::singleline(&mut source.source_group).desired_width(175.)).on_hover_text("Endpoints sharing an upstream clock should share a group. Each group contributes one timing vote.");
                    });
                });
            });
        }
        if let Some(index) = remove {
            self.settings_config.servers.remove(index);
        } else if let Some((from, to)) = move_to {
            self.settings_config.servers.swap(from, to);
        }
        if ui
            .add_enabled(
                self.settings_config.servers.len() < 32,
                egui::Button::new("+ Add time source"),
            )
            .clicked()
        {
            self.settings_config.servers.push(Server::default());
        }
        ui.add_space(8.);
        ui.weak("Move up or down to choose the saved order, then Save and apply. Overview follows this order when Configured order is selected.");
        ui.weak("Set groups according to independent upstream clocks. Different IP addresses or stratum values alone do not establish independence.");
        ui.weak("A source requiring NTS will report authentication failures rather than silently fall back to plain NTP.");
    }
    fn collection_settings(&mut self, ui: &mut egui::Ui) {
        section(
            ui,
            "Collection & history",
            "Keep enough evidence to compare every source and understand each selection decision.",
        );
        ui.checkbox(
            &mut self.settings_config.logging_enabled,
            "Record history and event logs",
        );
        ui.small("When off, clock control and live status continue. Only the current status and safety state are kept; existing history still expires at the retention limit.");
        ui.add_enabled_ui(self.settings_config.logging_enabled, |ui| {
            ui.checkbox(&mut self.settings_config.diagnostic_logging, "Detailed diagnostic logging");
            ui.small("Includes packet headers and full decision snapshots. Uses more disk space; leave off for normal use.");
        });
        ui.add_space(8.);
        egui::Grid::new("collection_options")
            .spacing([28., 16.])
            .show(ui, |ui| {
                for (label, value, low, high, unit) in [
                    (
                        "Check interval",
                        &mut self.settings_config.poll_seconds,
                        64.,
                        131072.,
                        "seconds · server rate limits can extend this",
                    ),
                    (
                        "Response timeout",
                        &mut self.settings_config.timeout_seconds,
                        0.1,
                        30.,
                        "seconds",
                    ),
                    (
                        "History retention",
                        &mut self.settings_config.retention_days,
                        1.,
                        3650.,
                        "days · 1–3650; default 7",
                    ),
                    (
                        "Maximum source distance",
                        &mut self.settings_config.max_distance_ms,
                        1.,
                        10000.,
                        "milliseconds · reject more uncertain sources",
                    ),
                ] {
                    ui.label(label);
                    ui.add(egui::DragValue::new(value).range(low..=high));
                    ui.weak(unit);
                    ui.end_row();
                }
            });
        if self.settings_config.retention_days < self.config.retention_days {
            ui.colored_label(AMBER, format!("Reducing retention to {:.0} days will permanently delete older history after you confirm Save and apply.", self.settings_config.retention_days));
        }
        ui.small("Records are compressed automatically after 24 hours and removed when they exceed retention. Cleanup runs after startup/settings changes and hourly thereafter.");
        ui.add_space(16.);
        ui.strong("Included when logging is enabled");
        for line in [
            "Each attempted response, including errors and authentication failures",
            "Offset, round trip, stratum, reference, reach and source security",
            "Filtered candidates, agreeing sources, exclusions and selection reasons",
            "Clock actions, rate limits, suspension reasons and service events",
        ] {
            ui.label(format!("• {line}"));
        }
        ui.add_space(8.);
        ui.weak("Source comparison exports CSV and JSON summaries. Full history exports use compressed JSON (.json.gz).");
    }
    fn clock_settings(&mut self, ui: &mut egui::Ui) {
        section(
            ui,
            "Clock control",
            "The service starts in measurement mode. Taking over is a separate, explicit action.",
        );
        badge(
            ui,
            if self.settings_config.clock.mode == "observe" {
                "MEASUREMENT MODE"
            } else {
                "CLOCK CONTROL CONFIGURED"
            },
            BLUE,
        );
        ui.add_space(10.);
        egui::Grid::new("clock_options")
            .spacing([28., 16.])
            .show(ui, |ui| {
                ui.label("Minimum independent source groups");
                ui.add(
                    egui::DragValue::new(&mut self.settings_config.clock.min_sources).range(3..=32),
                );
                ui.weak("Required for source agreement");
                ui.end_row();
                for (label, value, low, high, unit) in [
                    (
                        "Maximum automatic offset",
                        &mut self.settings_config.clock.max_offset_ms,
                        1.,
                        128.,
                        "ms · larger offsets suspend correction",
                    ),
                    (
                        "Maximum correction rate",
                        &mut self.settings_config.clock.max_slew_ppm,
                        1.,
                        500.,
                        "ppm · bounded gradual adjustment",
                    ),
                    (
                        "Maximum holdover",
                        &mut self.settings_config.clock.holdover_seconds,
                        64.,
                        3600.,
                        "seconds without fresh agreement",
                    ),
                ] {
                    ui.label(label);
                    ui.add(egui::DragValue::new(value).range(low..=high));
                    ui.weak(unit);
                    ui.end_row();
                }
            });
        ui.add_space(10.);
        ui.checkbox(
            &mut self.settings_config.clock.require_nts,
            "Require NTS-authenticated sources for clock control",
        );
        ui.weak("NTS must also be enabled on enough independently grouped sources. Unauthenticated measurements remain visible.");
        ui.separator();
        section(
            ui,
            "Clock ownership",
            "Takeover saves the previous service states, stops competing clock controllers and enables bounded gradual correction.",
        );
        ui.label("Loss of agreement, conflicting controllers and excessive offsets suspend correction. The controller does not step the clock.");
        if self.dirty_settings {
            ui.colored_label(
                AMBER,
                "Save or discard your settings changes before changing clock ownership.",
            );
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.dirty_settings, egui::Button::new("Take over clock…"))
                .clicked()
            {
                self.administer(vec!["--takeover".into()]);
            }
            if ui
                .add_enabled(
                    !self.dirty_settings,
                    egui::Button::new("Restore previous clocks…"),
                )
                .clicked()
            {
                self.administer(vec!["--rollback".into()]);
            }
        });
        ui.weak("Both actions request administrator approval. Takeover also checks fresh source agreement and the configured offset limit.");
    }
    fn service_settings(&mut self, ui: &mut egui::Ui) {
        section(
            ui,
            "Windows service",
            "The collector runs independently of the desktop and starts automatically with Windows.",
        );
        ui.strong(&self.native_service_state);
        ui.add_space(8.);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Start service…").clicked() {
                self.administer(vec!["--start-service".into()]);
            }
            if ui
                .add_enabled(
                    self.settings_config.clock.mode == "observe",
                    egui::Button::new("Stop service…"),
                )
                .on_disabled_hover_text(
                    "Restore previous clocks before stopping an active controller.",
                )
                .clicked()
            {
                self.administer(vec!["--stop-service".into()]);
            }
        });
        ui.add_space(16.);
        ui.strong("Storage locations");
        ui.label(format!(
            "Settings: {}",
            crate::service::root().join("settings.json").display()
        ));
        ui.label(format!(
            "History: {}",
            crate::service::root()
                .join("data/history.sqlite3")
                .display()
        ));
        ui.weak("The desktop reads history without administrator access. Configuration and clock ownership changes use a separate administrator helper.");
    }
    fn appearance_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        section(
            ui,
            "Appearance",
            "Display preferences are saved for your Windows user and apply immediately.",
        );
        let mut changed = ui.checkbox(&mut self.dark, "Dark appearance").changed();
        ui.horizontal(|ui| {
            ui.label("Interface size");
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.zoom, 0.85..=1.5)
                        .step_by(0.05)
                        .suffix("×"),
                )
                .changed();
            if ui.button("Reset size").clicked() {
                self.zoom = 1.;
                changed = true;
            }
        });
        if changed {
            self.save_appearance(ctx);
        }
        ui.add_space(12.);
        ui.strong("System tray");
        ui.label("Minimise keeps the window on the taskbar. Closing with X hides it in the notification area. Click the clock icon to reopen it. Right-click it for Open, Settings and Exit desktop app.");
        ui.weak("Closing the desktop leaves the Windows collector running.");
    }
    fn apply_settings(&mut self) {
        if let Ok(current) = Config::load(&self.config_path) {
            self.config.retention_days = current.retention_days;
        }
        if self.settings_config.retention_days < self.config.retention_days {
            self.confirm_retention = true;
            return;
        }
        self.save_confirmed_settings();
    }
    fn save_confirmed_settings(&mut self) {
        let request = (|| -> Result<PathBuf> {
            let mut cfg = self.settings_config.clone();
            cfg.validate()?;
            let folder = PathBuf::from(
                std::env::var_os("LOCALAPPDATA")
                    .ok_or_else(|| anyhow::anyhow!("Local application data is unavailable"))?,
            )
            .join("NetTimeNextNativePreview");
            std::fs::create_dir_all(&folder)?;
            let path = folder.join("settings-request.json");
            crate::windows::atomic_write(&path, &serde_json::to_vec_pretty(&cfg)?)?;
            Ok(path)
        })();
        match request {
            Ok(path) => self.administer(vec![
                "--apply-config".into(),
                path.to_string_lossy().into_owned(),
            ]),
            Err(e) => self.message = format!("Settings invalid: {e:#}"),
        }
    }
}
