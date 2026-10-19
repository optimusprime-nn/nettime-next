mod dashboard;
mod panels;
mod reports;
mod settings;
mod style;
mod tray;
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
    actions: Arc<Mutex<Vec<tray::Action>>>,
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
    settings_config: Config,
    settings_tab: usize,
    source_sort: usize,
    history_hours: f64,
    zoom: f32,
    native_service_state: String,
    dirty_settings: bool,
    reload_after_operation: bool,
    exiting: bool,
}
pub fn run(
    database: PathBuf,
    config_path: PathBuf,
    smoke: bool,
    screen: String,
    software: bool,
    tray_test: Option<PathBuf>,
) -> Result<()> {
    // Windows' built-in Direct3D software adapter also works without an OpenGL
    // driver (server sessions, remote desktops and disposable test machines).
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::default();
    setup.instance_descriptor.backends = wgpu::Backends::DX12;
    setup
        .instance_descriptor
        .backend_options
        .dx12
        .shader_compiler = wgpu::Dx12Compiler::Fxc;
    setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
        let chosen = adapters
            .iter()
            .filter(|adapter| surface.is_none_or(|s| adapter.is_surface_supported(s)))
            .filter(|adapter| !software || adapter.get_info().device_type == wgpu::DeviceType::Cpu)
            .min_by_key(|adapter| match adapter.get_info().device_type {
                wgpu::DeviceType::IntegratedGpu => 0,
                wgpu::DeviceType::DiscreteGpu => 1,
                wgpu::DeviceType::VirtualGpu => 2,
                wgpu::DeviceType::Cpu => 3,
                _ => 4,
            })
            .ok_or_else(|| "No compatible Windows graphics adapter is available".to_string())?;
        eprintln!(
            "Desktop renderer: {} ({:?})",
            chosen.get_info().name,
            chosen.get_info().device_type
        );
        Ok(chosen.clone())
    }));
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: setup.into(),
            ..Default::default()
        },
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(if std::env::args().any(|a| a == "--compact") {
                [1000., 720.]
            } else {
                [1440., 1000.]
            })
            .with_min_inner_size([900., 600.]),
        ..Default::default()
    };
    eframe::run_native(
        &format!("NetTime Next {} · Rust", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(move |cc| {
            let appearance = style::Preferences::load();
            style::apply(&cc.egui_ctx, appearance.dark);
            style::install_fonts(&cc.egui_ctx);
            cc.egui_ctx.set_zoom_factor(appearance.zoom);
            let actions = Arc::new(Mutex::new(vec![]));
            let control = tray::Control {
                window: tray::Window::from_context(cc)?,
                queue: actions.clone(),
                context: cc.egui_ctx.clone(),
            };
            let click_control = control.clone();
            TrayIconEvent::set_event_handler(Some(move |event| {
                if tray::opens(&event) {
                    click_control.request(tray::Action::Open);
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
            let menu_control = control.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(index) = ids.iter().position(|id| *id == event.id) {
                    menu_control.request(
                        [
                            tray::Action::Open,
                            tray::Action::Settings,
                            tray::Action::Exit,
                        ][index],
                    );
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
                    .with_menu_on_left_click(false)
                    .build()
                    .ok()
            });
            if let Some(destination) = tray_test {
                if tray.is_none() {
                    return Err(anyhow::anyhow!("A working Windows notification area is required for the tray lifecycle test").into());
                }
                tray::test_lifecycle(control, destination);
            }
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
                settings_config: Config::load(&crate::service::root().join("settings.json"))
                    .unwrap_or_else(|_| cfg.clone()),
                config: cfg,
                decision: None,
                error: String::new(),
                selected: String::new(),
                history: vec![],
                last: Instant::now() - Duration::from_secs(5),
                settings: ["settings", "collection", "clock", "service", "appearance"]
                    .contains(&screen.as_str()),
                dark: appearance.dark,
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
                settings_tab: match screen.as_str() {
                    "collection" => 1,
                    "clock" => 2,
                    "service" => 3,
                    "appearance" => 4,
                    _ => 0,
                },
                source_sort: 0,
                history_hours: 24.,
                zoom: appearance.zoom,
                native_service_state: "Checking service…".into(),
                dirty_settings: false,
                reload_after_operation: false,
                exiting: false,
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
        if let Some(d) = &self.decision {
            let peers = dashboard::rows(d);
            if !peers.iter().any(|(key, _)| *key == self.selected) {
                self.selected = d["selected"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| peers.first().map(|(key, _)| key.clone()))
                    .unwrap_or_default();
            }
        }
        self.native_service_state = crate::ownership::WindowsServices
            .snapshot(crate::service::NAME)
            .map(|s| {
                if !s.exists {
                    "Rust service not installed"
                } else if s.running {
                    "Rust service running"
                } else {
                    "Rust service stopped"
                }
                .to_string()
            })
            .unwrap_or_else(|_| "Rust service status unavailable".into());
        self.load_history();
    }
    fn load_history(&mut self) {
        self.history.clear();
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
                    tray::Action::Open => show = true,
                    tray::Action::Settings => {
                        show = true;
                        open_settings = true
                    }
                    tray::Action::Exit => {
                        self.exiting = true;
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
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
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested && self.tray.is_some() && !self.exiting && !self.smoke {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }
        if !show
            && !self.exiting
            && !self.smoke
            && self.tray.is_some()
            && (close_requested || ctx.input(|i| i.viewport().minimized.unwrap_or(false)))
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
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.small(&self.native_service_state);
                ui.separator();
                ui.small("Collection continues when this window is closed");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(format!("v{}", env!("CARGO_PKG_VERSION")));
                });
            });
        });
        egui::SidePanel::left("navigation")
            .exact_width(184.)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(18.);
                ui.label(
                    egui::RichText::new("NETTIME")
                        .size(23.)
                        .strong()
                        .color(style::ink(ui, style::BLUE)),
                );
                ui.label(
                    egui::RichText::new("N E X T")
                        .size(12.)
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(25.);
                for (page, label) in [(0, "Overview"), (1, "Source comparison"), (2, "Settings")] {
                    let active = if self.settings {
                        2
                    } else {
                        usize::from(self.comparison)
                    };
                    if ui
                        .add_sized([160., 40.], egui::Button::selectable(active == page, label))
                        .clicked()
                    {
                        match page {
                            2 => self.open_settings(),
                            1 => {
                                self.settings = false;
                                self.comparison = true;
                                self.request_report();
                            }
                            _ => {
                                self.settings = false;
                                self.comparison = false;
                            }
                        }
                    }
                }
                ui.add_space(25.);
                ui.separator();
                ui.add_space(8.);
                ui.weak("HISTORY SOURCE");
                let native = self.database.starts_with(crate::service::root());
                if ui
                    .add_sized(
                        [160., 32.],
                        egui::Button::selectable(native, "Rust service"),
                    )
                    .clicked()
                {
                    self.change_source(true);
                }
                if ui
                    .add_sized(
                        [160., 32.],
                        egui::Button::selectable(!native, "Previous service"),
                    )
                    .clicked()
                {
                    self.change_source(false);
                }
                ui.add_space(10.);
                ui.small("Switch to compare the two services’ recorded evidence.");
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(12.);
                    if ui.checkbox(&mut self.dark, "Dark appearance").changed() {
                        self.save_appearance(ctx);
                    }
                    if ui.button("Refresh measurements").clicked() {
                        self.refresh();
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).inner_margin(22))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(if self.settings {
                        "Settings"
                    } else if self.comparison {
                        "Source comparison"
                    } else {
                        "Clock overview"
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        style::badge(
                            ui,
                            if self.database.starts_with(crate::service::root()) {
                                "RUST SERVICE"
                            } else {
                                "PREVIOUS SERVICE"
                            },
                            style::BLUE,
                        );
                    });
                });
                ui.add_space(8.);
                if !self.error.is_empty() {
                    style::card(ui, |ui| {
                        ui.colored_label(style::AMBER, &self.error);
                    });
                }
                if !self.message.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        if self.operation.is_some() {
                            ui.spinner();
                        }
                        ui.label(&self.message);
                        if self.operation.is_none() && ui.small_button("Dismiss").clicked() {
                            self.message.clear();
                        }
                    });
                    ui.add_space(6.);
                }
                egui::ScrollArea::vertical()
                    .id_salt(if self.settings {
                        "settings_page"
                    } else if self.comparison {
                        "comparison_page"
                    } else {
                        "overview_page"
                    })
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if self.settings {
                            self.settings_page(ui, ctx);
                        } else if self.comparison {
                            self.comparison_panel(ui);
                        } else {
                            self.dashboard(ui);
                        }
                    });
            });
        if self.smoke {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}
