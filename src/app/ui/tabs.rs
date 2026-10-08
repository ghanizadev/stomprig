use egui::{Id, WidgetText};

use super::theme::Theme;

#[derive(Clone, Debug)]
pub(crate) struct TabItem {
    pub(crate) id: String,
    pub(crate) label: WidgetText,
}

impl TabItem {
    pub(crate) fn new(id: impl Into<String>, label: impl Into<WidgetText>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum TabsSize {
    Size1,
    Size2,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum TabsVariant {
    Underline,
    Soft,
    Outline,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn show<R>(
    ui: &mut egui::Ui,
    theme: &Theme,
    id: Id,
    items: &[TabItem],
    active: &mut String,
    variant: TabsVariant,
    size: TabsSize,
    scrollable: bool,
    full_width: bool,
    render_content: impl FnOnce(&mut egui::Ui, &TabItem) -> R,
) -> R {
    let renderer_items: Vec<egui_shadcn::TabItem> = items
        .iter()
        .map(|item| egui_shadcn::TabItem::new(item.id.clone(), item.label.clone()))
        .collect();
    let result = egui_shadcn::tabs(
        ui,
        theme.renderer_theme(),
        egui_shadcn::TabsProps::new(id, &renderer_items, active)
            .variant(match variant {
                TabsVariant::Underline => egui_shadcn::TabsVariant::Underline,
                TabsVariant::Soft => egui_shadcn::TabsVariant::Soft,
                TabsVariant::Outline => egui_shadcn::TabsVariant::Outline,
            })
            .scrollable(scrollable)
            .full_width(full_width)
            .size(match size {
                TabsSize::Size1 => egui_shadcn::TabsSize::Size1,
                TabsSize::Size2 => egui_shadcn::TabsSize::Size2,
            }),
        |content_ui, active_item| {
            let item = items
                .iter()
                .find(|item| item.id == active_item.id)
                .unwrap_or(&items[0]);
            render_content(content_ui, item)
        },
    );
    result.content
}
