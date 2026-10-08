mod heading;
mod painter;
mod status;
mod text;

use egui::{Response, Sense, TextWrapMode, Ui, Widget, WidgetText};

pub(crate) use heading::{heading, section};
pub(crate) use painter::paint;
pub(crate) use status::{error, success, warning};
pub(crate) use text::{caption, label, muted, strong};

/// Semantic text styles used by the application UI.
///
/// Keeping these styles here gives the application one place to define its text hierarchy and
/// status colors. Call sites should choose a semantic style instead of depending on egui's
/// concrete text helpers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LabelStyle {
    #[default]
    Body,
    Heading,
    Section,
    Strong,
    Caption,
    Muted,
    Error,
    Success,
    Warning,
}

/// The common application label API.
pub(crate) struct Label {
    text: WidgetText,
    style: LabelStyle,
    wrap_mode: Option<TextWrapMode>,
    sense: Option<Sense>,
}

impl Label {
    pub(crate) fn new(text: impl Into<WidgetText>) -> Self {
        Self {
            text: text.into(),
            style: LabelStyle::default(),
            wrap_mode: None,
            sense: None,
        }
    }

    pub(crate) fn style(mut self, style: LabelStyle) -> Self {
        self.style = style;
        self
    }

    pub(crate) fn wrap(mut self) -> Self {
        self.wrap_mode = Some(TextWrapMode::Wrap);
        self
    }

    pub(crate) fn truncate(mut self) -> Self {
        self.wrap_mode = Some(TextWrapMode::Truncate);
        self
    }

    pub(crate) fn sense(mut self, sense: Sense) -> Self {
        self.sense = Some(sense);
        self
    }

    pub(crate) fn show(self, ui: &mut Ui) -> Response {
        ui.add(self)
    }
}

impl Widget for Label {
    fn ui(self, ui: &mut Ui) -> Response {
        let text = match self.style {
            LabelStyle::Body => self.text,
            LabelStyle::Heading => self.text.heading(),
            LabelStyle::Section | LabelStyle::Strong => self.text.strong(),
            LabelStyle::Caption => self.text.small().weak(),
            LabelStyle::Muted => self.text.weak(),
            LabelStyle::Error => self.text.color(ui.visuals().error_fg_color),
            LabelStyle::Success => self.text.color(egui::Color32::GREEN),
            LabelStyle::Warning => self.text.color(ui.visuals().warn_fg_color),
        };

        let mut label = egui::Label::new(text);
        if let Some(wrap_mode) = self.wrap_mode {
            label = label.wrap_mode(wrap_mode);
        }
        if let Some(sense) = self.sense {
            label = label.sense(sense);
        }
        ui.add(label)
    }
}
