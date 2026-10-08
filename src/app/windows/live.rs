use super::super::plugin_editor::PluginEffectState;
use super::super::ui::{self, ButtonType, Theme};

pub struct LiveWindow;

pub struct LiveWindowData<'a> {
    pub current_board_name: Option<&'a str>,
    pub live_preset_name: Option<&'a str>,
    pub live_preset_boards: Option<&'a [String]>,
    pub effects: &'a [PluginEffectState],
}

impl LiveWindow {
    pub fn show(
        ctx: &egui::Context,
        open: &mut bool,
        theme: &Theme,
        data: LiveWindowData<'_>,
        mut show_tuner: impl FnMut(&mut egui::Ui),
    ) {
        if !*open {
            return;
        }

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
                            show_contents(ui, theme, &data, &mut show_tuner, &mut exit_requested);
                        });
                    !visible
                } else {
                    viewport_ctx.request_repaint();
                    egui::CentralPanel::default().show(viewport_ctx, |ui| {
                        show_contents(ui, theme, &data, &mut show_tuner, &mut exit_requested);
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
    theme: &Theme,
    data: &LiveWindowData<'_>,
    show_tuner: &mut impl FnMut(&mut egui::Ui),
    exit_requested: &mut bool,
) {
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        ui.heading("Live mode");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui::button_with_type(ui, theme, "Exit live mode", ButtonType::Secondary).clicked() {
                *exit_requested = true;
            }
        });
    });
    ui.add_space(16.0);

    ui.columns(2, |columns| {
        columns[0].group(|ui| {
            ui.heading("Tuner");
            ui.add_space(8.0);
            show_tuner(ui);
        });
        columns[1].group(|ui| {
            ui.heading("Loaded live preset");
            ui.add_space(8.0);
            match data.live_preset_name {
                Some(name) => {
                    ui::strong(ui, name);
                    show_live_preset_boards(ui, data.live_preset_boards, data.current_board_name);
                }
                None => {
                    ui::muted(ui, "No live preset selected.");
                }
            }
        });
    });

    ui.add_space(16.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.heading("Current board configuration");
        ui.add_space(4.0);
        ui::muted(
            ui,
            data.current_board_name
                .unwrap_or("No saved board preset is loaded."),
        );
        ui.add_space(10.0);
        if data.effects.is_empty() {
            ui::muted(ui, "No effects are loaded.");
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("live-effects")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, effect) in data.effects.iter().enumerate() {
                    show_effect_row(ui, index, effect);
                }
            });
    });
}

fn show_live_preset_boards(
    ui: &mut egui::Ui,
    boards: Option<&[String]>,
    current_board_name: Option<&str>,
) {
    let Some(boards) = boards else {
        ui::muted(ui, "The live preset has no saved board list.");
        return;
    };
    if boards.is_empty() {
        ui::muted(ui, "The live preset has no board presets.");
        return;
    }

    ui.add_space(8.0);
    egui::ScrollArea::vertical()
        .id_salt("live-board-presets")
        .max_height(180.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, board_name) in boards.iter().enumerate() {
                let loaded = current_board_name == Some(board_name.as_str());
                ui.horizontal(|ui| {
                    ui::muted(ui, format!("{}.", index + 1));
                    ui.add(ui::Label::new(board_name).truncate());
                    if loaded {
                        ui::success_label(ui, "Loaded");
                    }
                });
            }
        });
}

fn show_effect_row(ui: &mut egui::Ui, index: usize, effect: &PluginEffectState) {
    let active_color = egui::Color32::from_rgb(74, 222, 128);
    let bypassed_color = ui.visuals().weak_text_color();
    let color = if effect.bypassed {
        bypassed_color
    } else {
        active_color
    };
    let status = if effect.bypassed {
        "BYPASSED"
    } else {
        "ACTIVE"
    };

    egui::Frame::group(ui.style())
        .fill(color.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0_f32, color.gamma_multiply(0.65)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui::muted(ui, format!("{}.", index + 1));
                ui.add(ui::Label::new(&effect.name).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(status).color(color).strong());
                });
            });
        });
}
