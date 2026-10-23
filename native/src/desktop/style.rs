use super::*;
use egui::{Color32, RichText};
use serde::{Deserialize, Serialize};

pub(super) const BLUE: Color32 = Color32::from_rgb(91, 169, 255);
pub(super) const GREEN: Color32 = Color32::from_rgb(60, 204, 164);
pub(super) const AMBER: Color32 = Color32::from_rgb(231, 176, 86);
pub(super) fn ink(ui: &egui::Ui, colour: Color32) -> Color32 {
    if ui.visuals().dark_mode {
        colour
    } else {
        Color32::from_rgb(
            (colour.r() as f32 * 0.55) as u8,
            (colour.g() as f32 * 0.55) as u8,
            (colour.b() as f32 * 0.55) as u8,
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub dark: bool,
    pub zoom: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            dark: true,
            zoom: 1.,
        }
    }
}
impl Preferences {
    fn path() -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("NetTimeNextNativePreview/appearance.json"))
    }
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .map(|mut p| {
                p.zoom = if p.zoom.is_finite() {
                    p.zoom.clamp(0.85, 1.5)
                } else {
                    1.
                };
                p
            })
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<()> {
        let path =
            Self::path().ok_or_else(|| anyhow::anyhow!("Local application data is unavailable"))?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
pub(super) fn apply(ctx: &egui::Context, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark {
        visuals.panel_fill = Color32::from_rgb(15, 21, 31);
        visuals.window_fill = Color32::from_rgb(23, 31, 44);
        visuals.extreme_bg_color = Color32::from_rgb(12, 18, 27);
        visuals.faint_bg_color = Color32::from_rgb(28, 38, 53);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(23, 31, 44);
        visuals.widgets.noninteractive.fg_stroke.color = Color32::from_rgb(221, 229, 242);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(32, 44, 61);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(32, 44, 61);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(43, 63, 87);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(43, 63, 87);
        visuals.widgets.active.bg_fill = Color32::from_rgb(35, 76, 118);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(35, 76, 118);
        visuals.selection.bg_fill = Color32::from_rgb(35, 76, 118);
        visuals.selection.stroke.color = Color32::WHITE;
    }
    ctx.set_visuals(visuals);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10., 8.);
        s.spacing.button_padding = egui::vec2(12., 7.);
        s.spacing.interact_size.y = 30.;
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.));
        s.text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
        s.text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(24.));
        s.text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.));
    });
    ctx.send_viewport_cmd(ViewportCommand::SetTheme(if dark {
        egui::SystemTheme::Dark
    } else {
        egui::SystemTheme::Light
    }));
}
pub(super) fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    let width = (ui.available_width() - 20.).max(0.);
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(egui::Stroke::new(
            1_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(10)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_min_width(width);
            content(ui);
        });
}
pub(super) fn install_fonts(ctx: &egui::Context) {
    if let Some(bytes) = std::env::var_os("WINDIR")
        .and_then(|root| std::fs::read(PathBuf::from(root).join("Fonts/segoeui.ttf")).ok())
    {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Windows UI".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "Windows UI".into());
        ctx.set_fonts(fonts);
    }
}
pub(super) fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(if ui.visuals().dark_mode { 0.15 } else { 0.08 }))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(ink(ui, color)).strong().size(12.));
        });
}
pub(super) fn metric(ui: &mut egui::Ui, label: &str, value: &str, detail: &str) {
    card(ui, |ui| {
        ui.set_min_height(42.);
        ui.set_min_width((ui.available_width() - 2.).max(0.));
        ui.label(
            RichText::new(label)
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.label(RichText::new(value).size(19.).strong())
            .on_hover_text(detail);
    });
}
pub(super) fn summary(ui: &mut egui::Ui, values: [(&str, String, String); 4]) {
    let columns = if ui.available_width() < 440. {
        1
    } else if ui.available_width() < 680. {
        2
    } else {
        4
    };
    for (index, row) in values.chunks(columns).enumerate() {
        if index > 0 {
            ui.add_space(4.);
        }
        ui.columns(columns, |cols| {
            for (col, (label, value, detail)) in cols.iter_mut().zip(row.iter()) {
                metric(col, label, value, detail);
            }
        });
    }
}
pub(super) fn section(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).size(18.).strong());
    ui.label(RichText::new(description).color(ui.visuals().weak_text_color()));
    ui.add_space(6.);
}
pub(super) fn duration(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "—".into();
    }
    let seconds = seconds.max(0.);
    if seconds < 60. {
        format!("{seconds:.0}s")
    } else if seconds < 3600. {
        format!("{:.0}m", seconds / 60.)
    } else if seconds < 86400. {
        format!("{:.1}h", seconds / 3600.)
    } else {
        format!("{:.1}d", seconds / 86400.)
    }
}
