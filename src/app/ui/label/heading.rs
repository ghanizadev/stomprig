use egui::{Response, Ui, WidgetText};

use super::{Label, LabelStyle};

pub(crate) fn heading(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).style(LabelStyle::Heading).show(ui)
}

pub(crate) fn section(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    Label::new(text).style(LabelStyle::Section).show(ui)
}
