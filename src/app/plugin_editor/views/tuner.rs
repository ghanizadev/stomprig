use super::super::*;

impl PluginEditor {
    pub(crate) fn show_tuner_container(&mut self, ui: &mut egui::Ui) {
        let collapsed = self.tuner_collapsed;

        ui.horizontal_top(|ui| {
            ui.allocate_space(egui::vec2(
                32.0,
                if collapsed {
                    TUNER_COLLAPSED_EDITOR_HEIGHT
                } else {
                    0.0
                },
            ));
            let body_width = ui.available_width().max(0.0);
            let body_rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(body_width, ui.available_height().max(0.0)),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(body_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if collapsed {
                            ui.set_min_height(TUNER_COLLAPSED_EDITOR_HEIGHT);
                        }
                        self.show_tuner(ui);
                    });
                },
            );
        });
        ui.add_space(8.0);

        if self.engine.is_some() && !self.tuner.is_bypassed() {
            ui.ctx().request_repaint();
        }
    }

    fn show_tuner(&mut self, ui: &mut egui::Ui) {
        let theme = self.settings.theme.clone();
        let collapsed = self.tuner_collapsed;
        let readout = self.tuner.read();
        let mut tuner_settings = self.tuner_settings.clone();
        let mut bypass_requested = false;
        let mut collapse_requested = false;

        ui.horizontal(|ui| {
            let bypassed = self.tuner.is_bypassed();
            let bypass_icon =
                |painter: &egui::Painter, center: egui::Pos2, size: f32, color: egui::Color32| {
                    paint_lucide_icon(painter, center, size, color, Icon::Power);
                };
            if UiButton::new("")
                .button_type(if bypassed {
                    ButtonType::Secondary
                } else {
                    ButtonType::Outline
                })
                .size(ButtonSize::IconSm)
                .icon(&bypass_icon)
                .show(ui, &theme)
                .on_hover_text(if bypassed {
                    "Enable tuner"
                } else {
                    "Bypass tuner"
                })
                .clicked()
            {
                bypass_requested = true;
            }

            strong(ui, "Tuner");
            if let Some(note) = tuner_note(
                readout,
                self.tuner_settings.a4_pitch_hz,
                self.tuner_settings.transposition_semitones,
            ) {
                ui_label(ui, note);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let collapse_icon = |painter: &egui::Painter,
                                     center: egui::Pos2,
                                     size: f32,
                                     color: egui::Color32| {
                    paint_lucide_icon(
                        painter,
                        center,
                        size,
                        color,
                        if collapsed {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronUp
                        },
                    );
                };
                if UiButton::new("")
                    .button_type(ButtonType::Outline)
                    .size(ButtonSize::IconSm)
                    .icon(&collapse_icon)
                    .show(ui, &theme)
                    .on_hover_text(if collapsed {
                        "Expand tuner"
                    } else {
                        "Collapse tuner"
                    })
                    .clicked()
                {
                    collapse_requested = true;
                }
            });
        });

        if bypass_requested {
            self.tuner.set_bypassed(!self.tuner.is_bypassed());
        }
        if collapse_requested {
            self.tuner_collapsed = !collapsed;
        }

        if !self.tuner.is_bypassed() {
            ui.add_space(3.0);
            self.show_tuner_dial(ui, readout);
        }
        if collapsed {
            return;
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if let Some(note) = tuner_note(
                readout,
                self.tuner_settings.a4_pitch_hz,
                self.tuner_settings.transposition_semitones,
            ) {
                strong(ui, note);
                ui.separator();
                ui_label(ui, format!("{:.1} Hz", readout.frequency_hz));
                ui.separator();
                ui_label(ui, format_cents(readout.cents));
            } else {
                muted(ui, "Play an instrument to tune");
            }
        });
        ui.add_space(8.0);
        ui.columns(2, |columns| {
            columns[0].group(|ui| {
                strong(ui, "Tuner settings");
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui_label(ui, "Input sensitivity");
                    ui.add(
                        egui::Slider::new(
                            &mut tuner_settings.input_sensitivity_db,
                            MIN_INPUT_SENSITIVITY_DB..=MAX_INPUT_SENSITIVITY_DB,
                        )
                        .suffix(" dB")
                        .step_by(1.0),
                    )
                    .on_hover_text("Lower dB values make the tuner respond to quieter input.");
                });
                ui.horizontal(|ui| {
                    ui_label(ui, "A4 pitch");
                    ui.add(
                        egui::Slider::new(
                            &mut tuner_settings.a4_pitch_hz,
                            MIN_A4_PITCH_HZ..=MAX_A4_PITCH_HZ,
                        )
                        .suffix(" Hz")
                        .step_by(1.0),
                    )
                    .on_hover_text("Set the reference pitch used for the note and cents display.");
                });
                ui.horizontal(|ui| {
                    ui_label(ui, "Display transposition");
                    ui.add(
                        egui::Slider::new(
                            &mut tuner_settings.transposition_semitones,
                            MIN_TRANSPOSITION_SEMITONES..=MAX_TRANSPOSITION_SEMITONES,
                        )
                        .suffix(" semitones")
                        .step_by(1.0),
                    )
                    .on_hover_text(
                        "Positive values shift displayed notes up, for example +2 for whole-step-down tuning.",
                    );
                });
            });
            columns[1].group(|ui| {
                strong(ui, "Tuning presets");
                ui.add_space(2.0);
                ui_label(ui, "Tuning preset");
                let mut selected_preset = Some(tuner_settings.tuning_preset.label().to_owned());
                let preset_items: Vec<SelectItem> = TuningPreset::ALL
                    .into_iter()
                    .map(|preset| SelectItem::option(preset.label(), preset.label()))
                    .collect();
                if Select::new("tuner-tuning-preset", &mut selected_preset, &preset_items)
                    .width(ui.available_width().min(300.0))
                    .show(ui, &theme)
                .changed()
                    && let Some(selected) = selected_preset.as_deref()
                    && let Some(preset) = TuningPreset::from_label(selected)
                {
                    tuner_settings.tuning_preset = preset;
                }
            });
        });
        if tuner_settings != self.tuner_settings {
            self.tuner_settings = tuner_settings.normalized();
            self.apply_tuner_settings();
        }
    }

    fn show_tuner_dial(&self, ui: &mut egui::Ui, readout: TunerReadout) {
        const BAR_COUNT: usize = 101;
        const DIAL_HEIGHT: f32 = 24.0;

        let width = ui.available_width().max(1.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(width, DIAL_HEIGHT), egui::Sense::hover());
        let painter = ui.painter();
        let bar_gap = (rect.width() / BAR_COUNT as f32 * 0.25).min(1.0);
        let bar_width =
            ((rect.width() - bar_gap * (BAR_COUNT - 1) as f32) / BAR_COUNT as f32).max(0.5);
        let target = if readout.confidence >= 0.25 && readout.frequency_hz > 0.0 {
            Some((readout.cents.clamp(-50.0, 50.0) + 50.0).round() as usize)
        } else {
            None
        };
        let center = BAR_COUNT / 2;
        let green = egui::Color32::from_rgb(74, 222, 128);
        let yellow = egui::Color32::from_rgb(250, 204, 21);
        let inactive = ui.visuals().widgets.noninteractive.fg_stroke.color;

        for index in 0..BAR_COUNT {
            let distance = index.abs_diff(center);
            let color = if distance == 0 {
                green
            } else if distance <= 2 {
                yellow
            } else {
                inactive
            };
            let height = if distance == 0 {
                22.0
            } else if distance <= 2 {
                18.0
            } else {
                12.0
            };
            let active = target.is_some_and(|target| target == index);
            let color = if active {
                color
            } else {
                color.gamma_multiply(if target.is_some() { 0.28 } else { 0.16 })
            };
            let left = rect.left() + index as f32 * (bar_width + bar_gap);
            let bar = egui::Rect::from_center_size(
                egui::pos2(left + bar_width * 0.5, rect.center().y),
                egui::vec2(bar_width, height),
            );
            painter.rect_filled(bar, 2.0, color);
        }
    }
}
