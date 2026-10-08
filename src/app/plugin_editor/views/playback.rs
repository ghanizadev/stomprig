use super::super::*;

impl PluginEditor {
    pub(crate) fn show_playback_container(&mut self, ui: &mut egui::Ui) {
        let body_height = if self.playback_collapsed {
            COLLAPSED_PLUGIN_EDITOR_HEIGHT
        } else {
            PLAYBACK_EDITOR_HEIGHT
        };

        ui.horizontal_top(|ui| {
            ui.allocate_exact_size(egui::vec2(32.0, body_height), egui::Sense::hover());
            let body_width = ui.available_width().max(0.0);
            ui.allocate_ui_with_layout(
                egui::vec2(body_width, body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(body_height);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.show_playback(ui);
                    });
                },
            );
        });
        ui.add_space(8.0);

        if self.playback.is_playing() {
            ui.ctx().request_repaint();
        }
    }

    fn show_playback(&mut self, ui: &mut egui::Ui) {
        let theme = self.settings.theme.clone();
        let has_clip = self.playback_clip.is_some();
        let duration = self
            .playback_clip
            .as_ref()
            .map(|clip| clip.frames() as f64 / f64::from(clip.sample_rate))
            .unwrap_or(0.0);
        let collapsed = self.playback_collapsed;
        let mut bypass_requested = false;
        let mut collapse_requested = false;

        ui.horizontal(|ui| {
            let bypassed = self.playback.is_bypassed();
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
                    "Enable playback"
                } else {
                    "Bypass playback"
                })
                .clicked()
            {
                bypass_requested = true;
            }

            strong(ui, "Playback");
            if self.playback.is_playing() {
                ui_label(ui, "(playing)");
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
                        "Expand playback"
                    } else {
                        "Collapse playback"
                    })
                    .clicked()
                {
                    collapse_requested = true;
                }
            });
        });

        if bypass_requested {
            self.playback.set_bypassed(!self.playback.is_bypassed());
        }
        if collapse_requested {
            self.playback_collapsed = !collapsed;
        }

        if collapsed {
            return;
        }

        ui.add_space(6.0);
        let choose_audio_label = self
            .playback_file
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Choose audio".to_owned());
        if UiButton::new(choose_audio_label)
            .button_type(ButtonType::Outline)
            .justify(ButtonJustify::Center)
            .min_width(ui.available_width().max(40.0))
            .show(ui, &theme)
            .clicked()
        {
            self.choose_playback_file();
        }
        let mut position = if duration > 0.0 {
            self.playback.position_frames() as f64
                / self
                    .playback_clip
                    .as_ref()
                    .map(|clip| f64::from(clip.sample_rate))
                    .unwrap_or(1.0)
        } else {
            0.0
        };
        position = position.clamp(0.0, duration);
        let progress_width = ui.available_width();
        let progress = ui.add_enabled_ui(has_clip, |ui| {
            ui.set_width(progress_width);
            ui.spacing_mut().slider_width = progress_width;
            ui.add(egui::Slider::new(&mut position, 0.0..=duration.max(0.001)).show_value(false))
        });
        if progress.inner.changed()
            && let Some(clip) = &self.playback_clip
        {
            self.playback
                .request_seek((position * f64::from(clip.sample_rate)) as u64);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let is_playing = self.playback.is_playing();
            let play_icon = move |painter: &egui::Painter,
                                  center: egui::Pos2,
                                  size: f32,
                                  color: egui::Color32| {
                paint_lucide_icon(
                    painter,
                    center,
                    size,
                    color,
                    if is_playing { Icon::Pause } else { Icon::Play },
                );
            };
            if UiButton::new("")
                .button_type(ButtonType::Primary)
                .size(ButtonSize::IconSm)
                .icon(&play_icon)
                .enabled(has_clip)
                .show(ui, &theme)
                .on_hover_text(if is_playing {
                    "Pause playback"
                } else {
                    "Play playback"
                })
                .clicked()
            {
                if !is_playing && position >= duration && has_clip {
                    self.playback.request_seek(0);
                }
                self.playback.set_playing(!is_playing);
            }
            let stop_icon =
                |painter: &egui::Painter, center: egui::Pos2, size: f32, color: egui::Color32| {
                    paint_lucide_icon(painter, center, size, color, Icon::Square);
                };
            if UiButton::new("")
                .button_type(ButtonType::Outline)
                .size(ButtonSize::IconSm)
                .icon(&stop_icon)
                .enabled(has_clip)
                .show(ui, &theme)
                .on_hover_text("Stop playback")
                .clicked()
            {
                self.playback.reset();
            }
            ui.add_space(8.0);
            ui_label(ui, "Volume");
            let volume_width = (ui.available_width() - 150.0).clamp(60.0, 140.0);
            let mut volume = self.playback.volume();
            let volume_changed = ui
                .add_enabled_ui(has_clip, |ui| {
                    ui.add_sized(
                        [volume_width, 18.0],
                        egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false),
                    )
                })
                .inner
                .changed();
            if volume_changed {
                self.playback.set_volume(volume);
            }
            ui_label(ui, format!("{:.0}%", self.playback.volume() * 100.0));
            ui.separator();
            ui_label(ui, format_time(position));
            ui_label(ui, "/");
            ui_label(ui, format_time(duration));
        });
    }

    fn choose_playback_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio files", &["mp3", "wav"])
            .pick_file()
        else {
            return;
        };
        match decode_file(&path) {
            Ok(clip) => {
                let clip = Arc::new(clip);
                self.playback.reset();
                match self.runtime.handle().set_playback_clip(Arc::clone(&clip)) {
                    Ok(()) => {
                        self.playback_clip = Some(clip);
                        self.playback_file = Some(path);
                        self.error = None;
                    }
                    Err(error) => {
                        self.error =
                            Some(format!("Could not load audio into the runtime: {error}"));
                    }
                }
            }
            Err(error) => self.error = Some(error),
        }
    }
}
