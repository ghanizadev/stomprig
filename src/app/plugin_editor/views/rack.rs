use super::super::*;

struct PluginUiRequests<'a> {
    edits: &'a mut Vec<(usize, u32, f64)>,
    remove_requested: &'a mut Option<usize>,
    parameter_learn_requested: &'a mut Option<(usize, u32)>,
    parameter_mapping_remove_requested: &'a mut Vec<(usize, u32)>,
}

impl PluginEditor {
    pub(crate) fn show_plugin_rack(&mut self, ui: &mut egui::Ui) {
        self.service_editor_windows();
        self.poll_parameter_worker_results();
        if self.last_parameter_refresh.elapsed() >= PARAMETER_REFRESH_INTERVAL {
            self.refresh_parameters();
        }
        ui.ctx().request_repaint_after(PARAMETER_REFRESH_INTERVAL);
        let mut edits = Vec::new();
        let mut remove_requested = None;
        let mut reorder_requested = None;
        let mut parameter_learn_requested = None;
        let mut parameter_mapping_remove_requested = Vec::new();
        let previous_drop_zone = if egui::DragAndDrop::has_payload_of_type::<usize>(ui.ctx()) {
            self.drop_zone_hovered
        } else {
            self.drop_zone_hovered = None;
            None
        };
        self.drop_zone_hovered = None;
        egui::ScrollArea::vertical()
            .id_salt("plugin-rack-scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                self.show_signal_entry(ui);
                if self.show_tuner {
                    self.show_signal_path_arrow(ui);
                    self.show_tuner_container(ui);
                }
                if let Some(reorder) = self.show_plugin_drop_zone(ui, 0, previous_drop_zone) {
                    reorder_requested = Some(reorder);
                }
                for index in 0..self.rack.slots.len() {
                    let mut requests = PluginUiRequests {
                        edits: &mut edits,
                        remove_requested: &mut remove_requested,
                        parameter_learn_requested: &mut parameter_learn_requested,
                        parameter_mapping_remove_requested: &mut parameter_mapping_remove_requested,
                    };
                    self.show_plugin_container(ui, index, &mut requests);
                    if let Some(reorder) =
                        self.show_plugin_drop_zone(ui, index + 1, previous_drop_zone)
                    {
                        reorder_requested = Some(reorder);
                    }
                }
                if self.show_playback {
                    self.show_playback_container(ui);
                }
                self.show_add_plugin_button(ui);
            });
        for (plugin_index, parameter_id, value) in edits {
            self.set_parameter(plugin_index, parameter_id, value);
        }
        let mut midi_mappings_changed = false;
        for (plugin_index, parameter_id) in parameter_mapping_remove_requested {
            if let Some(slot) = self.rack.slots.get_mut(plugin_index) {
                let previous_len = slot.midi_mappings.len();
                slot.midi_mappings.retain(|mapping| {
                    !matches!(
                        mapping.action,
                        super::super::super::midi::MidiAction::SetParameter {
                            parameter_id: mapped_id
                        } if mapped_id == parameter_id
                    )
                });
                midi_mappings_changed |= slot.midi_mappings.len() != previous_len;
            }
        }
        if midi_mappings_changed {
            self.sync_midi_mappings();
        }
        if let Some((plugin_index, parameter_id)) = parameter_learn_requested {
            self.start_midi_parameter_learn(plugin_index, parameter_id);
        }
        if let Some(plugin_index) = remove_requested {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Progress);
            self.remove_plugin(plugin_index);
        }
        if let Some((dragged_index, drop_index)) = reorder_requested {
            self.reorder_plugin(*dragged_index, drop_index);
        }
    }

    fn show_signal_entry(&self, ui: &mut egui::Ui) {
        const SIGNAL_ENTRY_HEIGHT: f32 = 48.0;

        ui.horizontal_top(|ui| {
            ui.allocate_exact_size(egui::vec2(32.0, SIGNAL_ENTRY_HEIGHT), egui::Sense::hover());
            let content_width = ui.available_width().max(0.0);
            ui.allocate_ui_with_layout(
                egui::vec2(content_width, SIGNAL_ENTRY_HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(SIGNAL_ENTRY_HEIGHT);
                        ui.with_layout(
                            egui::Layout::centered_and_justified(egui::Direction::TopDown),
                            |ui| {
                                strong(ui, "Instrument");
                            },
                        );
                    });
                },
            );
        });
    }

    fn show_signal_path_arrow(&self, ui: &mut egui::Ui) {
        const SIGNAL_PATH_ARROW_HEIGHT: f32 = 16.0;

        let (arrow_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), SIGNAL_PATH_ARROW_HEIGHT),
            egui::Sense::hover(),
        );
        let arrow_center =
            arrow_rect.center() + egui::vec2((32.0 + ui.spacing().item_spacing.x) * 0.5, 0.0);
        paint_lucide_icon(
            ui.painter(),
            arrow_center,
            20.0,
            ui.visuals().text_color().gamma_multiply(0.35),
            Icon::ArrowDown,
        );
    }

    fn show_plugin_drop_zone(
        &mut self,
        ui: &mut egui::Ui,
        drop_index: usize,
        previous_drop_zone: Option<usize>,
    ) -> Option<(Arc<usize>, usize)> {
        let is_dragging = egui::DragAndDrop::has_payload_of_type::<usize>(ui.ctx());
        let is_expanded = is_dragging && previous_drop_zone == Some(drop_index);
        let zone_height = if is_expanded { 44.0 } else { 12.0 };
        let (zone_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), zone_height),
            egui::Sense::hover(),
        );
        let zone_response = ui.interact(
            zone_rect,
            ui.make_persistent_id(("plugin-drop-zone", drop_index)),
            egui::Sense::hover(),
        );
        let is_drop_target = is_dragging && zone_response.contains_pointer();
        if is_drop_target {
            self.drop_zone_hovered = Some(drop_index);
            ui.ctx().request_repaint();
        }
        if is_dragging {
            let is_highlighted = is_drop_target || is_expanded;
            if is_highlighted {
                ui.painter().rect_filled(
                    zone_response.rect,
                    4.0,
                    ui.visuals().selection.bg_fill.linear_multiply(0.18),
                );
                paint_label(
                    ui.painter(),
                    zone_response.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop here",
                    egui::TextStyle::Small.resolve(ui.style()),
                    ui.visuals().selection.stroke.color,
                );
            } else {
                ui.painter().line_segment(
                    [
                        egui::pos2(zone_response.rect.left(), zone_response.rect.center().y),
                        egui::pos2(zone_response.rect.right(), zone_response.rect.center().y),
                    ],
                    egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
                );
            }
        } else if drop_index <= self.rack.slots.len() {
            let content_center = zone_response.rect.center()
                + egui::vec2((32.0 + ui.spacing().item_spacing.x) * 0.5, 0.0);
            paint_lucide_icon(
                ui.painter(),
                content_center,
                20.0,
                ui.visuals().text_color().gamma_multiply(0.35),
                Icon::ArrowDown,
            );
        }
        let dropped = zone_response.dnd_release_payload::<usize>();
        dropped.map(|dragged_index| (dragged_index, drop_index))
    }

    fn show_plugin_container(
        &mut self,
        ui: &mut egui::Ui,
        plugin_index: usize,
        requests: &mut PluginUiRequests<'_>,
    ) {
        let plugin_name = self.rack.slots[plugin_index].snapshot.name.clone();
        let drag_id = ui.make_persistent_id(("plugin-drag-handle", plugin_index));
        let is_dragged = ui.ctx().is_being_dragged(drag_id);
        let parameter_count = self.rack.slots[plugin_index].snapshot.parameters.len();

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                let (handle_rect, _) =
                    ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                let handle_response = ui.interact(handle_rect, drag_id, egui::Sense::drag());
                let handle_dragged = handle_response.dragged();
                let handle_hovered = handle_response.hovered() || handle_dragged;
                let handle_fill = if handle_hovered {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                };
                ui.painter().rect_filled(handle_rect, 5.0, handle_fill);
                for row in 0..3 {
                    for column in 0..2 {
                        let center = egui::pos2(
                            handle_rect.left() + 11.0 + column as f32 * 8.0,
                            handle_rect.center().y - 8.0 + row as f32 * 8.0,
                        );
                        ui.painter().circle_filled(
                            center,
                            1.5,
                            if handle_hovered {
                                ui.visuals().selection.stroke.color
                            } else {
                                ui.visuals().text_color()
                            },
                        );
                    }
                }
                let handle_response = handle_response
                    .on_hover_cursor(if handle_dragged {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    })
                    .on_hover_text("Drag to reorder plugin");
                handle_response.dnd_set_drag_payload(plugin_index);
            });

            let body_width = ui.available_width().max(0.0);
            let body_height = if self.rack.slots[plugin_index].collapsed {
                COLLAPSED_PLUGIN_EDITOR_HEIGHT
            } else {
                CUSTOM_PLUGIN_EDITOR_HEIGHT
            };
            ui.allocate_ui_with_layout(
                egui::vec2(body_width, body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(body_height);
                    if is_dragged {
                        ui.set_opacity(0.35);
                    }
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.show_plugin_container_body(
                            ui,
                            plugin_index,
                            &plugin_name,
                            parameter_count,
                            requests,
                        );
                    });
                },
            );
            if is_dragged {
                show_plugin_drag_preview(ui.ctx(), &plugin_name, parameter_count);
            }
        });
        ui.add_space(8.0);
    }

    fn show_plugin_container_body(
        &mut self,
        ui: &mut egui::Ui,
        plugin_index: usize,
        plugin_name: &str,
        parameter_count: usize,
        requests: &mut PluginUiRequests<'_>,
    ) {
        let theme = self.settings.theme.clone();
        let collapsed = self.rack.slots[plugin_index].collapsed;
        let mut bypass_requested = false;
        let mut open_editor_requested = false;
        let mut collapse_requested = false;

        ui.horizontal(|ui| {
            let bypassed = self.plugin_bypassed(plugin_index);
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
                    "Enable plugin"
                } else {
                    "Bypass plugin"
                })
                .clicked()
            {
                bypass_requested = true;
            }

            strong(ui, plugin_name);
            ui_label(ui, format!("{parameter_count} parameters"));

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
                        "Expand plugin editor"
                    } else {
                        "Collapse plugin editor"
                    })
                    .clicked()
                {
                    collapse_requested = true;
                }

                let remove_icon = |painter: &egui::Painter,
                                   center: egui::Pos2,
                                   size: f32,
                                   color: egui::Color32| {
                    paint_lucide_icon(painter, center, size, color, Icon::Trash2);
                };
                if UiButton::new("")
                    .button_type(ButtonType::Outline)
                    .size(ButtonSize::IconSm)
                    .icon(&remove_icon)
                    .show(ui, &theme)
                    .on_hover_text("Remove plugin")
                    .clicked()
                {
                    *requests.remove_requested = Some(plugin_index);
                }

                let editor_icon = |painter: &egui::Painter,
                                   center: egui::Pos2,
                                   size: f32,
                                   color: egui::Color32| {
                    paint_lucide_icon(painter, center, size, color, Icon::Eye);
                };
                if UiButton::new("")
                    .button_type(ButtonType::Outline)
                    .size(ButtonSize::IconSm)
                    .enabled(self.rack.slots[plugin_index].snapshot.has_editor)
                    .icon(&editor_icon)
                    .show(ui, &theme)
                    .on_hover_text(if self.rack.slots[plugin_index].snapshot.has_editor {
                        "Open native editor"
                    } else {
                        "No native editor"
                    })
                    .clicked()
                {
                    open_editor_requested = true;
                }
            });
        });

        if bypass_requested {
            self.toggle_plugin_bypass(plugin_index);
        }
        if open_editor_requested {
            self.open_native_editor(plugin_index);
        }
        if collapse_requested {
            self.rack.slots[plugin_index].collapsed = !collapsed;
        }

        if collapsed {
            return;
        }

        let learned_trigger = self
            .rack
            .slots
            .get(plugin_index)
            .map(|slot| &slot.midi_mappings)
            .and_then(|mappings| {
                mappings.iter().find(|mapping| {
                    matches!(
                        mapping.action,
                        super::super::super::midi::MidiAction::ToggleBypass
                    )
                })
            })
            .map(PresetMidiMapping::label);
        let is_learning = self.midi_learn_plugin == Some(plugin_index);
        let learn_button_label = if is_learning {
            "..."
        } else if learned_trigger.is_some() {
            "CC"
        } else {
            "M"
        };
        let learn_button_tooltip = if is_learning {
            "Cancel MIDI learn".to_owned()
        } else if let Some(trigger) = learned_trigger.as_deref() {
            format!("{trigger}; learn a different control")
        } else {
            "Learn a MIDI CC for bypass".to_owned()
        };
        let mut learn_requested = false;
        let mut cancel_learn_requested = false;
        let mut cancel_parameter_learn_requested = false;
        let mut clear_mapping_requested = false;
        let theme = self.settings.theme.clone();
        ui.horizontal(|ui| {
            strong(ui, "MIDI");
            if is_learning {
                ui_label(ui, "Listening for MIDI...");
            } else {
                ui_label(ui, learned_trigger.as_deref().unwrap_or("No mapping"));
            }
            let learn_response = ui
                .add_sized(
                    [24.0, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                    egui::Button::new(learn_button_label),
                )
                .on_hover_text(learn_button_tooltip);
            if learn_response.clicked() {
                if is_learning {
                    cancel_learn_requested = true;
                } else {
                    learn_requested = true;
                }
            }
            if !is_learning
                && learned_trigger.is_some()
                && UiButton::new("Clear").show(ui, &theme).clicked()
            {
                clear_mapping_requested = true;
            }
            muted(ui, "Toggle bypass");
        });
        if learn_requested {
            self.start_midi_learn(plugin_index);
        }
        if cancel_learn_requested {
            self.cancel_midi_learn();
        }
        if cancel_parameter_learn_requested {
            self.cancel_midi_learn();
        }
        if clear_mapping_requested {
            if let Some(mappings) = self
                .rack
                .slots
                .get_mut(plugin_index)
                .map(|slot| &mut slot.midi_mappings)
            {
                mappings.retain(|mapping| {
                    !matches!(
                        mapping.action,
                        super::super::super::midi::MidiAction::ToggleBypass
                    )
                });
            }
            self.sync_midi_mappings();
        }

        if self.rack.slots[plugin_index].snapshot.parameters.is_empty() {
            ui_label(ui, "This plugin does not expose any parameters.");
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt(("plugin-parameters-scroll", plugin_index))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (group_index, (unit, parameter_indices)) in
                    group_parameter_indices(&self.rack.slots[plugin_index].snapshot.parameters)
                        .into_iter()
                        .enumerate()
                {
                    ui.add_space(4.0);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        strong(ui, unit);
                        ui.add_space(4.0);

                        let column_ranges = balanced_column_ranges(
                            parameter_indices.len(),
                            CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT,
                        );
                        ui.columns(CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT, |columns| {
                            for (column_index, (start, end)) in
                                column_ranges.iter().copied().enumerate()
                            {
                                let ui = &mut columns[column_index];
                                let parameter_indices = &parameter_indices[start..end];
                                let column_width = ui.available_width();
                                let name_width = (column_width * 0.26).max(0.0);
                                let value_width = 56.0_f32.min((column_width * 0.28).max(0.0));
                                let midi_width = 44.0_f32.min((column_width * 0.24).max(0.0));
                                let control_width = (column_width
                                    - name_width
                                    - value_width
                                    - midi_width
                                    - ui.spacing().item_spacing.x * 3.0)
                                    .max(0.0);

                                egui::Grid::new((
                                    "plugin-parameters",
                                    plugin_index,
                                    group_index,
                                    column_index,
                                ))
                                .striped(true)
                                .num_columns(4)
                                .show(ui, |ui| {
                                    ui_label(ui, "Parameter");
                                    ui_label(ui, "Control");
                                    ui_label(ui, "Value");
                                    ui_label(ui, "MIDI");
                                    ui.end_row();

                                    for &parameter_index in parameter_indices {
                                        let parameter_id = self.rack.slots[plugin_index].snapshot
                                            .parameters[parameter_index]
                                            .id;
                                        let midi_mapping_label = self.rack.slots[plugin_index]
                                            .midi_mappings
                                            .iter()
                                            .find(|mapping| {
                                                matches!(
                                                    mapping.action,
                                                    super::super::super::midi::MidiAction::SetParameter {
                                                        parameter_id: mapped_id
                                                    } if mapped_id == parameter_id
                                                )
                                            })
                                            .map(|mapping| mapping.label());
                                        let rack_parameter = &mut self.rack.slots[plugin_index]
                                            .snapshot
                                            .parameters[parameter_index];
                                        ui.add_sized(
                                            [name_width, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                                            Label::new(&rack_parameter.name).wrap(),
                                        );

                                        if is_booleanish_parameter(
                                            rack_parameter,
                                            &rack_parameter.formatted_value,
                                        ) {
                                            let mut checked = boolean_parameter_value(
                                                rack_parameter,
                                                &rack_parameter.formatted_value,
                                            );
                                            let response = ui
                                                .add_enabled_ui(
                                                    !rack_parameter.is_read_only,
                                                    |ui| {
                                                        ui.add_sized(
                                                            [
                                                                control_width,
                                                                CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT,
                                                            ],
                                                            egui::Checkbox::without_text(
                                                                &mut checked,
                                                            ),
                                                        )
                                                    },
                                                )
                                                .inner;
                                            if response.changed() {
                                                rack_parameter.value = if checked {
                                                    rack_parameter.max
                                                } else {
                                                    rack_parameter.min
                                                };
                                                rack_parameter.formatted_value =
                                                    rack_parameter.value.to_string();
                                                requests.edits.push((
                                                    plugin_index,
                                                    rack_parameter.id,
                                                    rack_parameter.value,
                                                ));
                                            }
                                        } else if !rack_parameter.choices.is_empty() {
                                            let mut selected_choice = Some(
                                                rack_parameter
                                                    .selected_choice_index()
                                                    .unwrap_or_default()
                                                    .to_string(),
                                            );
                                            let choice_items = rack_parameter
                                                .choices
                                                .iter()
                                                .enumerate()
                                                .map(|(index, label)| {
                                                    SelectItem::option(index.to_string(), label)
                                                })
                                                .collect::<Vec<_>>();
                                            let response = Select::new(
                                                (
                                                    "plugin-parameter-choice",
                                                    plugin_index,
                                                    rack_parameter.id,
                                                ),
                                                &mut selected_choice,
                                                &choice_items,
                                            )
                                            .size(SelectSize::Size1)
                                            .width(control_width)
                                            .enabled(!rack_parameter.is_read_only)
                                            .show(ui, &theme);
                                            if response.changed()
                                                && let Some(selected_choice) = selected_choice
                                                && let Ok(index) = selected_choice.parse::<usize>()
                                                && let Some(value) =
                                                    rack_parameter.value_for_choice(index)
                                            {
                                                rack_parameter.set_value(value, None);
                                                requests.edits.push((
                                                    plugin_index,
                                                    rack_parameter.id,
                                                    value,
                                                ));
                                            }
                                        } else {
                                            let mut value = rack_parameter.value;
                                            let mut slider = egui::Slider::new(
                                                &mut value,
                                                rack_parameter.min..=rack_parameter.max,
                                            )
                                            .show_value(false);
                                            if rack_parameter.is_discrete() {
                                                let steps = f64::from(rack_parameter.step_count);
                                                slider = slider.step_by(
                                                    (rack_parameter.max - rack_parameter.min)
                                                        / steps,
                                                );
                                            }
                                            let response = ui.add_enabled_ui(
                                                !rack_parameter.is_read_only,
                                                |ui| {
                                                    ui.add_sized(
                                                        [
                                                            control_width,
                                                            CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT,
                                                        ],
                                                        slider,
                                                    )
                                                },
                                            );
                                            if response.inner.changed() {
                                                rack_parameter.value = value;
                                                rack_parameter.formatted_value =
                                                    format_custom_editor_float(value);
                                                requests.edits.push((
                                                    plugin_index,
                                                    rack_parameter.id,
                                                    value,
                                                ));
                                            }
                                        }
                                        let display_value = if is_booleanish_parameter(
                                            rack_parameter,
                                            &rack_parameter.formatted_value,
                                        ) || !rack_parameter.choices.is_empty() {
                                            rack_parameter.formatted_value.clone()
                                        } else {
                                            format_custom_editor_float(rack_parameter.value)
                                        };
                                        ui.add_sized(
                                            [value_width, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                                            Label::new(display_value),
                                        );
                                        let learning = self.midi_learn_plugin == Some(plugin_index)
                                            && self.midi_learn_parameter == Some(parameter_id);
                                        let midi_label = if learning {
                                            "..."
                                        } else if midi_mapping_label.is_some() {
                                            "CC"
                                        } else {
                                            "M"
                                        };
                                        let midi_tooltip = if learning {
                                            "Cancel MIDI learn".to_owned()
                                        } else if let Some(label) = midi_mapping_label.as_deref() {
                                            format!("{label}; learn a different control")
                                        } else {
                                            "Learn a MIDI CC for this parameter".to_owned()
                                        };
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 2.0;
                                            let midi_response = ui
                                                .add_enabled_ui(
                                                    !rack_parameter.is_read_only || learning,
                                                    |ui| ui.add_sized(
                                                        [24.0, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                                                        egui::Button::new(midi_label),
                                                    ),
                                                )
                                                .inner
                                                .on_hover_text(midi_tooltip);
                                            if midi_response.clicked() {
                                                if learning {
                                                    cancel_parameter_learn_requested = true;
                                                } else {
                                                    *requests.parameter_learn_requested =
                                                        Some((plugin_index, parameter_id));
                                                }
                                            }
                                            let remove_response = ui
                                                .add_enabled_ui(midi_mapping_label.is_some(), |ui| {
                                                    ui.add_sized(
                                                        [18.0, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                                                        egui::Button::new("×"),
                                                    )
                                                })
                                                .inner
                                                .on_hover_text("Remove MIDI control");
                                            if remove_response.clicked() {
                                                requests.parameter_mapping_remove_requested.push((
                                                    plugin_index,
                                                    parameter_id,
                                                ));
                                            }
                                        });
                                        ui.end_row();
                                    }
                                });
                            }
                        });
                    });
                }
            });
    }
}
