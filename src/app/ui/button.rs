use egui::{Color32, CornerRadius, Response, WidgetText};
use egui_shadcn::{
    Button as ShadcnButton, ButtonJustify as ShadcnButtonJustify, ButtonSize as ShadcnButtonSize,
    ButtonStyle as ShadcnButtonStyle, ButtonVariant as ShadcnButtonVariant,
};

use super::theme::Theme;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ButtonVariant {
    #[default]
    Default,
    Solid,
    Classic,
    Soft,
    Surface,
    Destructive,
    Outline,
    Secondary,
    Ghost,
    Link,
}

impl From<ButtonVariant> for ShadcnButtonVariant {
    fn from(variant: ButtonVariant) -> Self {
        match variant {
            ButtonVariant::Default => Self::Default,
            ButtonVariant::Solid => Self::Solid,
            ButtonVariant::Classic => Self::Classic,
            ButtonVariant::Soft => Self::Soft,
            ButtonVariant::Surface => Self::Surface,
            ButtonVariant::Destructive => Self::Destructive,
            ButtonVariant::Outline => Self::Outline,
            ButtonVariant::Secondary => Self::Secondary,
            ButtonVariant::Ghost => Self::Ghost,
            ButtonVariant::Link => Self::Link,
        }
    }
}

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

impl From<ButtonSize> for ShadcnButtonSize {
    fn from(size: ButtonSize) -> Self {
        match size {
            ButtonSize::Sm => Self::Sm,
            ButtonSize::Default => Self::Default,
            ButtonSize::Lg => Self::Lg,
            ButtonSize::Icon => Self::Icon,
            ButtonSize::IconSm => Self::IconSm,
            ButtonSize::IconLg => Self::IconLg,
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

impl From<ButtonJustify> for ShadcnButtonJustify {
    fn from(justify: ButtonJustify) -> Self {
        match justify {
            ButtonJustify::Start => Self::Start,
            ButtonJustify::Center => Self::Center,
            ButtonJustify::Between => Self::Between,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ButtonStyle {
    pub(crate) bg: Color32,
    pub(crate) bg_hover: Color32,
    pub(crate) bg_active: Color32,
    pub(crate) text: Color32,
    pub(crate) text_hover: Color32,
    pub(crate) text_active: Color32,
    pub(crate) border: Color32,
    pub(crate) border_hover: Color32,
    pub(crate) focus_ring: Color32,
    pub(crate) disabled_opacity: f32,
    pub(crate) rounding: CornerRadius,
}

impl ButtonStyle {
    pub(crate) fn from_variant(theme: &Theme, variant: ButtonVariant) -> Self {
        Self::from_shadcn(ShadcnButtonStyle::from_variant(
            &theme.renderer_theme().palette,
            variant.into(),
        ))
    }

    fn from_shadcn(style: ShadcnButtonStyle) -> Self {
        Self {
            bg: style.bg,
            bg_hover: style.bg_hover,
            bg_active: style.bg_active,
            text: style.text,
            text_hover: style.text_hover,
            text_active: style.text_active,
            border: style.border,
            border_hover: style.border_hover,
            focus_ring: style.focus_ring,
            disabled_opacity: style.disabled_opacity,
            rounding: style.rounding,
        }
    }

    fn into_shadcn(self) -> ShadcnButtonStyle {
        ShadcnButtonStyle {
            bg: self.bg,
            bg_hover: self.bg_hover,
            bg_active: self.bg_active,
            text: self.text,
            text_hover: self.text_hover,
            text_active: self.text_active,
            border: self.border,
            border_hover: self.border_hover,
            focus_ring: self.focus_ring,
            disabled_opacity: self.disabled_opacity,
            rounding: self.rounding,
        }
    }
}

/// Semantic button roles used by the application.
///
/// Keeping this mapping in the app makes it possible to change the visual language without
/// updating every call site.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ButtonType {
    #[default]
    Primary,
    Secondary,
    Outline,
    Danger,
    Ghost,
    Menu,
    Link,
}

pub(crate) use ButtonType as ButtonKind;

impl From<ButtonType> for ButtonVariant {
    fn from(button_type: ButtonType) -> Self {
        match button_type {
            ButtonType::Primary => ButtonVariant::Default,
            ButtonType::Secondary => ButtonVariant::Secondary,
            ButtonType::Outline => ButtonVariant::Outline,
            ButtonType::Danger => ButtonVariant::Destructive,
            ButtonType::Ghost => ButtonVariant::Ghost,
            ButtonType::Menu => ButtonVariant::Ghost,
            ButtonType::Link => ButtonVariant::Link,
        }
    }
}

/// The common application button API. Width is a minimum width, matching egui's layout
/// semantics while still allowing labels to grow when necessary.
pub(crate) struct Button<'a> {
    inner: ShadcnButton<'a>,
}

impl<'a> Button<'a> {
    pub(crate) fn new(label: impl Into<WidgetText>) -> Self {
        Self {
            inner: ShadcnButton::new(label),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn label(label: impl Into<WidgetText>) -> Self {
        Self::new(label)
    }

    pub(crate) fn button_type(mut self, button_type: ButtonType) -> Self {
        self.inner = self
            .inner
            .variant(ShadcnButtonVariant::from(ButtonVariant::from(button_type)));
        self
    }

    #[allow(dead_code)]
    pub(crate) fn kind(self, button_type: ButtonKind) -> Self {
        self.button_type(button_type)
    }

    #[allow(dead_code)]
    pub(crate) fn variant(mut self, variant: ButtonVariant) -> Self {
        self.inner = self.inner.variant(variant.into());
        self
    }

    pub(crate) fn size(mut self, size: ButtonSize) -> Self {
        self.inner = self.inner.size(size.into());
        self
    }

    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.inner = self.inner.enabled(enabled);
        self
    }

    pub(crate) fn width(mut self, width: f32) -> Self {
        self.inner = self.inner.min_width(width);
        self
    }

    pub(crate) fn min_width(self, width: f32) -> Self {
        self.width(width)
    }

    pub(crate) fn justify(mut self, justify: ButtonJustify) -> Self {
        self.inner = self.inner.justify(justify.into());
        self
    }

    pub(crate) fn icon(
        mut self,
        icon: &'a dyn Fn(&egui::Painter, egui::Pos2, f32, egui::Color32),
    ) -> Self {
        self.inner = self.inner.icon(icon);
        self
    }

    #[allow(dead_code)]
    pub(crate) fn style(mut self, style: ButtonStyle) -> Self {
        self.inner = self.inner.style(style.into_shadcn());
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, theme: &Theme) -> Response {
        self.inner.show(ui, theme.renderer_theme())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_types_map_to_shadcn_variants() {
        assert_eq!(
            ButtonVariant::from(ButtonType::Primary),
            ButtonVariant::Default
        );
        assert_eq!(
            ButtonVariant::from(ButtonType::Outline),
            ButtonVariant::Outline
        );
        assert_eq!(
            ButtonVariant::from(ButtonType::Danger),
            ButtonVariant::Destructive
        );
        assert_eq!(ButtonVariant::from(ButtonType::Menu), ButtonVariant::Ghost);
    }
}
