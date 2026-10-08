use egui::Response;
use egui_shadcn::{
    SelectItem as ShadcnSelectItem, SelectProps as ShadcnSelectProps,
    SelectSize as ShadcnSelectSize, select_with_items as shadcn_select_with_items,
};
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

impl From<SelectSize> for ShadcnSelectSize {
    fn from(size: SelectSize) -> Self {
        match size {
            SelectSize::Size1 => Self::Size1,
            SelectSize::Size2 => Self::Size2,
            SelectSize::Size3 => Self::Size3,
        }
    }
}

/// The common application select API.
pub(crate) struct Select<'a, Id>
where
    Id: Hash + Debug,
{
    props: ShadcnSelectProps<'a, Id>,
    items: &'a [SelectItem],
}

impl<'a, Id> Select<'a, Id>
where
    Id: Hash + Debug,
{
    pub(crate) fn new(id: Id, selected: &'a mut Option<String>, items: &'a [SelectItem]) -> Self {
        Self {
            props: ShadcnSelectProps::new(id, selected),
            items,
        }
    }

    pub(crate) fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.props = self.props.placeholder(placeholder);
        self
    }

    #[allow(dead_code)]
    pub(crate) fn size(mut self, size: SelectSize) -> Self {
        self.props = self.props.size(size.into());
        self
    }

    #[allow(dead_code)]
    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.props = self.props.enabled(enabled);
        self
    }

    pub(crate) fn width(mut self, width: f32) -> Self {
        self.props = self.props.width(width);
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, theme: &Theme) -> Response {
        let items: Vec<ShadcnSelectItem> = self
            .items
            .iter()
            .map(|item| ShadcnSelectItem::option(item.value.clone(), item.label.clone()))
            .collect();
        shadcn_select_with_items(ui, theme.renderer_theme(), self.props, &items)
    }
}
