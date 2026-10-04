use super::*;

pub(super) const BG: Color32 = Color32::from_rgb(12, 14, 17);
pub(super) const PANEL: Color32 = Color32::from_rgb(17, 19, 23);
pub(super) const LINE: Color32 = Color32::from_rgb(29, 33, 40);
pub(super) const TEXT: Color32 = Color32::from_rgb(215, 222, 232);
pub(super) const MUTED: Color32 = Color32::from_rgb(112, 121, 139);
pub(super) const ACCENT: Color32 = Color32::from_rgb(74, 225, 118);
pub(super) const CYAN: Color32 = Color32::from_rgb(56, 189, 248);
pub(super) const ROSE: Color32 = Color32::from_rgb(243, 134, 161);

pub(super) fn apply(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes, family) in [
        (
            "JetBrains Mono",
            include_bytes!("../assets/fonts/JetBrainsMono.ttf").as_slice(),
            egui::FontFamily::Monospace,
        ),
        (
            "Space Grotesk",
            include_bytes!("../assets/fonts/SpaceGrotesk.ttf").as_slice(),
            egui::FontFamily::Proportional,
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
        fonts
            .families
            .get_mut(&family)
            .unwrap()
            .insert(0, name.into());
    }
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = BG;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = Color32::from_rgb(23, 38, 45);
    style.visuals.selection.stroke = Stroke::new(1.0_f32, CYAN);
    style.visuals.faint_bg_color = Color32::from_rgb(21, 24, 29);
    style.visuals.window_stroke = Stroke::new(1.0_f32, LINE);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(0.5_f32, LINE);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(26, 28, 33);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(21, 24, 29);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(0.5_f32, LINE);
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(3);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(32, 40, 48);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(0.5_f32, CYAN.gamma_multiply(0.45));
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(29, 51, 50);
    style.visuals.widgets.active.bg_stroke = Stroke::new(0.5_f32, ACCENT);
    style.visuals.widgets.open.bg_fill = Color32::from_rgb(26, 30, 36);
    style.spacing.item_spacing = Vec2::new(7.0, 5.0);
    style.spacing.button_padding = Vec2::new(7.0, 3.0);
    style.spacing.interact_size = Vec2::new(36.0, 20.0);
    style.spacing.slider_width = 88.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::monospace(9.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(11.0));
    ctx.set_style(style);
}

pub(super) fn track_color(color: [u8; 3]) -> Color32 {
    // Preserve the project's palette, with more light in clip outlines and waveforms.
    Color32::from_rgb(
        color[0].saturating_add(22),
        color[1].saturating_add(25),
        color[2].saturating_add(28),
    )
}
