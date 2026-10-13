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
#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum TabsSize {
    Size1,
    #[default]
    Size2,
}

const TAB_GAP: f32 = 2.0;

/// A compact, top-aligned tab strip with the content for its selected tab.
pub(crate) struct Tabs<'a> {
    id: Id,
    items: &'a [TabItem],
    active: &'a mut String,
    size: TabsSize,
    scrollable: bool,
}

impl<'a> Tabs<'a> {
    pub(crate) fn new(id: Id, items: &'a [TabItem], active: &'a mut String) -> Self {
        Self {
            id,
            items,
            active,
            size: TabsSize::default(),
            scrollable: false,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn size(mut self, size: TabsSize) -> Self {
        self.size = size;
        self
    }

    pub(crate) fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    pub(crate) fn show<R>(
        self,
        ui: &mut egui::Ui,
        _theme: &Theme,
        render_content: impl FnOnce(&mut egui::Ui, &TabItem) -> R,
    ) -> Option<R> {
        let Some(first_item) = self.items.first() else {
            ui.separator();
            return None;
        };

        if !self.items.iter().any(|item| item.id == *self.active) {
            *self.active = first_item.id.clone();
        }

        let button_height = match self.size {
            TabsSize::Size1 => 28.0,
            TabsSize::Size2 => 32.0,
        };
        let id = self.id;
        let active = self.active;
        let items = self.items;
        let scrollable = self.scrollable;

        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = TAB_GAP;

            let mut show_tab_buttons = |ui: &mut egui::Ui| {
                ui.horizontal(|ui| {
                    for item in items {
                        ui.push_id((id, &item.id), |ui| {
                            let selected = *active == item.id;
                            let mut response = ui.add(
                                egui::Button::new(item.label.clone())
                                    .selected(selected)
                                    .frame(true)
                                    .corner_radius(egui::CornerRadius::same(2))
                                    .min_size(vec2(0.0, button_height)),
                            );

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
                    .auto_shrink([false, true])
                    .show(ui, |ui| show_tab_buttons(ui));
            } else {
                show_tab_buttons(ui);
            }
        });

        ui.separator();

        let active_item = self.items.iter().find(|item| item.id == *active)?;
        Some(render_content(ui, active_item))
    }
}
