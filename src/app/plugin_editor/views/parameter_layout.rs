use super::super::CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT;

#[derive(Clone, Copy)]
pub(super) struct ParameterTableWidths {
    pub(super) name: f32,
    pub(super) control: f32,
    pub(super) value: f32,
    pub(super) midi: f32,
    pub(super) spacing: f32,
}

#[derive(Clone, Copy)]
pub(super) enum ParameterColumnLayout {
    Table(ParameterTableWidths),
    Stacked { inline_midi_actions: bool },
}

pub(super) const COMPACT_PARAMETER_TABLE_WIDTH: f32 = 260.0;
pub(super) const MIN_PARAMETER_CONTROL_WIDTH: f32 = 40.0;
pub(super) const PARAMETER_MIDI_CONTROLS_WIDTH: f32 = 44.0;
pub(super) const PARAMETER_MIDI_LEARN_BUTTON_WIDTH: f32 = 24.0;
pub(super) const PARAMETER_MIDI_REMOVE_BUTTON_WIDTH: f32 = 18.0;
pub(super) const PARAMETER_MIDI_CONTROLS_SPACING: f32 = 2.0;
pub(super) const PARAMETER_TABLE_HEADER_HEIGHT: f32 = 20.0;
pub(super) const STACKED_PARAMETER_LABEL_HEIGHT: f32 = 20.0;
pub(super) const STACKED_PARAMETER_VALUE_HEIGHT: f32 = 20.0;

pub(super) fn parameter_table_widths(
    available_width: f32,
    default_spacing: f32,
) -> Option<ParameterTableWidths> {
    let available_width = available_width.max(0.0);
    if !available_width.is_finite() {
        return None;
    }

    let compact = available_width < COMPACT_PARAMETER_TABLE_WIDTH;
    let spacing = if compact {
        PARAMETER_MIDI_CONTROLS_SPACING
    } else {
        default_spacing.max(0.0)
    };
    let name = available_width * if compact { 0.22 } else { 0.26 };
    let value = if compact {
        available_width * 0.22
    } else {
        56.0_f32.min(available_width * 0.28)
    };
    let control = available_width - name - value - PARAMETER_MIDI_CONTROLS_WIDTH - spacing * 3.0;

    (control >= MIN_PARAMETER_CONTROL_WIDTH).then_some(ParameterTableWidths {
        name,
        control,
        value,
        midi: PARAMETER_MIDI_CONTROLS_WIDTH,
        spacing,
    })
}

pub(super) fn parameter_column_layout(
    available_width: f32,
    default_spacing: f32,
) -> ParameterColumnLayout {
    parameter_table_widths(available_width, default_spacing).map_or(
        ParameterColumnLayout::Stacked {
            inline_midi_actions: available_width >= PARAMETER_MIDI_CONTROLS_WIDTH,
        },
        ParameterColumnLayout::Table,
    )
}

pub(super) fn parameter_row_height(layout: ParameterColumnLayout, vertical_spacing: f32) -> f32 {
    match layout {
        ParameterColumnLayout::Table(_) => CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT,
        ParameterColumnLayout::Stacked {
            inline_midi_actions,
        } => {
            let midi_action_rows = if inline_midi_actions { 1.0 } else { 2.0 };
            let component_count = 3.0 + midi_action_rows;
            STACKED_PARAMETER_LABEL_HEIGHT
                + CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT
                + STACKED_PARAMETER_VALUE_HEIGHT
                + midi_action_rows * CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT
                + (component_count - 1.0) * vertical_spacing.max(0.0)
        }
    }
}

pub(super) fn parameter_column_height(
    parameter_count: usize,
    layout: ParameterColumnLayout,
    vertical_spacing: f32,
) -> f32 {
    if parameter_count == 0 {
        return 0.0;
    }

    let vertical_spacing = vertical_spacing.max(0.0);
    let header_height = match layout {
        ParameterColumnLayout::Table(_) => PARAMETER_TABLE_HEADER_HEIGHT + vertical_spacing,
        ParameterColumnLayout::Stacked { .. } => 0.0,
    };
    header_height
        + parameter_count as f32 * parameter_row_height(layout, vertical_spacing)
        + parameter_count.saturating_sub(1) as f32 * vertical_spacing
}

pub(super) fn parameter_table_headers(column_width: f32) -> [&'static str; 4] {
    if column_width < 200.0 {
        ["P", "C", "V", "M"]
    } else if column_width < COMPACT_PARAMETER_TABLE_WIDTH {
        ["Param", "Ctrl", "Value", "MIDI"]
    } else {
        ["Parameter", "Control", "Value", "MIDI"]
    }
}

pub(super) fn fixed_child_ui(
    parent: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    layout: egui::Layout,
) -> egui::Ui {
    let clip_rect = parent.clip_rect().intersect(rect);
    let mut child = parent.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(layout),
    );
    child.set_clip_rect(clip_rect);
    child
}

pub(super) fn fixed_cell_rect(row_rect: egui::Rect, offset_x: f32, width: f32) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(row_rect.left() + offset_x, row_rect.top()),
        egui::vec2(width.max(0.0), row_rect.height()),
    )
}

pub(super) fn show_truncated_label(
    parent: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    label: egui::Label,
) -> egui::Response {
    let mut label_ui = fixed_child_ui(
        parent,
        rect,
        id_salt,
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
    );
    label_ui.add_sized(rect.size(), label.truncate())
}

#[cfg(test)]
mod tests {
    use super::{
        ParameterColumnLayout, parameter_column_height, parameter_column_layout,
        parameter_table_headers, parameter_table_widths,
    };
    use crate::app::plugin_editor::CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT;
    use crate::app::plugin_editor::formatting::balanced_column_ranges;

    #[test]
    fn custom_plugin_parameters_are_always_split_into_two_balanced_columns() {
        assert_eq!(CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT, 2);
        assert_eq!(
            balanced_column_ranges(0, CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT),
            [(0, 0), (0, 0)]
        );
        assert_eq!(
            balanced_column_ranges(1, CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT),
            [(0, 1), (1, 1)]
        );
        assert_eq!(
            balanced_column_ranges(6, CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT),
            [(0, 3), (3, 6)]
        );
        assert_eq!(
            balanced_column_ranges(7, CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT),
            [(0, 4), (4, 7)]
        );
    }

    #[test]
    fn table_column_widths_never_exceed_their_available_width() {
        for available_width in [164.0, 180.0, 260.0, 480.0, 960.0] {
            let widths = parameter_table_widths(available_width, 8.0)
                .expect("test width should allow a horizontal parameter row");
            let used_width =
                widths.name + widths.control + widths.value + widths.midi + widths.spacing * 3.0;
            assert!((used_width - available_width).abs() < 0.01);
        }
    }

    #[test]
    fn narrow_columns_stack_controls_instead_of_overflowing() {
        assert!(parameter_table_widths(120.0, 8.0).is_none());
        let layout = parameter_column_layout(120.0, 8.0);
        assert!(matches!(layout, ParameterColumnLayout::Stacked { .. }));
        assert!(parameter_column_height(3, layout, 4.0) > 3.0 * 24.0);
        assert_eq!(parameter_table_headers(180.0), ["P", "C", "V", "M"]);
    }
}
