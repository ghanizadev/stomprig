mod effects;
mod preset;
mod tuner;

use super::audio::TunerReadout;
use super::plugin_editor::PluginEffectState;
use effects::show_effects;
use preset::show_preset;
use tuner::show;

const STAGE_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(3, 5, 9);
const PANEL_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(10, 16, 25);
const PANEL_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(16, 27, 41);
const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(245, 250, 255);
const TEXT_SECONDARY: egui::Color32 = egui::Color32::from_rgb(161, 181, 202);
const LIVE_CYAN: egui::Color32 = egui::Color32::from_rgb(92, 229, 255);
const LIVE_GREEN: egui::Color32 = egui::Color32::from_rgb(194, 255, 76);
const LIVE_AMBER: egui::Color32 = egui::Color32::from_rgb(255, 190, 64);
const LIVE_RED: egui::Color32 = egui::Color32::from_rgb(255, 92, 111);

#[derive(Clone, Copy)]
struct LivePalette {
    panel: egui::Color32,
    panel_highlight: egui::Color32,
    text_primary: egui::Color32,
    text_secondary: egui::Color32,
    cyan: egui::Color32,
    green: egui::Color32,
    amber: egui::Color32,
    red: egui::Color32,
}

impl Default for LivePalette {
    fn default() -> Self {
        Self {
            panel: PANEL_BACKGROUND,
            panel_highlight: PANEL_HIGHLIGHT,
            text_primary: TEXT_PRIMARY,
            text_secondary: TEXT_SECONDARY,
            cyan: LIVE_CYAN,
            green: LIVE_GREEN,
            amber: LIVE_AMBER,
            red: LIVE_RED,
        }
    }
}

pub(crate) struct LiveTunerData {
    pub(crate) readout: TunerReadout,
    pub(crate) bypassed: bool,
    pub(crate) a4_pitch_hz: f32,
    pub(crate) transposition_semitones: i32,
}

pub(crate) struct LiveWindowData<'a> {
    pub(crate) current_board_name: Option<&'a str>,
    pub(crate) live_preset_name: Option<&'a str>,
    pub(crate) live_preset_boards: Option<&'a [String]>,
    pub(crate) effects: &'a [PluginEffectState],
    pub(crate) tuner: LiveTunerData,
}

pub(crate) struct LiveWindow;

impl LiveWindow {
    pub(crate) fn show(ctx: &egui::Context, open: &mut bool, data: LiveWindowData<'_>) {
        if !*open {
            return;
        }

        let palette = LivePalette::default();
        let viewport_id = egui::ViewportId::from_hash_of("live-mode");
        let viewport_builder = egui::ViewportBuilder::default()
            .with_title("Live Mode")
            .with_fullscreen(true);
        let mut exit_requested = false;
        let close_requested = ctx.show_viewport_immediate(
            viewport_id,
            viewport_builder,
            |viewport_ctx, viewport_class| {
                if matches!(viewport_class, egui::ViewportClass::Embedded) {
                    let mut visible = true;
                    egui::Window::new("Live Mode")
                        .open(&mut visible)
                        .resizable(true)
                        .show(viewport_ctx, |ui| {
                            show_contents(ui, &palette, &data, &mut exit_requested);
                        });
                    !visible
                } else {
                    viewport_ctx.request_repaint();
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(STAGE_BACKGROUND))
                        .show(viewport_ctx, |ui| {
                            show_contents(ui, &palette, &data, &mut exit_requested);
                        });
                    viewport_ctx.input(|input| input.viewport().close_requested())
                }
            },
        );

        if close_requested || exit_requested {
            *open = false;
        }
    }
}

fn show_contents(
    ui: &mut egui::Ui,
    palette: &LivePalette,
    data: &LiveWindowData<'_>,
    exit_requested: &mut bool,
) {
    ui.set_min_width(900.0);
    ui.add_space(28.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("LIVE MODE")
                .size(44.0)
                .strong()
                .color(palette.text_primary),
        );
        ui.add_space(18.0);
        ui.label(
            egui::RichText::new("STAGE VIEW")
                .size(18.0)
                .strong()
                .color(palette.cyan),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if show_exit_button(ui, palette) {
                *exit_requested = true;
            }
        });
    });
    ui.add_space(26.0);

    ui.columns(2, |columns| {
        show_panel(&mut columns[0], palette, "TUNER", |ui| {
            show(ui, palette, &data.tuner);
        });
        show_panel(&mut columns[1], palette, "LOADED LIVE PRESET", |ui| {
            show_preset(
                ui,
                palette,
                data.live_preset_name,
                data.live_preset_boards,
                data.current_board_name,
            );
        });
    });

    ui.add_space(22.0);
    show_panel(ui, palette, "CURRENT BOARD CONFIGURATION", |ui| {
        ui.label(
            egui::RichText::new(
                data.current_board_name
                    .unwrap_or("NO SAVED BOARD PRESET IS LOADED"),
            )
            .size(25.0)
            .strong()
            .color(palette.text_primary),
        );
        ui.add_space(16.0);
        show_effects(ui, palette, data.effects);
    });
}

fn show_panel(
    ui: &mut egui::Ui,
    palette: &LivePalette,
    title: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::NONE
        .fill(palette.panel)
        .stroke(egui::Stroke::new(
            2.0_f32,
            palette.cyan.gamma_multiply(0.45),
        ))
        .inner_margin(egui::Margin::same(24))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(18.0)
                    .strong()
                    .color(palette.cyan),
            );
            ui.add_space(18.0);
            contents(ui);
        });
}

fn show_exit_button(ui: &mut egui::Ui, palette: &LivePalette) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(220.0, 62.0), egui::Sense::click());
    let fill = if response.hovered() {
        palette.panel_highlight
    } else {
        palette.panel
    };
    let border = if response.hovered() {
        palette.text_primary
    } else {
        palette.text_secondary
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, fill);
    painter.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(2.0_f32, border),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "EXIT LIVE MODE",
        egui::FontId::proportional(18.0),
        palette.text_primary,
    );
    response.clicked()
}
