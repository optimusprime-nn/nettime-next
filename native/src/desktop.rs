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
}
pub fn run(database: PathBuf, config_path: PathBuf, smoke: bool) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320., 850.])
            .with_min_inner_size([900., 600.]),
        ..Default::default()
    };
    eframe::run_native(
        "NetTime Next · Rust preview",
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
            let cfg = Config::load(&config_path).unwrap_or_default();
            Ok(Box::new(Desktop {
                database,
                config: cfg,
                decision: None,
                error: String::new(),
                selected: String::new(),
                history: vec![],
                last: Instant::now() - Duration::from_secs(5),
                settings: false,
                dark: true,
                tray,
                _open: open,
                _settings_item: settings_item,
                _exit: exit,
                actions,
                hidden: false,
                smoke,
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
    fn settings(&mut self, ctx: &egui::Context) {
        let mut visible = self.settings;
        egui::Window::new("Settings").open(&mut visible).default_width(900.).show(ctx,|ui|{
        ui.label("Source settings draft · the running clock service remains in control during the Rust migration.");
        egui::Grid::new("settings_sources").striped(true).show(ui,|ui|{
            for title in ["Server","Port","Security","Independent source group",""]{ui.strong(title);}ui.end_row();
            let mut remove=None;for (i,s) in self.config.servers.iter_mut().enumerate(){ui.add(egui::TextEdit::singleline(&mut s.host).desired_width(190.));ui.add(egui::DragValue::new(&mut s.port).range(1..=65535));
                egui::ComboBox::from_id_salt(("security",i)).selected_text(if s.security=="nts_required"{"NTS required"}else{"Plain NTP"}).show_ui(ui,|ui|{ui.selectable_value(&mut s.security,"plain_ntp".into(),"Plain NTP");ui.selectable_value(&mut s.security,"nts_required".into(),"NTS required");});
                ui.add(egui::TextEdit::singleline(&mut s.source_group).desired_width(140.));if ui.button("Remove").clicked(){remove=Some(i);}ui.end_row();
            }if let Some(i)=remove{self.config.servers.remove(i);}
        });
        if self.config.servers.len()<32 && ui.button("Add server").clicked(){self.config.servers.push(Server::default());}
        ui.separator();egui::Grid::new("options").show(ui,|ui|{ui.label("Check every (seconds)");ui.add(egui::DragValue::new(&mut self.config.poll_seconds).range(64. ..=131072.));ui.end_row();ui.label("Keep history (days)");ui.add(egui::DragValue::new(&mut self.config.retention_days).range(7. ..=3650.));ui.end_row();ui.label("Response timeout (seconds)");ui.add(egui::DragValue::new(&mut self.config.timeout_seconds).range(0.1..=30.));ui.end_row();});
        ui.label("Every configured server is checked. Servers sharing a source group are measured separately and count as one timing vote.");
        if ui.button("Save settings draft").clicked(){match self.config.validate().and_then(|_|{let mut draft=self.config.clone();draft.clock.mode="observe".into();let folder=PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(||".".into())).join("NetTimeNextNativePreview");std::fs::create_dir_all(&folder)?;std::fs::write(folder.join("settings-rust-draft.json"),serde_json::to_vec_pretty(&draft)?)?;Ok(())}){Ok(())=>self.error="Settings draft saved; running service settings unchanged.".into(),Err(e)=>self.error=e.to_string()}}
    });
        self.settings = visible;
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
        let mut show = false;
        if let Ok(mut q) = self.actions.lock() {
            for action in q.drain(..) {
                match action {
                    1 => show = true,
                    2 => {
                        show = true;
                        self.settings = true
                    }
                    3 => ctx.send_viewport_cmd(ViewportCommand::Close),
                    _ => {}
                }
            }
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
        egui::TopBottomPanel::top("header").show(ctx,|ui|{ui.add_space(10.);ui.horizontal(|ui|{ui.heading("NetTime Next");ui.label("Rust preview");if ui.button("Settings").clicked(){self.settings=true;}if ui.checkbox(&mut self.dark,"Dark mode").changed(){ctx.set_visuals(if self.dark{egui::Visuals::dark()}else{egui::Visuals::light()});ctx.send_viewport_cmd(ViewportCommand::SetTheme(if self.dark{egui::SystemTheme::Dark}else{egui::SystemTheme::Light}));}if ui.button("Refresh").clicked(){self.refresh();}});
            ui.label("Windows service continues while this desktop window is closed.");ui.label("Native desktop preview · reading the current service’s evidence. Clock ownership stays with the tested service.");ui.add_space(8.);});
        egui::CentralPanel::default().show(ctx,|ui|{
            if !self.error.is_empty(){ui.colored_label(egui::Color32::LIGHT_YELLOW,&self.error);}
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
            self.settings(ctx)
        }
        if self.smoke {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}
