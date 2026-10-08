use egui::{Id, Response, WidgetText};

use super::theme::Theme;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
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

pub(crate) struct SheetContext<'a, 'b> {
    inner: &'a mut egui_shadcn::SheetContext<'b>,
}

pub(crate) fn show<R>(
    ui: &mut egui::Ui,
    props: SheetProps<'_>,
    add_contents: impl for<'a, 'b> FnOnce(&mut egui::Ui, &mut SheetContext<'a, 'b>) -> R,
) -> R {
    egui_shadcn::sheet(
        ui,
        egui_shadcn::SheetProps::new(props.id, props.open).side(match props.side {
            SheetSide::Top => egui_shadcn::SheetSide::Top,
            SheetSide::Right => egui_shadcn::SheetSide::Right,
            SheetSide::Bottom => egui_shadcn::SheetSide::Bottom,
            SheetSide::Left => egui_shadcn::SheetSide::Left,
        }),
        |ui, context| {
            let mut context = SheetContext { inner: context };
            add_contents(ui, &mut context)
        },
    )
}

impl SheetContext<'_, '_> {
    pub(crate) fn set_open(&mut self, open: bool) {
        *self.inner.open = open;
    }
}

pub(crate) fn trigger(
    ui: &mut egui::Ui,
    context: &mut SheetContext<'_, '_>,
    add_trigger: impl FnOnce(&mut egui::Ui) -> Response,
) -> Response {
    egui_shadcn::sheet_trigger(ui, context.inner, add_trigger)
}

pub(crate) fn content<R>(
    ui: &mut egui::Ui,
    theme: &Theme,
    context: &mut SheetContext<'_, '_>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    egui_shadcn::sheet_content(ui, theme.renderer_theme(), context.inner, add_contents)
}

pub(crate) fn header<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui_shadcn::sheet_header(ui, add_contents)
}

pub(crate) fn footer<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui_shadcn::sheet_footer(ui, add_contents)
}

pub(crate) fn title(ui: &mut egui::Ui, theme: &Theme, text: impl Into<WidgetText>) -> Response {
    egui_shadcn::sheet_title(ui, theme.renderer_theme(), text)
}

pub(crate) fn description(
    ui: &mut egui::Ui,
    theme: &Theme,
    text: impl Into<WidgetText>,
) -> Response {
    egui_shadcn::sheet_description(ui, theme.renderer_theme(), text)
}
