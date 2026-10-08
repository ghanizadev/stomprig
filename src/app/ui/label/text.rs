use egui::{Response, Ui, WidgetText};

use super::{Label, LabelStyle};

pub(crate) fn label(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).show(ui)
}

pub(crate) fn strong(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).style(LabelStyle::Strong).show(ui)
}

pub(crate) fn caption(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).style(LabelStyle::Caption).show(ui)
}

pub(crate) fn muted(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).style(LabelStyle::Muted).show(ui)
}
