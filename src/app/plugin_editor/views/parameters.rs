use super::super::*;
use super::parameter_layout::{
    PARAMETER_TABLE_HEADER_HEIGHT, ParameterColumnLayout, ParameterTableWidths, fixed_cell_rect,
    parameter_column_height, parameter_column_layout, parameter_row_height,
    parameter_table_headers, show_truncated_label,
};
use super::{PluginParameterUiData, PluginUiRequests};

impl PluginEditor {
    pub(super) fn show_parameter_groups(
        &mut self,
        ui: &mut egui::Ui,
        plugin_index: usize,
        theme: &Theme,
        requests: &mut PluginUiRequests<'_>,
    ) {
        let groups = group_parameter_indices(&self.rack.slots[plugin_index].snapshot.parameters);

        for (group_index, (unit, parameter_indices)) in groups.into_iter().enumerate() {
            let column_ranges =
                balanced_column_ranges(parameter_indices.len(), CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT);

            ui.add_space(4.0);
            egui::Frame::group(ui.style()).show(ui, |group_ui| {
                let group_width = group_ui.available_width().max(0.0);
                group_ui.set_width(group_width);
                group_ui
                    .add_sized(
                        [group_width, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                        egui::Label::new(egui::RichText::new(&unit).strong()).truncate(),
                    )
                    .on_hover_text(&unit);
                group_ui.add_space(4.0);

                let vertical_spacing = group_ui.spacing().item_spacing.y.max(0.0);
                let column_gap = group_ui
                    .spacing()
                    .item_spacing
                    .x
                    .max(0.0)
                    .min(group_width / 3.0);
                let column_width = ((group_width - column_gap).max(0.0)) / 2.0;
                let layout =
                    parameter_column_layout(column_width, group_ui.spacing().item_spacing.x);
                let max_parameter_count = column_ranges
                    .iter()
                    .map(|(start, end)| end - start)
                    .max()
                    .unwrap_or_default();
                let columns_height =
                    parameter_column_height(max_parameter_count, layout, vertical_spacing);
                let (_, columns_rect) =
                    group_ui.allocate_space(egui::vec2(group_width, columns_height));
                let group_clip_rect = group_ui.clip_rect();

                for (column_index, (start, end)) in column_ranges.into_iter().enumerate() {
                    let column_left =
                        columns_rect.left() + column_index as f32 * (column_width + column_gap);
                    let column_rect = egui::Rect::from_min_size(
                        egui::pos2(column_left, columns_rect.top()),
                        egui::vec2(column_width, columns_height),
                    );
                    let mut column_ui = group_ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt((
                                "plugin-parameter-column",
                                plugin_index,
                                group_index,
                                column_index,
                            ))
                            .max_rect(column_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    column_ui.set_clip_rect(group_clip_rect.intersect(column_rect));
                    self.show_parameter_column(
                        &mut column_ui,
                        column_rect,
                        plugin_index,
                        group_index,
                        column_index,
                        &parameter_indices[start..end],
                        layout,
                        vertical_spacing,
                        theme,
                        requests,
                    );
                }
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_parameter_column(
        &mut self,
        ui: &mut egui::Ui,
        column_rect: egui::Rect,
        plugin_index: usize,
        group_index: usize,
        column_index: usize,
        parameter_indices: &[usize],
        layout: ParameterColumnLayout,
        vertical_spacing: f32,
        theme: &Theme,
        requests: &mut PluginUiRequests<'_>,
    ) {
        if parameter_indices.is_empty() {
            return;
        }

        let mut top = column_rect.top();
        if let ParameterColumnLayout::Table(widths) = layout {
            self.show_parameter_table_header(
                ui,
                column_rect,
                top,
                plugin_index,
                group_index,
                column_index,
                widths,
            );
            top += PARAMETER_TABLE_HEADER_HEIGHT + vertical_spacing;
        }

        let row_height = parameter_row_height(layout, vertical_spacing);
        for (parameter_position, &parameter_index) in parameter_indices.iter().enumerate() {
            let row_rect = egui::Rect::from_min_size(
                egui::pos2(column_rect.left(), top),
                egui::vec2(column_rect.width(), row_height),
            );
            self.show_parameter_row(
                ui,
                row_rect,
                plugin_index,
                group_index,
                column_index,
                parameter_index,
                layout,
                theme,
                requests,
            );
            top += row_height;
            if parameter_position + 1 < parameter_indices.len() {
                top += vertical_spacing;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_parameter_table_header(
        &self,
        ui: &mut egui::Ui,
        column_rect: egui::Rect,
        top: f32,
        plugin_index: usize,
        group_index: usize,
        column_index: usize,
        widths: ParameterTableWidths,
    ) {
        let header_rect = egui::Rect::from_min_size(
            egui::pos2(column_rect.left(), top),
            egui::vec2(column_rect.width(), PARAMETER_TABLE_HEADER_HEIGHT),
        );
        let headers = parameter_table_headers(column_rect.width());
        let cell_widths = [widths.name, widths.control, widths.value, widths.midi];
        let mut offset_x = 0.0;

        for (cell_index, (header, cell_width)) in headers.into_iter().zip(cell_widths).enumerate() {
            let cell_rect = fixed_cell_rect(header_rect, offset_x, cell_width);
            show_truncated_label(
                ui,
                cell_rect,
                (
                    "plugin-parameter-header",
                    plugin_index,
                    group_index,
                    column_index,
                    cell_index,
                ),
                egui::Label::new(header),
            );
            offset_x += cell_width + widths.spacing;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_parameter_row(
        &mut self,
        ui: &mut egui::Ui,
        row_rect: egui::Rect,
        plugin_index: usize,
        group_index: usize,
        column_index: usize,
        parameter_index: usize,
        layout: ParameterColumnLayout,
        theme: &Theme,
        requests: &mut PluginUiRequests<'_>,
    ) {
        let data = {
            let slot = &self.rack.slots[plugin_index];
            let parameter = &slot.snapshot.parameters[parameter_index];
            let midi_mapping_label = slot
                .midi_mappings
                .iter()
                .find(|mapping| {
                    matches!(
                        mapping.action,
                        super::super::super::midi::MidiAction::SetParameter {
                            parameter_id
                        } if parameter_id == parameter.id
                    )
                })
                .map(|mapping| mapping.label());
            PluginParameterUiData {
                id: parameter.id,
                name: parameter.name.clone(),
                is_read_only: parameter.is_read_only,
                is_learning: self.midi_learn_plugin == Some(plugin_index)
                    && self.midi_learn_parameter == Some(parameter.id),
                midi_mapping_label,
            }
        };
        let id_salt = (
            "plugin-parameter-row",
            plugin_index,
            group_index,
            column_index,
            data.id,
        );

        match layout {
            ParameterColumnLayout::Table(widths) => {
                self.show_table_parameter_row(
                    ui,
                    row_rect,
                    plugin_index,
                    parameter_index,
                    data,
                    id_salt,
                    widths,
                    theme,
                    requests,
                );
            }
            ParameterColumnLayout::Stacked {
                inline_midi_actions,
            } => {
                self.show_stacked_parameter_row(
                    ui,
                    row_rect,
                    plugin_index,
                    parameter_index,
                    data,
                    id_salt,
                    inline_midi_actions,
                    theme,
                    requests,
                );
            }
        }
    }
}
