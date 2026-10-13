use super::super::ui::{Button as UiButton, ButtonSize, ButtonType, Theme};

const CHIP_FONT_SIZE: f32 = 13.0;
const CHIP_HORIZONTAL_PADDING: f32 = 12.0;
const CHECKBOX_ICON_SIZE: f32 = 14.0;
const CHECKBOX_ICON_GAP: f32 = 6.0;
const MINIMUM_CHIP_WIDTH: f32 = 40.0;
const CHIP_WIDTH_GUARD: f32 = 2.0;

#[derive(Clone, Copy)]
enum EffectTypeFilterChip<'a> {
    EffectType(&'a str),
    Clear,
}

impl<'a> EffectTypeFilterChip<'a> {
    fn label(self) -> &'a str {
        match self {
            Self::EffectType(effect_type) => effect_type,
            Self::Clear => "Clear",
        }
    }

    fn horizontal_decoration_width(self) -> f32 {
        let button_padding = CHIP_HORIZONTAL_PADDING * 2.0;
        match self {
            Self::EffectType(_) => button_padding + CHECKBOX_ICON_SIZE + CHECKBOX_ICON_GAP,
            Self::Clear => button_padding,
        }
    }
}

pub(super) fn show_effect_type_filter(
    ui: &mut egui::Ui,
    theme: &Theme,
    effect_types: &[String],
    selected_effect_types: &mut Vec<String>,
) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(16, 0))
        .show(ui, |filter_ui| {
            filter_ui.spacing_mut().item_spacing.y = 4.0;
            let filter_width = filter_ui.available_width().max(0.0);
            filter_ui.take_available_width();

            let mut chips = effect_types
                .iter()
                .map(|effect_type| EffectTypeFilterChip::EffectType(effect_type))
                .collect::<Vec<_>>();
            if !selected_effect_types.is_empty() {
                chips.push(EffectTypeFilterChip::Clear);
            }

            let rows = pack_filter_chips(filter_ui, &chips, filter_width);
            for row in rows {
                filter_ui.horizontal(|row_ui| {
                    for (chip, label) in row {
                        match chip {
                            EffectTypeFilterChip::EffectType(effect_type) => {
                                let mut checked = selected_effect_types
                                    .iter()
                                    .any(|selected| selected == effect_type);
                                let response = row_ui.checkbox(&mut checked, label);
                                let clicked = response.clicked();
                                response.on_hover_text(effect_type);

                                if clicked {
                                    if checked {
                                        selected_effect_types.push(effect_type.to_owned());
                                    } else {
                                        selected_effect_types
                                            .retain(|selected| selected != effect_type);
                                    }
                                }
                            }
                            EffectTypeFilterChip::Clear => {
                                if UiButton::new(label)
                                    .button_type(ButtonType::Link)
                                    .size(ButtonSize::Sm)
                                    .show(row_ui, theme)
                                    .clicked()
                                {
                                    selected_effect_types.clear();
                                }
                            }
                        }
                    }
                });
            }
        });
}

fn pack_filter_chips<'a>(
    ui: &egui::Ui,
    chips: &[EffectTypeFilterChip<'a>],
    available_width: f32,
) -> Vec<Vec<(EffectTypeFilterChip<'a>, String)>> {
    let spacing = ui.spacing().item_spacing.x;
    let labels = chips
        .iter()
        .map(|chip| {
            let label_width_limit =
                (available_width - chip.horizontal_decoration_width() - CHIP_WIDTH_GUARD).max(0.0);
            fit_label(ui, chip.label(), label_width_limit)
        })
        .collect::<Vec<_>>();
    let chip_widths = chips
        .iter()
        .zip(&labels)
        .map(|(chip, label)| {
            (measured_text_width(ui, label) + chip.horizontal_decoration_width())
                .max(MINIMUM_CHIP_WIDTH)
                + CHIP_WIDTH_GUARD
        })
        .collect::<Vec<_>>();

    split_chip_rows(&chip_widths, spacing, available_width)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|index| (chips[index], labels[index].clone()))
                .collect()
        })
        .collect()
}

fn split_chip_rows(chip_widths: &[f32], spacing: f32, available_width: f32) -> Vec<Vec<usize>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut row_width = 0.0;

    for (index, chip_width) in chip_widths.iter().copied().enumerate() {
        let next_row_width = row_width + if row.is_empty() { 0.0 } else { spacing } + chip_width;
        if !row.is_empty() && next_row_width > available_width {
            rows.push(std::mem::take(&mut row));
            row_width = 0.0;
        }

        if !row.is_empty() {
            row_width += spacing;
        }
        row_width += chip_width;
        row.push(index);
    }

    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn fit_label(ui: &egui::Ui, label: &str, max_width: f32) -> String {
    if measured_text_width(ui, label) <= max_width {
        return label.to_owned();
    }

    let mut fitted = String::new();
    for character in label.chars() {
        let mut candidate = fitted.clone();
        candidate.push(character);
        candidate.push('…');
        if measured_text_width(ui, &candidate) > max_width {
            break;
        }
        fitted.push(character);
    }
    fitted.push('…');
    fitted
}

fn measured_text_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(CHIP_FONT_SIZE),
                egui::Color32::WHITE,
            )
            .size()
            .x
    })
}

#[cfg(test)]
mod tests {
    use super::split_chip_rows;

    #[test]
    fn filter_chips_break_before_exceeding_the_available_width() {
        let available_width = 200.0;
        let spacing = 8.0;
        let chip_widths = [90.0, 80.0, 75.0, 60.0, 140.0];

        let rows = split_chip_rows(&chip_widths, spacing, available_width);

        assert!(rows.len() > 1);
        for row in rows {
            let row_width = row.iter().map(|index| chip_widths[*index]).sum::<f32>()
                + spacing * row.len().saturating_sub(1) as f32;
            assert!(row_width <= available_width);
        }
    }
}
