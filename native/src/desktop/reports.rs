use super::*;
use style::{BLUE, card, duration, section, summary};
fn value(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.3}")).unwrap_or_else(|| "—".into())
}
fn colour(index: usize) -> egui::Color32 {
    [
        BLUE,
        style::GREEN,
        style::AMBER,
        egui::Color32::from_rgb(184, 143, 255),
        egui::Color32::from_rgb(245, 131, 169),
        egui::Color32::from_rgb(111, 213, 231),
    ][index % 6]
}
impl Desktop {
    pub(super) fn comparison_panel(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Recorded period");
                let old = self.report_days;
                ui.add_enabled_ui(self.report_worker.is_none(), |ui| {
                    let mut periods = vec![(1. / 24., "1 hour"), (1., "24 hours"), (7., "7 days")];
                    if self.config.retention_days != 7. {
                        periods.push((self.config.retention_days, "All retained"));
                    }
                    for (days, name) in periods {
                        ui.selectable_value(&mut self.report_days, days, name);
                    }
                });
                if (old != self.report_days
                    || ui
                        .add_enabled(
                            self.report_worker.is_none(),
                            egui::Button::new("Refresh report"),
                        )
                        .clicked())
                    && self.report_worker.is_none()
                {
                    self.request_report();
                }
                if self.report_worker.is_some() {
                    ui.spinner();
                    ui.weak("Reading recorded history…");
                }
                ui.menu_button("Export reports", |ui| {
                    if ui
                        .add_enabled(
                            self.report.is_some() && self.operation.is_none(),
                            egui::Button::new("Comparison · CSV and JSON"),
                        )
                        .clicked()
                    {
                        let report = self.report.clone().unwrap();
                        self.operation(move || {
                            let folder = super::panels::report_folder()?;
                            comparison::export(&report, &folder)?;
                            Ok(format!("Comparison exported to {}", folder.display()))
                        });
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.operation.is_none(),
                            egui::Button::new("Full samples, decisions and events · JSON"),
                        )
                        .clicked()
                    {
                        let database = self.database.clone();
                        self.operation(move || {
                            let folder = super::panels::report_folder()?;
                            Store::open(&database, true)?.export(&folder)?;
                            Ok(format!("Full evidence exported to {}", folder.display()))
                        });
                        ui.close();
                    }
                });
            });
            ui.weak("Compare recorded source quality and selection decisions. Records older than 24 hours are compressed automatically.");
        });
        ui.add_space(12.);
        let Some(report) = &self.report else {
            ui.weak("Choose a period or refresh to build a comparison from retained measurements.");
            return;
        };
        let attempts: usize = report.servers.iter().map(|s| s.attempts).sum();
        let valid: usize = report.servers.iter().map(|s| s.valid).sum();
        summary(
            ui,
            [
                (
                    "RECORDED SOURCES",
                    report.servers.len().to_string(),
                    "Sources with attempts in this period".into(),
                ),
                (
                    "VALID ATTEMPTS",
                    if attempts == 0 {
                        "—".into()
                    } else {
                        format!("{:.1}%", valid as f64 * 100. / attempts as f64)
                    },
                    format!("{valid} / {attempts} attempts"),
                ),
                (
                    "SELECTION CHANGES",
                    report.selection_changes.len().to_string(),
                    "Includes transitions to no selection".into(),
                ),
                (
                    "REPORT BUILT",
                    format!("{} ago", duration(crate::protocol::utc() - report.end_utc)),
                    crate::windows::utc_label(report.end_utc),
                ),
            ],
        );
        ui.add_space(12.);
        card(ui, |ui| {
            section(
                ui,
                "Source offsets",
                "Overlay of recorded measurements relative to this PC · UTC",
            );
            if report
                .offset_series
                .values()
                .any(|points| !points.is_empty())
            {
                use egui_plot::{Legend, Plot, PlotPoints, Points};
                let colours: Vec<_> = (0..report.offset_series.len())
                    .map(|i| style::ink(ui, colour(i)))
                    .collect();
                Plot::new((
                    "comparison_offsets",
                    report.start_utc.to_bits(),
                    report.end_utc.to_bits(),
                ))
                .height(270.)
                .legend(Legend::default())
                .include_y(0.)
                .y_axis_label("Offset (ms)")
                .x_axis_label("UTC")
                .x_axis_formatter(|mark, _| crate::windows::utc_label(mark.value))
                .label_formatter(|name, p| {
                    format!("{name}\n{}\n{:.3} ms", crate::windows::utc_label(p.x), p.y)
                })
                .show(ui, |plot| {
                    for (index, (peer, points)) in report.offset_series.iter().enumerate() {
                        plot.points(
                            Points::new(peer, PlotPoints::from(points.clone()))
                                .color(colours[index])
                                .radius(1.8_f32),
                        );
                    }
                });
                ui.weak("Click a legend entry to hide a source. Drag to pan and scroll to zoom. Missing measurements remain gaps.");
            } else {
                ui.weak("No valid offsets were recorded in this period.");
            }
        });
        ui.add_space(12.);
        card(ui, |ui| {
            section(
                ui,
                "Source quality",
                "Select a source for its full statistics, recorded failures and selection reasons.",
            );
            egui::ScrollArea::horizontal()
                .id_salt("comparison_table_scroll")
                .show(ui, |ui| {
                    egui::Grid::new("comparison_table")
                        .striped(true)
                        .spacing([20., 12.])
                        .min_row_height(30.)
                        .show(ui, |ui| {
                            for title in [
                                "Server",
                                "Valid %",
                                "Median offset · ms",
                                "P95 |offset| · ms",
                                "Median RTT · ms",
                                "Stratum",
                                "Selected",
                            ] {
                                ui.strong(title);
                            }
                            ui.end_row();
                            for (index, s) in report.servers.iter().enumerate() {
                                if ui
                                    .selectable_label(
                                        self.report_peer == s.peer,
                                        egui::RichText::new(&s.peer)
                                            .color(style::ink(ui, colour(index))),
                                    )
                                    .clicked()
                                {
                                    self.report_peer = s.peer.clone();
                                }
                                ui.label(
                                    s.valid_percent
                                        .map(|v| format!("{v:.1}%"))
                                        .unwrap_or_else(|| "—".into()),
                                );
                                ui.monospace(value(s.median_offset_ms));
                                ui.monospace(value(s.p95_abs_offset_ms));
                                ui.monospace(value(s.median_rtt_ms));
                                ui.label(
                                    s.min_stratum
                                        .map(|v| format!("{v}–{}", s.max_stratum.unwrap_or(v)))
                                        .unwrap_or_else(|| "—".into()),
                                );
                                ui.label(duration(s.selected_hours * 3600.));
                                ui.end_row();
                            }
                        });
                });
            ui.weak("P95: 95% of valid measurements were at or below this absolute offset. Selection time excludes expired decisions and gaps.");
        });
        ui.add_space(12.);
        if let Some(source) = report.servers.iter().find(|s| s.peer == self.report_peer) {
            card(ui, |ui| {
                section(
                    ui,
                    &source.peer,
                    "Recorded evidence for the selected report period",
                );
                ui.columns(2, |cols| {
                    egui::Grid::new("report_detail_left")
                        .spacing([20., 10.])
                        .show(&mut cols[0], |ui| {
                            for (label, val) in [
                                (
                                    "Valid / attempted",
                                    format!("{} / {}", source.valid, source.attempts),
                                ),
                                ("Observed span", duration(source.observed_hours * 3600.)),
                                (
                                    "Longest attempt gap",
                                    duration(source.longest_attempt_gap_seconds),
                                ),
                                (
                                    "PC-relative offset variation",
                                    format!("{} ms SD", value(source.offset_stddev_ms)),
                                ),
                            ] {
                                ui.label(label);
                                ui.strong(val);
                                ui.end_row();
                            }
                        });
                    egui::Grid::new("report_detail_right")
                        .spacing([20., 10.])
                        .show(&mut cols[1], |ui| {
                            for (label, val) in [
                                ("P95 round trip", format!("{} ms", value(source.p95_rtt_ms))),
                                ("Stratum changes", source.stratum_changes.to_string()),
                                (
                                    "NTS-authenticated valid responses",
                                    format!("{}%", value(source.authenticated_percent)),
                                ),
                                ("Selection entries", source.selection_entries.to_string()),
                            ] {
                                ui.label(label);
                                ui.strong(val);
                                ui.end_row();
                            }
                        });
                });
                ui.add_space(10.);
                ui.strong("Recorded failures");
                if source.failures.is_empty() {
                    ui.weak("No failed attempts recorded in this period.");
                }
                for (reason, count) in &source.failures {
                    ui.label(format!("{count} attempts · {reason}"));
                }
                ui.add_space(8.);
                ui.strong("Selection explanations");
                if source.selection_reasons.is_empty() {
                    ui.weak("No per-source decision reasons recorded in this period.");
                }
                let mut reasons: Vec<_> = source.selection_reasons.iter().collect();
                reasons.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
                for (reason, count) in reasons.iter().take(8) {
                    ui.label(format!(
                        "{count} decisions · {}",
                        super::dashboard::explain(reason)
                    ));
                }
                if reasons.len() > 8 {
                    egui::CollapsingHeader::new(format!(
                        "All {} recorded reason variants",
                        reasons.len()
                    ))
                    .id_salt("all_source_reasons")
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("reason_variants")
                            .max_height(260.)
                            .show(ui, |ui| {
                                for (reason, count) in &reasons {
                                    ui.label(format!(
                                        "{count} decisions · {}",
                                        super::dashboard::explain(reason)
                                    ));
                                }
                            });
                    });
                }
            });
            ui.add_space(12.);
        }
        card(ui, |ui| {
            section(
                ui,
                "Selection timeline",
                "Most recent 100 changes · the full timeline is included in report exports",
            );
            if report.selection_changes.is_empty() {
                ui.weak("No selection changes recorded in this period.");
            }
            for (index, change) in report.selection_changes.iter().rev().take(100).enumerate() {
                egui::CollapsingHeader::new(format!(
                    "{}  ·  {} → {}",
                    crate::windows::utc_label(change.utc),
                    change.previous.as_deref().unwrap_or("No source"),
                    change.selected.as_deref().unwrap_or("No source")
                ))
                .id_salt(("selection_change", index))
                .default_open(index < 3)
                .show(ui, |ui| {
                    ui.label(&change.reason);
                });
            }
            egui::CollapsingHeader::new("How to read this report").show(ui, |ui| {
                for note in &report.notes {
                    ui.label(note);
                }
            });
        });
    }
}
