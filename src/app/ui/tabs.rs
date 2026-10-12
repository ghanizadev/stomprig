use egui::{Id, WidgetText, vec2};

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
    _theme: &Theme,
    id: Id,
    items: &[TabItem],
    active: &mut String,
    _variant: TabsVariant,
    size: TabsSize,
    scrollable: bool,
    full_width: bool,
    render_content: impl FnOnce(&mut egui::Ui, &TabItem) -> R,
) -> R {
    let button_height = match size {
        TabsSize::Size1 => 28.0,
        TabsSize::Size2 => 32.0,
    };

    let mut show_items = |ui: &mut egui::Ui| {
        let available_width = ui.available_width();
        let item_width = if full_width && !items.is_empty() {
            available_width / items.len() as f32
        } else {
            0.0
        };
        ui.horizontal(|ui| {
            for item in items {
                ui.push_id((id, &item.id), |ui| {
                    let selected = *active == item.id;
                    let button = egui::Button::new(item.label.clone())
                        .selected(selected)
                        .min_size(vec2(item_width, button_height));
                    let mut response = ui.add(button);
                    if response.clicked() && !selected {
                        *active = item.id.clone();
                        response.mark_changed();
                    }
                });
            }
        });
    };

    if scrollable {
        egui::ScrollArea::horizontal()
            .id_salt((id, "tabs-scroll"))
            .auto_shrink([false, false])
            .show(ui, |ui| show_items(ui));
    } else {
        show_items(ui);
    }
    ui.separator();

    let active_item = items
        .iter()
        .find(|item| item.id == *active)
        .unwrap_or(&items[0]);
    render_content(ui, active_item)
}
