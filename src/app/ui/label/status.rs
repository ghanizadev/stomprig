use egui::{Response, Ui, WidgetText};

use super::{Label, LabelStyle};

pub(crate) fn error(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).style(LabelStyle::Error).show(ui)
}

pub(crate) fn success(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).style(LabelStyle::Success).show(ui)
}

pub(crate) fn warning(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).style(LabelStyle::Warning).show(ui)
}
