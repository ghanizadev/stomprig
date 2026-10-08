use egui::{Response, RichText};
use lucide_icons::Icon;

use super::button::{Button, ButtonSize, Theme};

pub(crate) fn square_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    icon: Icon,
    enabled: bool,
    tooltip: &str,
) -> Response {
    let label = RichText::new(icon.unicode().to_owned())
        .family(egui::FontFamily::Name("lucide".into()))
        .size(16.0);
    Button::new(label)
        .size(ButtonSize::IconSm)
        .enabled(enabled)
        .show(ui, theme)
        .on_hover_text(tooltip)
}
