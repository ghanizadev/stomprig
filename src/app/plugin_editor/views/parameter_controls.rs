use super::super::*;
use super::parameter_layout::{
    PARAMETER_MIDI_CONTROLS_SPACING, PARAMETER_MIDI_LEARN_BUTTON_WIDTH,
    PARAMETER_MIDI_REMOVE_BUTTON_WIDTH, ParameterTableWidths, STACKED_PARAMETER_LABEL_HEIGHT,
    STACKED_PARAMETER_VALUE_HEIGHT, fixed_cell_rect, fixed_child_ui, show_truncated_label,
};
use super::{PluginParameterUiData, PluginUiRequests};

impl PluginEditor {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn show_table_parameter_row(
        &mut self,
        ui: &mut egui::Ui,
        row_rect: egui::Rect,
        plugin_index: usize,
        parameter_index: usize,
        data: PluginParameterUiData,
        id_salt: impl std::hash::Hash + Copy,
        widths: ParameterTableWidths,
        theme: &Theme,
        requests: &mut PluginUiRequests<'_>,
    ) {
        let mut offset_x = 0.0;
        let name_rect = fixed_cell_rect(row_rect, offset_x, widths.name);
        show_truncated_label(
            ui,
            name_rect,
            (id_salt, "name"),
            egui::Label::new(data.name.clone()),
        )
        .on_hover_text(data.name.clone());
        offset_x += widths.name + widths.spacing;

        let control_rect = fixed_cell_rect(row_rect, offset_x, widths.control);
        self.show_parameter_control(
            ui,
            control_rect,
            (id_salt, "control"),
            plugin_index,
            parameter_index,
            theme,
            requests,
        );
        offset_x += widths.control + widths.spacing;

        let value_rect = fixed_cell_rect(row_rect, offset_x, widths.value);
        self.show_parameter_value(
            ui,
            value_rect,
            (id_salt, "value"),
            plugin_index,
            parameter_index,
        );
        offset_x += widths.value + widths.spacing;

        let midi_rect = fixed_cell_rect(row_rect, offset_x, widths.midi);
        self.show_parameter_midi_controls(
            ui,
            midi_rect,
            (id_salt, "midi"),
            plugin_index,
            &data,
            requests,
            true,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn show_stacked_parameter_row(
        &mut self,
        ui: &mut egui::Ui,
        row_rect: egui::Rect,
        plugin_index: usize,
        parameter_index: usize,
        data: PluginParameterUiData,
        id_salt: impl std::hash::Hash + Copy,
        inline_midi_actions: bool,
        theme: &Theme,
        requests: &mut PluginUiRequests<'_>,
    ) {
        let mut top = row_rect.top();
        let name_rect = egui::Rect::from_min_size(
            egui::pos2(row_rect.left(), top),
            egui::vec2(row_rect.width(), STACKED_PARAMETER_LABEL_HEIGHT),
        );
        show_truncated_label(
            ui,
            name_rect,
            (id_salt, "name"),
            egui::Label::new(data.name.clone()),
        )
        .on_hover_text(data.name.clone());
        top += STACKED_PARAMETER_LABEL_HEIGHT + ui.spacing().item_spacing.y.max(0.0);

        let control_rect = egui::Rect::from_min_size(
            egui::pos2(row_rect.left(), top),
            egui::vec2(row_rect.width(), CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT),
        );
        self.show_parameter_control(
            ui,
            control_rect,
            (id_salt, "control"),
            plugin_index,
            parameter_index,
            theme,
            requests,
        );
        top += CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT + ui.spacing().item_spacing.y.max(0.0);

        let value_rect = egui::Rect::from_min_size(
            egui::pos2(row_rect.left(), top),
            egui::vec2(row_rect.width(), STACKED_PARAMETER_VALUE_HEIGHT),
        );
        self.show_parameter_value(
            ui,
            value_rect,
            (id_salt, "value"),
            plugin_index,
            parameter_index,
        );
        top += STACKED_PARAMETER_VALUE_HEIGHT + ui.spacing().item_spacing.y.max(0.0);

        let midi_height = if inline_midi_actions {
            CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT
        } else {
            CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT * 2.0 + ui.spacing().item_spacing.y.max(0.0)
        };
        let midi_rect = egui::Rect::from_min_size(
            egui::pos2(row_rect.left(), top),
            egui::vec2(row_rect.width(), midi_height),
        );
        self.show_parameter_midi_controls(
            ui,
            midi_rect,
            (id_salt, "midi"),
            plugin_index,
            &data,
            requests,
            inline_midi_actions,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn show_parameter_control(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        id_salt: impl std::hash::Hash,
        plugin_index: usize,
        parameter_index: usize,
        theme: &Theme,
        requests: &mut PluginUiRequests<'_>,
    ) {
        let mut control_ui = fixed_child_ui(
            ui,
            rect,
            id_salt,
            egui::Layout::centered_and_justified(egui::Direction::TopDown),
        );
        let parameter = &mut self.rack.slots[plugin_index].snapshot.parameters[parameter_index];
        let control_size = rect.size();

        if is_booleanish_parameter(parameter, &parameter.formatted_value) {
            let mut checked = boolean_parameter_value(parameter, &parameter.formatted_value);
            let response = control_ui
                .add_enabled_ui(!parameter.is_read_only, |ui| {
                    ui.add_sized(control_size, egui::Checkbox::without_text(&mut checked))
                })
                .inner;
            if response.changed() {
                parameter.value = if checked {
                    parameter.max
                } else {
                    parameter.min
                };
                parameter.formatted_value = parameter.value.to_string();
                requests
                    .edits
                    .push((plugin_index, parameter.id, parameter.value));
            }
        } else if !parameter.choices.is_empty() {
            let mut selected_choice = Some(
                parameter
                    .selected_choice_index()
                    .unwrap_or_default()
                    .to_string(),
            );
            let choice_items = parameter
                .choices
                .iter()
                .enumerate()
                .map(|(index, label)| SelectItem::option(index.to_string(), label))
                .collect::<Vec<_>>();
            let response = Select::new(
                ("plugin-parameter-choice", plugin_index, parameter.id),
                &mut selected_choice,
                &choice_items,
            )
            .size(SelectSize::Size1)
            .width(rect.width())
            .enabled(!parameter.is_read_only)
            .show(&mut control_ui, theme);
            if response.changed()
                && let Some(selected_choice) = selected_choice
                && let Ok(index) = selected_choice.parse::<usize>()
                && let Some(value) = parameter.value_for_choice(index)
            {
                parameter.set_value(value, None);
                requests.edits.push((plugin_index, parameter.id, value));
            }
        } else {
            let mut value = parameter.value;
            let mut slider =
                egui::Slider::new(&mut value, parameter.min..=parameter.max).show_value(false);
            if parameter.is_discrete() {
                let steps = f64::from(parameter.step_count);
                slider = slider.step_by((parameter.max - parameter.min) / steps);
            }
            let response = control_ui
                .add_enabled_ui(!parameter.is_read_only, |ui| {
                    ui.add_sized(control_size, slider)
                })
                .inner;
            if response.changed() {
                parameter.value = value;
                parameter.formatted_value = format_custom_editor_float(value);
                requests.edits.push((plugin_index, parameter.id, value));
            }
        }
    }

    fn show_parameter_value(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        id_salt: impl std::hash::Hash,
        plugin_index: usize,
        parameter_index: usize,
    ) {
        let parameter = &self.rack.slots[plugin_index].snapshot.parameters[parameter_index];
        let display_value = if is_booleanish_parameter(parameter, &parameter.formatted_value)
            || !parameter.choices.is_empty()
        {
            parameter.formatted_value.clone()
        } else {
            format_custom_editor_float(parameter.value)
        };
        show_truncated_label(ui, rect, id_salt, egui::Label::new(display_value.clone()))
            .on_hover_text(display_value);
    }

    #[allow(clippy::too_many_arguments)]
    fn show_parameter_midi_controls(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        id_salt: impl std::hash::Hash,
        plugin_index: usize,
        data: &PluginParameterUiData,
        requests: &mut PluginUiRequests<'_>,
        inline_midi_actions: bool,
    ) {
        let controls_layout = if inline_midi_actions {
            egui::Layout::left_to_right(egui::Align::Center)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        };
        let mut controls_ui = fixed_child_ui(ui, rect, id_salt, controls_layout);
        controls_ui.spacing_mut().item_spacing.x = PARAMETER_MIDI_CONTROLS_SPACING;
        let learn_label = if data.is_learning {
            "..."
        } else if data.midi_mapping_label.is_some() {
            "CC"
        } else {
            "M"
        };
        let learn_tooltip = if data.is_learning {
            "Cancel MIDI learn".to_owned()
        } else if let Some(label) = data.midi_mapping_label.as_deref() {
            format!("{label}; learn a different control")
        } else {
            "Learn a MIDI CC for this parameter".to_owned()
        };
        let learn_button_width = if inline_midi_actions {
            PARAMETER_MIDI_LEARN_BUTTON_WIDTH
        } else {
            rect.width()
        };
        let learn_response = controls_ui
            .add_enabled_ui(!data.is_read_only || data.is_learning, |ui| {
                ui.add_sized(
                    [learn_button_width, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                    egui::Button::new(learn_label),
                )
            })
            .inner
            .on_hover_text(learn_tooltip);
        if learn_response.clicked() {
            if data.is_learning {
                *requests.parameter_learn_cancel_requested = true;
            } else {
                *requests.parameter_learn_requested = Some((plugin_index, data.id));
            }
        }

        if inline_midi_actions {
            let remove_response = controls_ui
                .add_enabled_ui(data.midi_mapping_label.is_some(), |ui| {
                    ui.add_sized(
                        [
                            PARAMETER_MIDI_REMOVE_BUTTON_WIDTH,
                            CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT,
                        ],
                        egui::Button::new("×"),
                    )
                })
                .inner
                .on_hover_text("Remove MIDI control");
            if remove_response.clicked() {
                requests
                    .parameter_mapping_remove_requested
                    .push((plugin_index, data.id));
            }
        } else {
            let remove_response = controls_ui
                .add_enabled_ui(data.midi_mapping_label.is_some(), |ui| {
                    ui.add_sized(
                        [rect.width(), CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                        egui::Button::new("×"),
                    )
                })
                .inner
                .on_hover_text("Remove MIDI control");
            if remove_response.clicked() {
                requests
                    .parameter_mapping_remove_requested
                    .push((plugin_index, data.id));
            }
        }
    }
}
