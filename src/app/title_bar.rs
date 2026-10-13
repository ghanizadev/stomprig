use egui::{Align2, Color32, FontFamily, FontId, Painter, Rect};
use egui_desktop::{
    CustomIcon, KeyboardShortcut, MenuItem, SubMenuItem, TitleBar, TitleBarOptions,
    render_resize_handles,
};
use lucide_icons::Icon;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use super::presets::PresetEntry;
use super::ui::{StompRigThemeProvider, ThemePreference};

const NO_ACTION: u8 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum MenuActionCode {
    Close = 1,
    OpenSettings = 2,
    ClearStack = 3,
    ToggleTuner = 4,
    TogglePlayback = 5,
    OpenLive = 6,
    SavePreset = 7,
    ExportPreset = 8,
    ImportPreset = 9,
    ManagePresets = 10,
    LoadPreset = 11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TitleBarAction {
    Close,
    OpenSettings,
    ClearStack,
    ToggleTuner,
    TogglePlayback,
    OpenLive,
    SavePreset,
    ExportPreset,
    ImportPreset,
    ManagePresets,
    LoadPreset(usize),
}

pub(crate) struct TitleBarFrame<'a> {
    pub(crate) theme_preference: ThemePreference,
    pub(crate) main_window_enabled: bool,
    pub(crate) has_plugin_stack: bool,
    pub(crate) has_loaded_plugin_stack: bool,
    pub(crate) show_tuner: bool,
    pub(crate) show_playback: bool,
    pub(crate) presets: &'a [PresetEntry],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MenuSignature {
    main_window_enabled: bool,
    has_plugin_stack: bool,
    has_loaded_plugin_stack: bool,
    show_tuner: bool,
    show_playback: bool,
    preset_names: Vec<Vec<String>>,
}

pub(crate) struct TitleBarState {
    title_bar: TitleBar,
    pending_action: Arc<AtomicU8>,
    pending_preset_index: Arc<AtomicUsize>,
    signature: Option<MenuSignature>,
}

impl Default for TitleBarState {
    fn default() -> Self {
        let pending_action = Arc::new(AtomicU8::new(NO_ACTION));
        let pending_preset_index = Arc::new(AtomicUsize::new(0));
        let theme_mode = ThemePreference::System.desktop_mode();
        let title_bar = TitleBar::new(
            TitleBarOptions::new()
                .with_title("StompRig")
                .with_theme_mode(theme_mode),
        )
        .with_theme_provider(StompRigThemeProvider)
        .add_icon(
            lucide_icon(Icon::Settings),
            Some(action_callback(
                Arc::clone(&pending_action),
                MenuActionCode::OpenSettings,
            )),
            Some("Settings".to_owned()),
            Some(shortcut("ctrl+e")),
        )
        .add_icon(
            lucide_icon(Icon::Radio),
            Some(action_callback(
                Arc::clone(&pending_action),
                MenuActionCode::OpenLive,
            )),
            Some("Open live mode".to_owned()),
            Some(shortcut("ctrl+shift+l")),
        );

        Self {
            title_bar,
            pending_action,
            pending_preset_index,
            signature: None,
        }
    }
}

impl TitleBarState {
    pub(crate) fn show(&mut self, ctx: &egui::Context, frame: TitleBarFrame<'_>) {
        let signature = MenuSignature {
            main_window_enabled: frame.main_window_enabled,
            has_plugin_stack: frame.has_plugin_stack,
            has_loaded_plugin_stack: frame.has_loaded_plugin_stack,
            show_tuner: frame.show_tuner,
            show_playback: frame.show_playback,
            preset_names: frame
                .presets
                .iter()
                .map(|entry| entry.segments.clone())
                .collect(),
        };
        if self.signature.as_ref() != Some(&signature) {
            self.update_menus(&signature, frame.presets);
            self.signature = Some(signature);
        }

        self.title_bar
            .update_theme_mode(frame.theme_preference.desktop_mode());
        self.title_bar.check_keyboard_shortcuts(ctx);
        self.title_bar.handle_icon_shortcuts(ctx);
        self.title_bar.show(ctx);
        if frame.main_window_enabled {
            render_resize_handles(ctx);
        }
    }

