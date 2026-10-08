use egui::{Response, WidgetText};
pub(crate) use egui_shadcn::{
    Button as ShadcnButton, ButtonJustify, ButtonSize, ButtonStyle, ButtonVariant, Theme,
};

/// Semantic button roles used by the application.
///
/// Keeping this mapping in the app makes it possible to change the visual language without
/// updating every call site. `variant` remains available on [`Button`] for compatibility with
/// the existing shadcn-based call sites.
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
            ButtonType::Primary => Self::Default,
            ButtonType::Secondary => Self::Secondary,
            ButtonType::Outline => Self::Outline,
            ButtonType::Danger => Self::Destructive,
            ButtonType::Ghost => Self::Ghost,
            ButtonType::Menu => Self::Ghost,
            ButtonType::Link => Self::Link,
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
        self.inner = self.inner.variant(button_type.into());
        self
    }

    #[allow(dead_code)]
    pub(crate) fn kind(self, button_type: ButtonKind) -> Self {
        self.button_type(button_type)
    }

    /// Compatibility builder for code that still uses the underlying shadcn variants.
    #[allow(dead_code)]
    pub(crate) fn variant(mut self, variant: ButtonVariant) -> Self {
        self.inner = self.inner.variant(variant);
        self
    }

    pub(crate) fn size(mut self, size: ButtonSize) -> Self {
        self.inner = self.inner.size(size);
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

    /// Compatibility alias for the previous shadcn builder.
    pub(crate) fn min_width(self, width: f32) -> Self {
        self.width(width)
    }

    pub(crate) fn justify(mut self, justify: ButtonJustify) -> Self {
        self.inner = self.inner.justify(justify);
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
        self.inner = self.inner.style(style);
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, theme: &Theme) -> Response {
        self.inner.show(ui, theme)
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
