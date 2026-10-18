mod panels;
use crate::{
    comparison::{self, Report},
    ownership::Services,
};
use crate::{
    config::{Config, Server},
    storage::Store,
};
use anyhow::Result;
use eframe::egui::{self, ViewportCommand};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
struct Desktop {
    database: PathBuf,
    config: Config,
    decision: Option<Value>,
    error: String,
    selected: String,
    history: Vec<[f64; 2]>,
    last: Instant,
    settings: bool,
    dark: bool,
    tray: Option<TrayIcon>,
    _open: MenuItem,
    _settings_item: MenuItem,
    _exit: MenuItem,
    actions: Arc<Mutex<Vec<u8>>>,
    hidden: bool,
    smoke: bool,
    config_path: PathBuf,
    comparison: bool,
    report: Option<Report>,
    report_days: f64,
    report_worker: Option<std::sync::mpsc::Receiver<std::result::Result<Report, String>>>,
    operation: Option<std::sync::mpsc::Receiver<std::result::Result<String, String>>>,
    message: String,
    report_peer: String,
}
pub fn run(database: PathBuf, config_path: PathBuf, smoke: bool, screen: String) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320., 850.])
            .with_min_inner_size([900., 600.]),
        ..Default::default()
    };
    eframe::run_native(
        &format!("NetTime Next {} · Rust", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            cc.egui_ctx
                .send_viewport_cmd(ViewportCommand::SetTheme(egui::SystemTheme::Dark));
            let actions = Arc::new(Mutex::new(vec![]));
            let queue = actions.clone();
            let context = cc.egui_ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event| {
                if matches!(
                    event,
                    TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } | TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    if let Ok(mut q) = queue.lock() {
                        q.push(1);
                    }
                    context.send_viewport_cmd(ViewportCommand::Visible(true));
                    context.send_viewport_cmd(ViewportCommand::Minimized(false));
                    context.request_repaint();
                }
            }));
            let open = MenuItem::new("Open NetTime Next", true, None);
            let settings_item = MenuItem::new("Settings", true, None);
            let exit = MenuItem::new("Exit desktop app", true, None);
            let ids = [
                open.id().clone(),
                settings_item.id().clone(),
                exit.id().clone(),
            ];
            let queue = actions.clone();
            let context = cc.egui_ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(index) = ids.iter().position(|id| *id == event.id) {
                    if let Ok(mut q) = queue.lock() {
                        q.push(index as u8 + 1);
                    }
                    if index != 2 {
                        context.send_viewport_cmd(ViewportCommand::Visible(true));
                        context.send_viewport_cmd(ViewportCommand::Minimized(false));
                    } else {
                        context.send_viewport_cmd(ViewportCommand::Close);
                    }
                    context.request_repaint();
                }
            }));
            let menu = Menu::new();
            let _ = menu.append_items(&[&open, &settings_item, &exit]);
            let icon = tray_icon::Icon::from_rgba(icon_pixels(), 32, 32).ok();
            let tray = icon.and_then(|icon| {
                TrayIconBuilder::new()
                    .with_tooltip("NetTime Next · Rust preview")
                    .with_icon(icon)
                    .with_menu(Box::new(menu))
                    .build()
                    .ok()
            });
            let (cfg, configuration_error) = match Config::load(&config_path) {
                Ok(cfg) => (cfg, String::new()),
                Err(error) => (
                    Config::default(),
                    format!("Settings could not be loaded: {error:#}"),
                ),
            };
            let report = if smoke && screen == "comparison" {
                Store::open(&database, true)
                    .and_then(|s| comparison::build(&s, 7., crate::protocol::utc()))
                    .ok()
            } else {
                None
            };
            Ok(Box::new(Desktop {
                database,
                config: cfg,
                decision: None,
                error: String::new(),
                selected: String::new(),
                history: vec![],
                last: Instant::now() - Duration::from_secs(5),
                settings: screen == "settings",
                dark: true,
                tray,
                _open: open,
                _settings_item: settings_item,
                _exit: exit,
                actions,
                hidden: false,
                smoke,
                config_path,
                comparison: screen == "comparison",
                report,
                report_days: 7.,
                report_worker: None,
                operation: None,
                message: configuration_error,
                report_peer: String::new(),
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Desktop: {e}"))
}
fn icon_pixels() -> Vec<u8> {
    let mut rgba = vec![0; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let dx = x as f64 - 15.5;
            let dy = y as f64 - 15.5;
            let r = dx.hypot(dy);
            let p = (y * 32 + x) * 4;
            let colour = if r < 14. && r > 11.5 {
                [83, 173, 255, 255]
            } else if r < 11.5
                && ((dx.abs() < 1. && dy < 1. && dy > -8.)
                    || (dy.abs() < 1. && dx > -1. && dx < 7.))
            {
                [102, 235, 192, 255]
            } else if r < 15. {
                [21, 30, 46, 255]
            } else {
                [0, 0, 0, 0]
            };
            rgba[p..p + 4].copy_from_slice(&colour);
        }
    }
    rgba
}
impl Desktop {
    fn refresh(&mut self) {
        match Store::open(&self.database, true).and_then(|s| s.latest()) {
            Ok(d) => {
                self.decision = d;
                self.error.clear();
            }
            Err(e) => self.error = e.to_string(),
        };
        self.load_history();
    }
    fn load_history(&mut self) {
        if let Some(s) = self
            .config
            .servers
            .iter()
            .find(|s| s.key() == self.selected)
        {
            self.history = Store::open(&self.database, true)
                .and_then(|db| db.history(&s.host, s.port))
                .unwrap_or_default();
        }
    }
}
fn number(v: &Value, key: &str) -> String {
    v[key]
        .as_f64()
        .map(|n| format!("{n:.3}"))
        .unwrap_or_else(|| "—".into())
}
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_operations();
        let mut show = false;
        let mut open_settings = false;
        if let Ok(mut q) = self.actions.lock() {
            for action in q.drain(..) {
                match action {
                    1 => show = true,
                    2 => {
                        show = true;
                        open_settings = true
                    }
                    3 => ctx.send_viewport_cmd(ViewportCommand::Close),
                    _ => {}
                }
            }
        }
        if open_settings {
            self.open_settings();
        }
        if show {
            self.hidden = false;
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        if !show
            && ctx.input(|i| i.viewport().minimized.unwrap_or(false))
            && self.tray.is_some()
            && !self.hidden
        {
            self.hidden = true;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        if self.last.elapsed() > Duration::from_secs(2) {
            self.refresh();
            self.last = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(10.);
            ui.horizontal(|ui| {
                ui.heading("NetTime Next");
                ui.label("Rust preview");
                if ui.button("Settings").clicked() {
                    self.open_settings();
                }
                if ui.checkbox(&mut self.dark, "Dark mode").changed() {
                    ctx.set_visuals(if self.dark {
                        egui::Visuals::dark()
                    } else {
                        egui::Visuals::light()
                    });
                    ctx.send_viewport_cmd(ViewportCommand::SetTheme(if self.dark {
                        egui::SystemTheme::Dark
                    } else {
                        egui::SystemTheme::Light
                    }));
                }
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }
            });
            ui.label("Windows service continues while this desktop window is closed.");
            ui.label(format!(
                "Viewing: {}",
                if self.database.starts_with(crate::service::root()) {
                    "Native Rust service"
                } else {
                    "Current NetTime Next service"
                }
            ));
            ui.horizontal(|ui| {
                if ui.button("View native service").clicked() {
                    self.change_source(true);
                }
                if ui.button("View current service").clicked() {
                    self.change_source(false);
                }
                ui.separator();
                if ui
                    .selectable_label(!self.comparison, "Live status")
                    .clicked()
                {
                    self.comparison = false;
                }
                if ui
                    .selectable_label(self.comparison, "Source comparison")
                    .clicked()
                {
                    self.comparison = true;
                    self.request_report();
                }
            });
            ui.add_space(8.);
        });
        egui::CentralPanel::default().show(ctx,|ui|{
            if !self.error.is_empty(){ui.colored_label(egui::Color32::LIGHT_YELLOW,&self.error);}
            if !self.message.is_empty(){ui.label(&self.message);}
            if self.comparison {self.comparison_panel(ui);return;}
            if let Some(d)=&self.decision {
                ui.strong(d["synchronization"]["explanation"].as_str().unwrap_or_else(||d["explanation"].as_str().unwrap_or("Collecting measurements")));
                let rows:Vec<(String,Value)>=if let Some(servers)=d["servers"].as_object(){servers.iter().map(|(k,v)|(k.clone(),v.clone())).collect()}else if let Some(servers)=d["servers"].as_array(){servers.iter().map(|s|{let mut latest=s["latest"].clone();if let Some(v)=latest.as_object_mut(){v.insert("reach".into(),s["reach_octal"].clone());}(s["peer"].as_str().unwrap_or("").into(),latest)}).collect()}else{vec![]};
                let mut selected=None;egui::ScrollArea::horizontal().show(ui,|ui|{egui::Grid::new("server_table").striped(true).min_col_width(70.).show(ui,|ui|{
                    for title in ["Server","Status","Stratum","Offset ms","RTT ms","Reach","Reference","Security"]{ui.strong(title);}ui.end_row();
                    for (key,s) in rows {if ui.selectable_label(self.selected==key,&key).clicked(){selected=Some(key.clone());}ui.label(s["status"].as_str().unwrap_or("—"));ui.label(s["stratum"].to_string());ui.label(number(&s,"offset_ms"));ui.label(number(&s,"delay_ms"));ui.label(s["reach"].as_str().unwrap_or("—"));ui.label(s["reference_id"].as_str().unwrap_or("—"));ui.label(s["authentication_status"].as_str().unwrap_or("Unauthenticated"));ui.end_row();}
                });});if let Some(s)=selected{self.selected=s;self.load_history();}
                ui.separator();ui.strong(format!("{} · retained history",if self.selected.is_empty(){"Select a server"}else{&self.selected}));
                if !self.history.is_empty(){use egui_plot::{Plot,Line,PlotPoints};let points:PlotPoints=self.history.iter().map(|p|[p[0],p[1]]).collect();Plot::new("offset_history").height(230.).y_axis_label("Offset (ms)").x_axis_label("UTC (Unix seconds)").show(ui,|plot|{plot.line(Line::new("Offset",points));});}
                egui::CollapsingHeader::new("Selection and clock evidence").default_open(true).show(ui,|ui|{egui::ScrollArea::vertical().max_height(240.).show(ui,|ui|{if let Some(d)=&self.decision{ui.monospace(serde_json::to_string_pretty(&d["synchronization"]).unwrap_or_default());ui.monospace(serde_json::to_string_pretty(&d["clock_action"]).unwrap_or_default());}});});
            }else{ui.label("Waiting for service history. The service and settings can be installed independently of this window.");}
        });
        if self.settings {
            self.settings_panel(ctx)
        }
        if self.smoke {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}
