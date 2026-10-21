use super::*;
use style::{AMBER, BLUE, GREEN, badge, card, duration, section, summary};

// Normalize the two retained-history formats at the display boundary.
pub(super) fn rows(d: &Value) -> Vec<(String, Value)> {
    if let Some(servers) = d["servers"].as_object() {
        servers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    } else if let Some(servers) = d["servers"].as_array() {
        servers
            .iter()
            .map(|s| {
                let mut latest = s["latest"].clone();
                if let Some(v) = latest.as_object_mut() {
                    v.insert("reach".into(), s["reach_octal"].clone());
                    if let Some(next) = s.get("next_check_seconds") {
                        v.insert("next_check_seconds".into(), next.clone());
                    } else if let (Some(next), Some(stamp)) =
                        (s["next_check_utc"].as_f64(), d["utc"].as_f64())
                    {
                        v.insert(
                            "next_check_seconds".into(),
                            serde_json::json!((next - stamp).max(0.)),
                        );
                    }
                    v.insert("selection_reason".into(), s["reason"].clone());
                }
                (s["peer"].as_str().unwrap_or("").into(), latest)
            })
            .collect()
    } else {
        vec![]
    }
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("—")
}
fn nullable(v: &Value, key: &str) -> String {
    v[key]
        .as_u64()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".into())
}
pub(super) fn explain(reason: &str) -> String {
    if reason.starts_with("System peer;") || reason.starts_with("Combined survivor;") {
        let description = if reason.starts_with("System peer;") {
            "Chosen from the agreeing sources using stratum and estimated distance."
        } else {
            "Agrees with the other accepted clocks and contributes to the combined estimate."
        };
        if let Some(weight) = reason
            .split("weight ")
            .nth(1)
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && (0. ..=1.).contains(v))
        {
            return format!(
                "{description} Its contribution to the combined estimate is {:.1}%.",
                weight * 100.
            );
        }
        return description.into();
    }
    match reason {
        "Same resolved endpoint; no additional vote" => "Measured, but resolves to an endpoint already counted; it does not add another vote.",
        "Same source group; measured but no additional vote" => "Measured, but another endpoint in the same configured source group already contributes a vote.",
        "Assign independent source group" => "Not eligible for clock agreement until an independent source group is assigned in Settings.",
        "Filtered candidate" => "Has enough valid filtered measurements to be considered for source agreement.",
        "Falseticker: midpoint outside majority interval" => "Excluded because its filtered offset falls outside the range supported by the majority of clocks.",
        "Cluster outlier: largest selection jitter" => "Excluded during clustering because its filtered offset differs most from the other sources.",
        "Clock policy requires NTS" => "Excluded from clock agreement because the clock policy requires NTS authentication.",
        _ => reason,
    }.into()
}
impl Desktop {
    pub(super) fn dashboard(&mut self, ui: &mut egui::Ui) {
        let Some(d) = self.decision.clone() else {
            card(ui, |ui| {
                section(
                    ui,
                    "Waiting for measurements",
                    "The dashboard will fill as soon as the selected service records a check.",
                );
                ui.label("Open Settings → Windows service to check or start the Rust collector.");
            });
            return;
        };
        let sync = &d["synchronization"];
        let selected = d["selected"].as_str().unwrap_or("");
        let mut peers = rows(&d);
        // Include configured endpoints even while their first checks are pending.
        for server in &self.config.servers {
            if !peers.iter().any(|(key, _)| *key == server.key()) {
                peers.push((server.key(), Value::Null));
            }
        }
        let age = d["utc"]
            .as_f64()
            .map(|t| (crate::protocol::utc() - t).max(0.));
        let stale = age.is_none_or(|a| a > self.config.poll_seconds * 3.);
        let ready = sync["ready"].as_bool().unwrap_or(false) && !stale;
        let mode = d["clock_action"]["action"].as_str().unwrap_or("unknown");
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                badge(
                    ui,
                    if stale {
                        "STALE MEASUREMENTS"
                    } else if ready {
                        "SOURCES AGREE"
                    } else {
                        "CHECKING AGREEMENT"
                    },
                    if ready { GREEN } else { AMBER },
                );
                badge(
                    ui,
                    if mode == "observe" {
                        "MEASUREMENT ONLY"
                    } else if stale {
                        "CLOCK STATE NEEDS CHECKING"
                    } else {
                        match mode {
                            "slew" => "GRADUAL CLOCK CORRECTION",
                            "hold" => "WAITING FOR FRESH SAMPLE",
                            "warmup" => "CLOCK FILTER SETTLING",
                            "suspend" => "CORRECTION SUSPENDED",
                            _ => "CLOCK STATE NEEDS CHECKING",
                        }
                    },
                    BLUE,
                );
            });
            ui.add_space(4.);
            ui.label(
                egui::RichText::new(if selected.is_empty() {
                    "No source selected".to_string()
                } else {
                    format!("Recommended source  {selected}")
                })
                .size(22.)
                .strong(),
            );
            ui.label(
                sync["explanation"]
                    .as_str()
                    .or_else(|| d["explanation"].as_str())
                    .unwrap_or("Collecting measurements from every configured server."),
            );
            if stale {
                ui.colored_label(style::ink(ui, AMBER), "This is retained evidence, not a current clock-health assessment. Check the service below.");
            } else if mode == "observe" {
                ui.weak("The Rust service is observing your clock. Existing time services remain responsible for adjustment.");
            }
        });
        ui.add_space(8.);
        summary(
            ui,
            [
                (
                    "COMBINED OFFSET",
                    format!("{} ms", number(sync, "offset_ms")),
                    "Filtered source estimate".into(),
                ),
                (
                    "VALID RESPONSES",
                    format!(
                        "{} / {}",
                        peers.iter().filter(|(_, s)| s["status"] == "valid").count(),
                        peers.len()
                    ),
                    "Latest recorded check per source".into(),
                ),
                (
                    "LAST DECISION",
                    age.map(duration).unwrap_or_else(|| "—".into()),
                    "Age of recorded evidence".into(),
                ),
                (
                    "HISTORY RETENTION",
                    format!("{:.0} days", self.config.retention_days),
                    "Measurements and selection reasons".into(),
                ),
            ],
        );
        ui.add_space(12.);
        card(ui, |ui| {
            section(
                ui,
                "Time sources",
                "Every configured server is measured. Select a row to inspect its history and selection reason.",
            );
            ui.horizontal(|ui| {
                ui.label("Sort by");
                egui::ComboBox::from_id_salt("source_order")
                    .selected_text(
                        [
                            "Configured order",
                            "Server name",
                            "Lowest round trip",
                            "Smallest offset",
                        ][self.source_sort],
                    )
                    .show_ui(ui, |ui| {
                        for (i, name) in [
                            "Configured order",
                            "Server name",
                            "Lowest round trip",
                            "Smallest offset",
                        ]
                        .iter()
                        .enumerate()
                        {
                            ui.selectable_value(&mut self.source_sort, i, *name);
                        }
                    });
            });
            peers.sort_by(|a, b| {
                if self.source_sort == 0 {
                    let position = |key: &str| {
                        self.config
                            .servers
                            .iter()
                            .position(|s| s.key() == key)
                            .unwrap_or(usize::MAX)
                    };
                    return position(&a.0)
                        .cmp(&position(&b.0))
                        .then_with(|| a.0.cmp(&b.0));
                }
                if self.source_sort == 1 {
                    return a.0.cmp(&b.0);
                }
                let key = if self.source_sort == 2 {
                    "delay_ms"
                } else {
                    "offset_ms"
                };
                let value = |v: &Value| {
                    v[key]
                        .as_f64()
                        .map(|n| if self.source_sort == 3 { n.abs() } else { n })
                        .unwrap_or(f64::INFINITY)
                };
                value(&a.1)
                    .total_cmp(&value(&b.1))
                    .then_with(|| a.0.cmp(&b.0))
            });
            let mut clicked = None;
            egui::ScrollArea::horizontal().id_salt("source_table_scroll").show(ui, |ui| {
                egui::Grid::new("server_table").striped(true).spacing([18., 10.]).min_row_height(32.).show(ui, |ui| {
                    for title in ["Server", "Response", "Stratum", "Offset · ms", "Round trip · ms", "Reach", "Security", "Next check"] { ui.strong(title); } ui.end_row();
                    for (key, s) in &peers {
                        let is_selected = self.selected == *key;
                        let title = if key == selected { format!("* {key}") } else { format!("    {key}") };
                        if ui.selectable_label(is_selected, title).on_hover_text("View this source’s retained history and decision evidence").clicked() { clicked = Some(key.clone()); }
                        let valid = s["status"] == "valid";
                        ui.colored_label(style::ink(ui, if valid { GREEN } else { AMBER }), if s.is_null() { "Pending" } else { text(s, "status") }).on_hover_text(text(s, "reason"));
                        ui.label(nullable(s, "stratum"));
                        ui.monospace(number(s, "offset_ms")); ui.monospace(number(s, "delay_ms"));
                        ui.label(text(s, "reach")).on_hover_text("Octal bitmap of the last eight attempted checks. 377 means eight responses.");
                        let auth = text(s, "authentication_status");
                        ui.label(if auth == "—" || auth.is_empty() { "Unauthenticated" } else { auth });
                        let next = s["next_check_seconds"].as_f64().map(|t| t - age.unwrap_or(0.));
                        ui.label(if stale { "—".into() } else { next.map(|t| if t <= 0. { "Due".into() } else { duration(t) }).unwrap_or_else(|| "—".into()) });
                        ui.end_row();
                    }
                });
            });
            ui.weak("* Recommended source · Offset is server time minus this PC’s time. Stratum is the source’s distance in NTP hops from its reference.");
            if let Some(key) = clicked {
                self.selected = key;
                self.load_history();
            }
        });
        ui.add_space(12.);
        let inspected = peers.iter().find(|(k, _)| *k == self.selected);
        card(ui, |ui| {
            section(
                ui,
                if self.selected.is_empty() {
                    "Source history"
                } else {
                    &self.selected
                },
                "Recorded offset relative to this PC · times shown in UTC",
            );
            ui.horizontal_wrapped(|ui| {
                for (hours, label) in [
                    (1., "1 hour"),
                    (24., "24 hours"),
                    (168., "7 days"),
                    (0., "All retained"),
                ] {
                    ui.selectable_value(&mut self.history_hours, hours, label);
                }
            });
            let cutoff = if self.history_hours > 0. {
                crate::protocol::utc() - self.history_hours * 3600.
            } else {
                0.
            };
            let points: Vec<[f64; 2]> = self
                .history
                .iter()
                .copied()
                .filter(|p| p[0] >= cutoff)
                .collect();
            if points.is_empty() {
                ui.add_space(24.);
                ui.weak("No valid measurements in this time range. Select another source or a longer range.");
                ui.add_space(24.);
            } else {
                use egui_plot::{Plot, PlotPoints, Points};
                let plot_colour = style::ink(ui, BLUE);
                Plot::new((
                    "offset_history",
                    &self.selected,
                    self.history_hours.to_bits(),
                ))
                .height(230.)
                .include_y(0.)
                .y_axis_label("Offset (ms)")
                .x_axis_label("UTC")
                .x_axis_formatter(|mark, _| crate::windows::utc_label(mark.value))
                .label_formatter(|name, p| {
                    format!("{name}\n{}\n{:.3} ms", crate::windows::utc_label(p.x), p.y)
                })
                .show(ui, |plot| {
                    plot.points(
                        Points::new("Recorded offset", PlotPoints::from(points))
                            .color(plot_colour)
                            .radius(2_f32),
                    );
                });
                ui.weak("Each point is a recorded response. Missing measurements remain gaps. PC clock corrections affect these offsets.");
            }
            if let Some((key, s)) = inspected {
                ui.separator();
                ui.strong("Why this source is or isn’t selected");
                ui.label(explain(
                    sync["reasons"][key]
                        .as_str()
                        .or_else(|| s["selection_reason"].as_str())
                        .unwrap_or("No selection explanation has been recorded yet."),
                ));
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Reference: {}", text(s, "reference_id")));
                    ui.label(format!("Leap indicator: {}", nullable(s, "leap")));
                    ui.label(format!("Upstream delay: {} ms", number(s, "root_delay_ms")));
                    ui.label(format!(
                        "Root dispersion: {} ms",
                        number(s, "root_dispersion_ms")
                    ));
                });
                if let Some(reason) = s["reason"].as_str().filter(|r| !r.is_empty()) {
                    ui.label(format!("Latest response detail: {reason}"));
                }
            }
        });
        ui.add_space(12.);
        card(ui, |ui| {
            section(
                ui,
                "Clock and selection",
                "What the last recorded decision means",
            );
            ui.label(format!(
                "Clock action: {}",
                match mode {
                    "observe" => "Observing only",
                    "slew" => "Gradual correction",
                    "hold" => "Waiting for a fresh sample; retaining the current correction rate",
                    "warmup" => "Clock filter settling; correction paused",
                    "suspend" => "Correction suspended",
                    other => other,
                }
            ));
            ui.label(
                d["clock_action"]["reason"]
                    .as_str()
                    .unwrap_or("No clock action reason recorded."),
            );
            if let Some(rate) = d["clock_action"]["rate_ppm"].as_f64() {
                ui.label(format!("Requested correction rate: {rate:+.3} ppm"));
            }
            ui.label(format!(
                "Agreeing survivors: {} · Candidate sources: {}",
                sync["survivors"].as_array().map_or(0, Vec::len),
                sync["candidates"].as_array().map_or(0, Vec::len)
            ));
            if let Some(candidates) = sync["candidates"].as_array() {
                for c in candidates
                    .iter()
                    .filter(|c| c["peer"].as_str() == Some(&self.selected))
                {
                    ui.label(format!("Inspected source: filtered offset {} ms · variation {} ms · estimated distance {} ms", number(c, "offset_ms"), number(c, "jitter_ms"), number(c, "distance_ms")));
                }
            }
            egui::CollapsingHeader::new("Technical evidence · full recorded decision")
                .default_open(false)
                .show(ui, |ui| {
                    egui::ScrollArea::both()
                        .id_salt("decision_json")
                        .max_height(280.)
                        .show(ui, |ui| {
                            ui.monospace(serde_json::to_string_pretty(&d).unwrap_or_default());
                        });
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn previous_history_keeps_missing_measurements_and_next_check() {
        let records = rows(
            &json!({"utc":100.,"servers":[{"peer":"a:123","reach_octal":"017","next_check_utc":164.,"reason":"Same source group; measured but no additional vote","latest":{"status":"invalid","offset_ms":null}}]}),
        );
        assert_eq!(records[0].0, "a:123");
        assert!(records[0].1["offset_ms"].is_null());
        assert_eq!(records[0].1["reach"], "017");
        assert_eq!(records[0].1["next_check_seconds"], 64.);
        assert!(
            records[0].1["selection_reason"]
                .as_str()
                .unwrap()
                .contains("Same source group")
        );
    }
    #[test]
    fn native_history_is_not_reinterpreted() {
        let sample = json!({"status":"valid","offset_ms":-2.5,"stratum":1,"reach":"377"});
        assert_eq!(
            rows(&json!({"servers":{"a:123":sample.clone()}})),
            vec![("a:123".into(), sample)]
        );
    }
    #[test]
    fn readable_reasons_keep_weight_and_unknown_evidence() {
        assert!(explain("System peer; weight 0.405657").contains("40.6%"));
        assert!(
            explain("Same source group; measured but no additional vote")
                .contains("same configured source group")
        );
        assert_eq!(
            explain("Future diagnostic detail"),
            "Future diagnostic detail"
        );
        assert!(!explain("System peer; weight NaN").contains("NaN"));
    }
}
