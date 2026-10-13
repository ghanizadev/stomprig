use egui::{Id, Response, WidgetText};

use super::theme::Theme;

mod sized;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SheetSide {
    Top,
    #[default]
    Right,
    Bottom,
    Left,
}

pub(crate) struct SheetProps<'a> {
    id: Id,
    open: &'a mut bool,
    side: SheetSide,
}

impl<'a> SheetProps<'a> {
    pub(crate) fn new(id: Id, open: &'a mut bool) -> Self {
        Self {
            id,
            open,
            side: SheetSide::Right,
        }
    }

    pub(crate) fn side(mut self, side: SheetSide) -> Self {
        self.side = side;
        self
    }
}

pub(crate) struct SheetContext<'a> {
    id: Id,
    open: &'a mut bool,
    side: SheetSide,
}

pub(crate) fn show<R>(
    ui: &mut egui::Ui,
    props: SheetProps<'_>,
    add_contents: impl FnOnce(&mut egui::Ui, &mut SheetContext<'_>) -> R,
) -> R {
    let mut context = SheetContext {
        id: props.id,
        open: props.open,
        side: props.side,
    };
    add_contents(ui, &mut context)
}

impl SheetContext<'_> {
    pub(crate) fn set_open(&mut self, open: bool) {
        *self.open = open;
    }
}

pub(crate) fn trigger(
    ui: &mut egui::Ui,
    context: &mut SheetContext<'_>,
    add_trigger: impl FnOnce(&mut egui::Ui) -> Response,
) -> Response {
    let mut response = add_trigger(ui);
    if response.clicked() {
        *context.open = true;
        response.mark_changed();
    }
    response
}

pub(crate) fn content_bottom_with_height<R>(
    ui: &mut egui::Ui,
    theme: &Theme,
    context: &mut SheetContext<'_>,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    sized::content(ui, theme, context, height, add_contents)
}

pub(crate) fn header<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let result = ui.vertical(|ui| {
        let result = add_contents(ui);
        ui.separator();
        result
    });
    result.inner
}

pub(crate) fn footer<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.separator();
    ui.horizontal(|ui| add_contents(ui)).inner
}

pub(crate) fn title(ui: &mut egui::Ui, _theme: &Theme, text: impl Into<WidgetText>) -> Response {
    ui.label(text.into().heading())
}

pub(crate) fn description(
    ui: &mut egui::Ui,
    _theme: &Theme,
    text: impl Into<WidgetText>,
) -> Response {
    ui.label(text)
}
