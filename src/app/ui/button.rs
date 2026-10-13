use egui::{Color32, Response, WidgetText, vec2};

use super::theme::Theme;

type ButtonIcon<'a> = &'a dyn Fn(&egui::Painter, egui::Pos2, f32, Color32);

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ButtonSize {
    Sm,
    #[default]
    Default,
    Lg,
    Icon,
    IconSm,
    IconLg,
}

impl ButtonSize {
    pub(crate) fn height(self) -> f32 {
        match self {
            Self::Sm | Self::IconSm => 32.0,
            Self::Default | Self::Icon => 36.0,
            Self::Lg | Self::IconLg => 40.0,
        }
    }

    pub(crate) fn icon_size(self) -> f32 {
        match self {
            Self::Sm | Self::IconSm => 14.0,
            Self::Default | Self::Icon => 16.0,
            Self::Lg | Self::IconLg => 18.0,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ButtonJustify {
    Start,
    #[default]
    Center,
    Between,
}

/// Semantic button roles kept for call-site readability. egui supplies their appearance.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ButtonType {
    #[default]
    Primary,
    Secondary,
    Outline,
    BypassOn,
    BypassOff,
    Danger,
    Ghost,
    Menu,
    Link,
}

pub(crate) use ButtonType as ButtonKind;

/// App button wrapper that preserves sizing and alignment while inheriting egui visuals.
pub(crate) struct Button<'a> {
    label: WidgetText,
    button_type: ButtonType,
    size: ButtonSize,
    enabled: bool,
    selected: bool,
    min_width: f32,
    justify: ButtonJustify,
    icon: Option<ButtonIcon<'a>>,
}

impl<'a> Button<'a> {
    pub(crate) fn new(label: impl Into<WidgetText>) -> Self {
        Self {
            label: label.into(),
            button_type: ButtonType::default(),
            size: ButtonSize::Default,
            enabled: true,
            selected: false,
            min_width: 0.0,
            justify: ButtonJustify::Center,
            icon: None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn label(label: impl Into<WidgetText>) -> Self {
        Self::new(label)
    }

    pub(crate) fn button_type(mut self, button_type: ButtonType) -> Self {
        self.button_type = button_type;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn kind(self, button_type: ButtonKind) -> Self {
        self.button_type(button_type)
    }

    pub(crate) fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub(crate) fn width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    pub(crate) fn min_width(self, width: f32) -> Self {
        self.width(width)
    }

    pub(crate) fn justify(mut self, justify: ButtonJustify) -> Self {
        self.justify = justify;
        self
    }

    pub(crate) fn icon(mut self, icon: ButtonIcon<'a>) -> Self {
        self.icon = Some(icon);
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, theme: &Theme) -> Response {
        let bypass_colors = match self.button_type {
            ButtonType::BypassOn => Some(theme.bypass_colors(true)),
            ButtonType::BypassOff => Some(theme.bypass_colors(false)),
            _ => None,
        };
        let selected = self.selected || bypass_colors.is_some();
        let height = self.size.height();
        let available_width = ui.available_width().max(0.0);
        let minimum_width = if self.icon.is_some() {
            self.min_width.max(height)
        } else {
            self.min_width
        };
        let desired_size = vec2(minimum_width.min(available_width), height);
        let button = match self.justify {
            ButtonJustify::Start => egui::Button::new((self.label, egui::Atom::grow())),
            ButtonJustify::Center | ButtonJustify::Between => {
                egui::Button::new((egui::Atom::grow(), self.label, egui::Atom::grow()))
            }
        };

        let response = ui
            .scope(|button_ui| {
                button_ui.spacing_mut().icon_spacing = 0.0;
                if let Some((background, foreground)) = bypass_colors {
                    let selection = &mut button_ui.visuals_mut().selection;
                    selection.bg_fill = background;
                    selection.stroke = egui::Stroke::new(1.0_f32, foreground);
                }
                button_ui.add_enabled(
                    self.enabled,
                    button.min_size(desired_size).truncate().selected(selected),
                )
            })
            .inner;

        if let Some(icon) = self.icon {
            let icon_color = bypass_colors.map_or_else(
                || ui.style().interact(&response).text_color(),
                |(_, foreground)| foreground,
            );
            icon(
                ui.painter(),
                response.rect.center(),
                self.size.icon_size(),
                icon_color,
            );
        }

        response
    }
}

pub(crate) fn button(ui: &mut egui::Ui, theme: &Theme, label: impl Into<WidgetText>) -> Response {
    Button::new(label).show(ui, theme)
}

pub(crate) fn button_with_type(
    ui: &mut egui::Ui,
    theme: &Theme,
    label: impl Into<WidgetText>,
    button_type: ButtonType,
) -> Response {
    Button::new(label).button_type(button_type).show(ui, theme)
}