    pub(crate) fn take_action(&self) -> Option<TitleBarAction> {
        let code = self.pending_action.swap(NO_ACTION, Ordering::AcqRel);
        let action = match code {
            value if value == MenuActionCode::Close as u8 => TitleBarAction::Close,
            value if value == MenuActionCode::OpenSettings as u8 => TitleBarAction::OpenSettings,
            value if value == MenuActionCode::ClearStack as u8 => TitleBarAction::ClearStack,
            value if value == MenuActionCode::ToggleTuner as u8 => TitleBarAction::ToggleTuner,
            value if value == MenuActionCode::TogglePlayback as u8 => {
                TitleBarAction::TogglePlayback
            }
            value if value == MenuActionCode::OpenLive as u8 => TitleBarAction::OpenLive,
            value if value == MenuActionCode::SavePreset as u8 => TitleBarAction::SavePreset,
            value if value == MenuActionCode::ExportPreset as u8 => TitleBarAction::ExportPreset,
            value if value == MenuActionCode::ImportPreset as u8 => TitleBarAction::ImportPreset,
            value if value == MenuActionCode::ManagePresets as u8 => TitleBarAction::ManagePresets,
            value if value == MenuActionCode::LoadPreset as u8 => {
                TitleBarAction::LoadPreset(self.pending_preset_index.load(Ordering::Acquire))
            }
            _ => return None,
        };
        Some(action)
    }

    fn update_menus(&mut self, signature: &MenuSignature, presets: &[PresetEntry]) {
        let enabled = signature.main_window_enabled;
        let file_menu = MenuItem::new("File")
            .with_id("stomprig-file-menu")
            .add_subitem(
                menu_action("Close", MenuActionCode::Close, &self.pending_action)
                    .with_shortcut(shortcut("ctrl+q")),
            );

        let edit_menu = MenuItem::new("Edit")
            .with_id("stomprig-edit-menu")
            .add_subitem(
                menu_action(
                    "Settings",
                    MenuActionCode::OpenSettings,
                    &self.pending_action,
                )
                .with_shortcut(shortcut("ctrl+e"))
                .with_separator(),
            )
            .add_subitem(if signature.has_plugin_stack {
                menu_action(
                    "Clear Plugin Stack",
                    MenuActionCode::ClearStack,
                    &self.pending_action,
                )
            } else {
                menu_action(
                    "Clear Plugin Stack",
                    MenuActionCode::ClearStack,
                    &self.pending_action,
                )
                .disabled()
            });

        let view_menu = MenuItem::new("View")
            .with_id("stomprig-view-menu")
            .add_subitem(
                menu_action(
                    "Show tuner",
                    MenuActionCode::ToggleTuner,
                    &self.pending_action,
                )
                .with_checked(signature.show_tuner),
            )
            .add_subitem(
                menu_action(
                    "Show playback",
                    MenuActionCode::TogglePlayback,
                    &self.pending_action,
                )
                .with_checked(signature.show_playback),
            );

        let live_menu = MenuItem::new("Live")
            .with_id("stomprig-live-menu")
            .add_subitem(
                menu_action(
                    "Open live mode",
                    MenuActionCode::OpenLive,
                    &self.pending_action,
                )
                .with_shortcut(shortcut("ctrl+shift+l")),
            );

        let preset_menu = self.preset_menu(signature, presets);
        self.title_bar.menu_items_with_submenus = vec![
            if enabled {
                file_menu
            } else {
                file_menu.disabled()
            },
            if enabled {
                edit_menu
            } else {
                edit_menu.disabled()
            },
            if enabled {
                view_menu
            } else {
                view_menu.disabled()
            },
            if enabled {
                live_menu
            } else {
                live_menu.disabled()
            },
            if enabled {
                preset_menu
            } else {
                preset_menu.disabled()
            },
        ];
        self.title_bar.menu_order = (0..self.title_bar.menu_items_with_submenus.len())
            .map(|index| (true, index))
            .collect();
    }

