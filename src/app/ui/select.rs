use egui::Response;
use std::fmt::Debug;
use std::hash::Hash;

use super::theme::Theme;

#[derive(Clone, Debug)]
pub(crate) struct SelectItem {
    value: String,
    label: String,
}

impl SelectItem {
    pub(crate) fn option(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SelectSize {
    Size1,
    #[default]
    Size2,
    Size3,
}

/// The common application select API.
pub(crate) struct Select<'a, Id>
where
    Id: Hash + Debug,
{
    id: Id,
    selected: &'a mut Option<String>,
    items: &'a [SelectItem],
    placeholder: &'a str,
    size: SelectSize,
    enabled: bool,
    width: Option<f32>,
}

impl<'a, Id> Select<'a, Id>
where
    Id: Hash + Debug,
{
    pub(crate) fn new(id: Id, selected: &'a mut Option<String>, items: &'a [SelectItem]) -> Self {
        Self {
            id,
            selected,
            items,
            placeholder: "Select an option",
            size: SelectSize::default(),
            enabled: true,
            width: None,
        }
    }

    pub(crate) fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn size(mut self, size: SelectSize) -> Self {
        self.size = size;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, _theme: &Theme) -> Response {
        let selected_text = self
            .selected
            .as_deref()
            .and_then(|selected| self.items.iter().find(|item| item.value == selected))
            .map(|item| item.label.as_str())
            .unwrap_or(self.placeholder);
        let mut selection_changed = false;
        ui.add_enabled_ui(self.enabled, |ui| {
            let mut combo_box = egui::ComboBox::from_id_salt(self.id)
                .selected_text(selected_text)
                .width(self.width.unwrap_or_else(|| ui.available_width()));
            if self.size == SelectSize::Size1 {
                combo_box = combo_box.height(180.0);
            } else if self.size == SelectSize::Size3 {
                combo_box = combo_box.height(360.0);
            }
            let mut response = combo_box
                .show_ui(ui, |ui| {
                    for item in self.items {
                        if ui
                            .selectable_value(self.selected, Some(item.value.clone()), &item.label)
                            .changed()
                        {
                            selection_changed = true;
                        }
                    }
                })
                .response;
            if selection_changed {
                response.mark_changed();
            }
            response
        })
        .inner
    }
}
