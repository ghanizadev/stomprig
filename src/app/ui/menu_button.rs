use egui::{Response, WidgetText};

use super::button::{Button, ButtonJustify, ButtonSize, ButtonType, Theme};

pub(crate) fn menu_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: impl Into<WidgetText>,
) -> Response {
    Button::new(label)
        .button_type(ButtonType::Menu)
        .size(ButtonSize::Sm)
        .justify(ButtonJustify::Start)
        .show(ui, theme)
}

pub(crate) fn menu_button_enabled(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: impl Into<WidgetText>,
    enabled: bool,
) -> Response {
    Button::new(label)
        .button_type(ButtonType::Menu)
        .size(ButtonSize::Sm)
        .justify(ButtonJustify::Start)
        .enabled(enabled)
        .show(ui, theme)
}
