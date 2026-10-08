use egui::{
    Color32, CornerRadius, Response, Stroke, StrokeKind, TextStyle, WidgetText, pos2, vec2,
};
use egui_shadcn::{ControlSize, ControlVariant};

use super::button::{Button, ButtonJustify, ButtonSize, ButtonType};
use super::theme::Theme;

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

pub(crate) fn menu_button_checkbox(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: impl Into<WidgetText>,
    checked: &mut bool,
) -> Response {
    let checked_state = *checked;
    let checkbox_icon =
        move |painter: &egui::Painter, center: egui::Pos2, size: f32, color: Color32| {
            paint_checkbox(painter, center, size, color, checked_state, theme);
        };

    let mut response = Button::new(label)
        .button_type(ButtonType::Menu)
        .size(ButtonSize::Sm)
        .justify(ButtonJustify::Start)
        .icon(&checkbox_icon)
        .show(ui, theme);

    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }

    response
}

fn paint_checkbox(
    painter: &egui::Painter,
    center: egui::Pos2,
    size: f32,
    color: Color32,
    checked: bool,
    theme: &Theme,
) {
    let rounding = CornerRadius::same(3);
    let rect = egui::Rect::from_center_size(center, vec2(size, size));
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0_f32, color),
        StrokeKind::Inside,
    );

    if checked {
        painter.rect_filled(rect.shrink(1.0), rounding, color);
        let check_color = theme.renderer_theme().palette.background;
        let start = pos2(rect.left() + size * 0.24, center.y);
        let middle = pos2(rect.left() + size * 0.44, rect.bottom() - size * 0.26);
        let end = pos2(rect.right() - size * 0.2, rect.top() + size * 0.24);
        let stroke = Stroke::new(1.5_f32, check_color);
        painter.line_segment([start, middle], stroke);
        painter.line_segment([middle, end], stroke);
    }
}

pub(crate) enum MenuButtonItem<T> {
    Item { label: String, value: T },
    Submenu { label: String, items: Vec<Self> },
}

impl<T> MenuButtonItem<T> {
    pub(crate) fn item(label: impl Into<String>, value: T) -> Self {
        Self::Item {
            label: label.into(),
            value,
        }
    }

    pub(crate) fn submenu(label: impl Into<String>, items: Vec<Self>) -> Self {
        Self::Submenu {
            label: label.into(),
            items,
        }
    }
}

pub(crate) fn menu_button_with_items<T: Clone>(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: impl Into<WidgetText>,
    items: &[MenuButtonItem<T>],
    empty_label: Option<WidgetText>,
) -> Option<T> {
    let mut selected = None;
    styled_menu_button(ui, theme, label, |ui| {
        if items.is_empty() {
            if let Some(empty_label) = &empty_label {
                super::label(ui, empty_label.clone());
            }
        } else {
            show_menu_items(ui, theme, items, &mut selected);
        }
    });
    selected
}

fn styled_menu_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: impl Into<WidgetText>,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let renderer_theme = theme.renderer_theme();
    let control = renderer_theme.control(ControlVariant::Ghost, ControlSize::Sm);

    renderer_theme.scoped(ui, control.widgets, |ui| {
        let mut style = ui.style().as_ref().clone();
        style.spacing.button_padding = control.padding;
        style
            .text_styles
            .insert(TextStyle::Button, control.text_style);
        ui.set_style(style);

        ui.menu_button(label, add_contents);
    });
}

fn show_menu_items<T: Clone>(
    ui: &mut egui::Ui,
    theme: &Theme,
    items: &[MenuButtonItem<T>],
    selected: &mut Option<T>,
) {
    for item in items {
        match item {
            MenuButtonItem::Item { label, value } => {
                if menu_button(ui, theme, label).clicked() {
                    *selected = Some(value.clone());
                    ui.close();
                }
            }
            MenuButtonItem::Submenu { label, items } => {
                show_submenu_button(ui, theme, label, items, selected);
            }
        }
    }
}

fn show_submenu_button<T: Clone>(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: &str,
    items: &[MenuButtonItem<T>],
    selected: &mut Option<T>,
) {
    styled_menu_button(ui, theme, label, |ui| {
        show_menu_items(ui, theme, items, selected)
    });
}