    fn preset_menu(&self, signature: &MenuSignature, presets: &[PresetEntry]) -> MenuItem {
        let enabled = signature.main_window_enabled;
        let mut menu = MenuItem::new("Presets").with_id("stomprig-presets-menu");
        menu = menu.add_subitem(if signature.has_loaded_plugin_stack {
            menu_action(
                "Save preset",
                MenuActionCode::SavePreset,
                &self.pending_action,
            )
            .with_shortcut(shortcut("ctrl+s"))
            .with_separator()
        } else {
            menu_action(
                "Save preset",
                MenuActionCode::SavePreset,
                &self.pending_action,
            )
            .with_shortcut(shortcut("ctrl+s"))
            .with_separator()
            .disabled()
        });
        menu = menu.add_subitem(if signature.has_loaded_plugin_stack {
            menu_action(
                "Export preset...",
                MenuActionCode::ExportPreset,
                &self.pending_action,
            )
            .with_shortcut(shortcut("ctrl+shift+s"))
        } else {
            menu_action(
                "Export preset...",
                MenuActionCode::ExportPreset,
                &self.pending_action,
            )
            .with_shortcut(shortcut("ctrl+shift+s"))
            .disabled()
        });
        menu = menu
            .add_subitem(
                menu_action(
                    "Import preset...",
                    MenuActionCode::ImportPreset,
                    &self.pending_action,
                )
                .with_shortcut(shortcut("ctrl+o"))
                .with_separator(),
            )
            .add_subitem(menu_action(
                "Manage presets...",
                MenuActionCode::ManagePresets,
                &self.pending_action,
            ));

        let load_menu = load_preset_menu(
            presets,
            Arc::clone(&self.pending_action),
            Arc::clone(&self.pending_preset_index),
        );
        menu = menu.add_subitem(load_menu);
        if !enabled { menu.disabled() } else { menu }
    }
}

fn shortcut(value: &str) -> KeyboardShortcut {
    KeyboardShortcut::from_string(value).expect("static keyboard shortcut is valid")
}

fn menu_action(
    label: impl Into<String>,
    action: MenuActionCode,
    pending_action: &Arc<AtomicU8>,
) -> SubMenuItem {
    SubMenuItem::new(&label.into())
        .with_callback(action_callback(Arc::clone(pending_action), action))
}

fn action_callback(
    pending_action: Arc<AtomicU8>,
    action: MenuActionCode,
) -> Box<dyn Fn() + Send + Sync> {
    Box::new(move || pending_action.store(action as u8, Ordering::Release))
}

fn preset_callback(
    pending_action: Arc<AtomicU8>,
    pending_preset_index: Arc<AtomicUsize>,
    preset_index: usize,
) -> Box<dyn Fn() + Send + Sync> {
    Box::new(move || {
        pending_preset_index.store(preset_index, Ordering::Release);
        pending_action.store(MenuActionCode::LoadPreset as u8, Ordering::Release);
    })
}

fn lucide_icon(icon: Icon) -> CustomIcon {
    let glyph = icon.unicode().to_owned();
    CustomIcon::Drawn(Box::new(
        move |painter: &Painter, rect: Rect, color: Color32| {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                FontId::new(16.0, FontFamily::Name("lucide".into())),
                color,
            );
        },
    ))
}

#[derive(Default)]
struct PresetMenuNode {
    categories: BTreeMap<String, PresetMenuNode>,
    presets: Vec<(String, usize)>,
}

fn load_preset_menu(
    entries: &[PresetEntry],
    pending_action: Arc<AtomicU8>,
    pending_preset_index: Arc<AtomicUsize>,
) -> SubMenuItem {
    if entries.is_empty() {
        return SubMenuItem::new("Load preset")
            .add_child(SubMenuItem::new("No presets saved.").disabled());
    }

    let mut root = PresetMenuNode::default();
    for (preset_index, entry) in entries.iter().enumerate() {
        let Some((name, categories)) = entry.segments.split_last() else {
            continue;
        };
        let mut node = &mut root;
        for category in categories {
            node = node.categories.entry(category.clone()).or_default();
        }
        node.presets.push((name.clone(), preset_index));
    }
    let children = root.into_items(pending_action, pending_preset_index);
    let mut menu = SubMenuItem::new("Load preset");
    for child in children {
        menu = menu.add_child(child);
    }
    menu
}

impl PresetMenuNode {
    fn into_items(
        self,
        pending_action: Arc<AtomicU8>,
        pending_preset_index: Arc<AtomicUsize>,
    ) -> Vec<SubMenuItem> {
        let categories = self.categories.into_iter().map(|(label, node)| {
            let mut submenu = SubMenuItem::new(&label);
            for child in node.into_items(
                Arc::clone(&pending_action),
                Arc::clone(&pending_preset_index),
            ) {
                submenu = submenu.add_child(child);
            }
            submenu
        });
        let presets = self.presets.into_iter().map(|(label, preset_index)| {
            SubMenuItem::new(&label).with_callback(preset_callback(
                Arc::clone(&pending_action),
                Arc::clone(&pending_preset_index),
                preset_index,
            ))
        });
        categories.chain(presets).collect()
    }
}
