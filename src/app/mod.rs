mod audio;
mod cpu;
pub mod diagnostics;
mod live;
mod loading;
mod midi;
mod plugin;
mod plugin_editor;
mod plugin_runtime;
mod presets;
mod settings;
mod splash;
mod status_bar;
mod ui;
mod windows;

use self::ui as app_ui;
use audio::ShutdownMode;
use cpu::CpuUsage;
use eframe::Frame;
use lucide_icons::Icon;
use plugin::{DiscoveredPlugin, discover_plugins_with_progress};
use plugin_editor::PluginEditor;
use presets::{LivePreset, LivePresetStore, Preset, PresetEntry, PresetStore};
use settings::{AppSettings, SettingsStore};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use self::ui::Theme;

pub struct RackApp {
    plugin_editor: PluginEditor,
    settings: Option<SettingsStore>,
    startup: Option<StartupTask>,
    startup_error: Option<String>,
    splash_theme: Theme,
    progress_value: f32,
    plugins_loaded: usize,
    plugins_total: usize,
    cpu_usage: CpuUsage,
    audio_meters: status_bar::AudioMeterState,
    latency: status_bar::LatencyState,
    presets: PresetStore,
    live_presets: LivePresetStore,
    save_preset_open: bool,
    save_preset_name: String,
    save_preset_is_new: bool,
    preset_error: Option<String>,
    overwrite_preset_name: Option<String>,
    manage_presets_open: bool,
    manage_presets_tab: ManagePresetsTab,
    board_preset_search: String,
    live_board_search: String,
    selected_live_preset_name: Option<String>,
    live_preset_name: String,
    live_preset_draft: Option<LivePreset>,
    live_preset_edit_mode: LivePresetEditMode,
    overwrite_live_preset: Option<(String, LivePreset)>,
    delete_live_preset_name: Option<String>,
    delete_preset_name: Option<String>,
    live_window_open: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ManagePresetsTab {
    #[default]
    Board,
    Live,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LivePresetEditMode {
    #[default]
    None,
    SidebarSelection,
    TypedNew,
    TypedExistingNeedsConfirmation,
}

impl Default for RackApp {
    fn default() -> Self {
        let startup = StartupTask::spawn();
        let live_presets = LivePresetStore::load();
        let selected_live_preset_name = live_presets
            .entries()
            .first()
            .map(|entry| entry.segments.join("/"));
        let live_preset_draft = selected_live_preset_name
            .as_deref()
            .and_then(|name| live_presets.get(name))
            .cloned();
        Self {
            plugin_editor: PluginEditor::default(),
            settings: None,
            startup: Some(startup),
            startup_error: None,
            splash_theme: Theme::default(),
            progress_value: 0.0,
            plugins_loaded: 0,
            plugins_total: 0,
            cpu_usage: CpuUsage::default(),
            audio_meters: status_bar::AudioMeterState::default(),
            latency: status_bar::LatencyState::default(),
            presets: PresetStore::load(),
            live_presets,
            save_preset_open: false,
            save_preset_name: String::new(),
            save_preset_is_new: true,
            preset_error: None,
            overwrite_preset_name: None,
            manage_presets_open: false,
            manage_presets_tab: ManagePresetsTab::Board,
            board_preset_search: String::new(),
            live_board_search: String::new(),
            live_preset_name: selected_live_preset_name.clone().unwrap_or_default(),
            live_preset_draft,
            live_preset_edit_mode: if selected_live_preset_name.is_some() {
                LivePresetEditMode::SidebarSelection
            } else {
                LivePresetEditMode::None
            },
            overwrite_live_preset: None,
            selected_live_preset_name,
            delete_live_preset_name: None,
            delete_preset_name: None,
            live_window_open: false,
        }
    }
}

impl eframe::App for RackApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.poll_startup();
        if self.startup.is_some() || self.startup_error.is_some() {
            self.show_splash(ctx);
            return;
        }

        let loading = self.plugin_editor.is_loading();
        if loading {
            ctx.set_cursor_icon(egui::CursorIcon::Progress);
        }
        self.plugin_editor.process_pending_load();
        let main_window_enabled = !loading && !self.plugin_editor.audio_configuration_open();
        let mut preset_to_load = None;
        let mut import_preset_requested = false;
        let mut export_preset_requested = false;
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            ui.add_enabled_ui(main_window_enabled, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if app_ui::menu_button(ui, self.plugin_editor.theme(), "Close").clicked() {
                            ui.close();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });

                    ui.menu_button("Edit", |ui| {
                        if app_ui::menu_button(ui, self.plugin_editor.theme(), "Settings").clicked()
                        {
                            ui.close();
                            self.plugin_editor.open_audio_configuration();
                        }
                        if app_ui::menu_button_enabled(
                            ui,
                            self.plugin_editor.theme(),
                            "Clear Plugin Stack",
                            self.plugin_editor.has_plugin_stack(),
                        )
                        .clicked()
                        {
                            ui.close();
                            self.plugin_editor.clear_plugin_stack();
                        }
                    });

                    ui.menu_button("View", |ui| {
                        let mut show_tuner = self.plugin_editor.is_tuner_visible();
                        if app_ui::menu_button_checkbox(
                            ui,
                            self.plugin_editor.theme(),
                            "Show tuner",
                            &mut show_tuner,
                        )
                        .changed()
                        {
                            self.plugin_editor.set_show_tuner(show_tuner);
                        }
                        let mut show_playback = self.plugin_editor.is_playback_visible();
                        if app_ui::menu_button_checkbox(
                            ui,
                            self.plugin_editor.theme(),
                            "Show playback",
                            &mut show_playback,
                        )
                        .changed()
                        {
                            self.plugin_editor.set_show_playback(show_playback);
                        }
                    });

                    ui.menu_button("Live", |ui| {
                        if app_ui::menu_button(ui, self.plugin_editor.theme(), "Open live mode")
                            .clicked()
                        {
                            ui.close();
                            self.live_window_open = true;
                        }
                    });

                    ui.menu_button("Presets", |ui| {
                        if app_ui::menu_button_enabled(
                            ui,
                            self.plugin_editor.theme(),
                            "Save preset",
                            self.plugin_editor.has_loaded_plugin_stack(),
                        )
                        .clicked()
                        {
                            ui.close();
                            let loaded_name =
                                self.plugin_editor.loaded_preset_name().map(str::to_owned);
                            self.save_preset_name = loaded_name.unwrap_or_default();
                            self.save_preset_is_new = self.save_preset_name.is_empty();
                            self.preset_error = None;
                            self.overwrite_preset_name = None;
                            self.save_preset_open = true;
                        }

                        if app_ui::menu_button_enabled(
                            ui,
                            self.plugin_editor.theme(),
                            "Export preset...",
                            self.plugin_editor.has_loaded_plugin_stack(),
                        )
                        .clicked()
                        {
                            ui.close();
                            export_preset_requested = true;
                        }

                        if app_ui::menu_button(ui, self.plugin_editor.theme(), "Import preset...")
                            .clicked()
                        {
                            ui.close();
                            import_preset_requested = true;
                        }

                        if app_ui::menu_button(ui, self.plugin_editor.theme(), "Manage presets...")
                            .clicked()
                        {
                            ui.close();
                            self.preset_error = None;
                            if self.selected_live_preset_name.is_none() {
                                self.selected_live_preset_name = self
                                    .live_presets
                                    .entries()
                                    .first()
                                    .map(|entry| entry.segments.join("/"));
                                self.live_preset_name =
                                    self.selected_live_preset_name.clone().unwrap_or_default();
                            }
                            if self.live_preset_draft.is_none() {
                                self.live_preset_draft = self
                                    .selected_live_preset_name
                                    .as_deref()
                                    .and_then(|name| self.live_presets.get(name))
                                    .cloned();
                            }
                            self.manage_presets_open = true;
                        }

                        let preset_items = build_preset_menu_items(self.presets.entries());
                        preset_to_load = app_ui::menu_button_with_items(
                            ui,
                            self.plugin_editor.theme(),
                            "Load preset",
                            &preset_items,
                            Some("No presets saved.".into()),
                        );
                    });
                });
            });
        });

        if let Some((name, preset)) = preset_to_load {
            self.plugin_editor.request_load_preset(preset, name);
        }
        if export_preset_requested {
            self.export_preset();
        }
        if import_preset_requested {
            self.import_preset();
        }

        egui::TopBottomPanel::bottom("status_bar")
            .resizable(false)
            .show(ctx, |ui| {
                status_bar::show(
                    ui,
                    &mut self.cpu_usage,
                    self.plugin_editor.edit_state_label(),
                    self.plugin_editor.loaded_preset_name(),
                    self.selected_live_preset_name.as_deref(),
                    self.plugin_editor.audio_levels(),
                    self.plugin_editor.estimated_latency_ms(),
                    &mut self.audio_meters,
                    &mut self.latency,
                );
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(main_window_enabled, |ui| {
                self.plugin_editor.ui(ui);
                self.apply_pending_program_changes();
                let values = self.current_settings();
                if let Some(settings) = self.settings.as_mut() {
                    settings.save_if_changed(&values);
                    if let Some(error) = &settings.error {
                        app_ui::error_label(ui, error);
                    }
                }
            });
        });

        if self.plugin_editor.is_loading() {
            ctx.set_cursor_icon(egui::CursorIcon::Progress);
            self.plugin_editor.show_loading(ctx);
        }
        self.show_save_preset_modal(ctx);
        self.show_overwrite_preset_modal(ctx);
        let manage_presets_was_open = self.manage_presets_open;
        self.show_manage_presets_modal(ctx);
        self.show_overwrite_live_preset_modal(ctx);
        if !manage_presets_was_open {
            self.show_delete_preset_modal(ctx);
        }
        self.show_delete_live_preset_modal(ctx);
        self.show_live_window(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        // eframe invokes this while the root viewport still exists. Native VST editors
        // must be detached before eframe destroys their parent window.
        log::info!("application shutdown; stopping plugin stack");
        self.plugin_editor.shutdown(ShutdownMode::KeepPluginsLoaded);
    }
}

impl RackApp {
    fn poll_startup(&mut self) {
        let Some(startup) = self.startup.as_ref() else {
            return;
        };
        self.progress_value = startup.progress.load(Ordering::Acquire) as f32;
        self.plugins_loaded = startup.plugins_loaded.load(Ordering::Acquire);
        self.plugins_total = startup.plugins_total.load(Ordering::Acquire);

        let Some(startup) = self.startup.take() else {
            return;
        };
        match startup.receiver.try_recv() {
            Ok(Ok(result)) => {
                self.plugin_editor = PluginEditor::from_settings(&result.settings.values);
                self.plugin_editor
                    .set_discovered_plugins(result.discovered_plugins);
                self.settings = Some(result.settings);
                self.progress_value = 100.0;
            }
            Ok(Err(error)) => {
                self.progress_value = 100.0;
                self.startup_error = Some(error);
            }
            Err(TryRecvError::Empty) => self.startup = Some(startup),
            Err(TryRecvError::Disconnected) => {
                self.progress_value = 100.0;
                self.startup_error = Some("Startup loading stopped unexpectedly.".to_owned());
            }
        }
    }

    fn show_splash(&self, ctx: &egui::Context) {
        let status = if self.progress_value < 20.0 {
            "Loading settings...".to_owned()
        } else if self.progress_value < 100.0 {
            format!(
                "Scanning installed plugins... ({}/{})",
                self.plugins_loaded, self.plugins_total
            )
        } else {
            "Finishing startup...".to_owned()
        };
        splash::show(
            ctx,
            &self.splash_theme,
            self.progress_value,
            &status,
            self.startup_error.as_deref(),
        );
    }

    fn show_live_window(&mut self, ctx: &egui::Context) {
        if !self.live_window_open {
            return;
        }

        let current_board_name = self.plugin_editor.loaded_preset_name().map(str::to_owned);
        let live_preset_name = self.selected_live_preset_name.clone();
        let live_preset_boards = live_preset_name.as_deref().and_then(|name| {
            self.live_preset_draft
                .as_ref()
                .map(|preset| preset.presets.clone())
                .or_else(|| {
                    self.live_presets
                        .get(name)
                        .map(|preset| preset.presets.clone())
                })
        });
        let effects = self.plugin_editor.live_effects();
        let tuner_settings = self.plugin_editor.tuner_settings();

        live::LiveWindow::show(
            ctx,
            &mut self.live_window_open,
            live::LiveWindowData {
                current_board_name: current_board_name.as_deref(),
                live_preset_name: live_preset_name.as_deref(),
                live_preset_boards: live_preset_boards.as_deref(),
                effects: &effects,
                tuner: live::LiveTunerData {
                    readout: self.plugin_editor.live_tuner_readout(),
                    bypassed: self.plugin_editor.live_tuner_bypassed(),
                    a4_pitch_hz: tuner_settings.a4_pitch_hz,
                    transposition_semitones: tuner_settings.transposition_semitones,
                },
            },
        );
    }

    fn current_settings(&self) -> AppSettings {
        AppSettings {
            audio: self.plugin_editor.audio_settings(),
            plugin_discovery: self.plugin_editor.plugin_discovery_settings(),
            tuner: self.plugin_editor.tuner_settings(),
            midi: self.plugin_editor.midi_settings(),
            show_playback: self.plugin_editor.is_playback_visible(),
            show_tuner: self.plugin_editor.is_tuner_visible(),
        }
    }

    fn show_save_preset_modal(&mut self, ctx: &egui::Context) {
        if !self.save_preset_open {
            return;
        }

        let existing_names = self
            .presets
            .entries()
            .iter()
            .map(|entry| entry.segments.join("/"))
            .collect::<Vec<_>>();
        let loaded_preset_name = self.plugin_editor.loaded_preset_name().map(str::to_owned);
        let preset_items: Vec<app_ui::SelectItem> =
            std::iter::once(app_ui::SelectItem::option("", "New preset..."))
                .chain(
                    existing_names
                        .iter()
                        .map(|name| app_ui::SelectItem::option(name.clone(), name.clone())),
                )
                .collect();
        let theme = self.plugin_editor.theme().clone();
        let mut save_requested = false;
        let response = egui::Modal::new(egui::Id::new("save-preset-modal")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.set_max_width(360.0);
            app_ui::heading(ui, "Save preset");
            ui.add(
                app_ui::Label::new(
                    "Select an existing preset to overwrite it, or create a new preset.",
                )
                .wrap(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                app_ui::label(ui, "Preset name");
                let mut selected_preset = if self.save_preset_is_new {
                    Some(String::new())
                } else {
                    Some(self.save_preset_name.clone())
                };
                if app_ui::Select::new("preset-name", &mut selected_preset, &preset_items)
                    .placeholder("New preset...")
                    .width(ui.available_width())
                    .show(ui, &theme)
                    .changed()
                {
                    if selected_preset.as_deref() == Some("") {
                        self.save_preset_is_new = true;
                        self.save_preset_name.clear();
                    } else if let Some(name) = selected_preset {
                        self.save_preset_is_new = false;
                        self.save_preset_name = name;
                    }
                }
            });
            if self.save_preset_is_new {
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.save_preset_name)
                        .hint_text("e.g. Metal/Iron Man")
                        .desired_width(f32::INFINITY),
                );
            }
            if let Some(loaded_name) = &loaded_preset_name {
                ui.add_space(6.0);
                let use_loaded =
                    app_ui::button(ui, self.plugin_editor.theme(), "Use loaded preset name");
                if use_loaded
                    .on_hover_text(format!("Use \"{loaded_name}\""))
                    .clicked()
                {
                    self.save_preset_is_new = false;
                    self.save_preset_name = loaded_name.clone();
                }
            }
            if let Some(error) = &self.preset_error {
                ui.add_space(6.0);
                app_ui::error_label(ui, error);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if app_ui::button(ui, self.plugin_editor.theme(), "Cancel").clicked() {
                    ui.close();
                }
                if app_ui::Button::new("Save")
                    .enabled(!self.save_preset_name.trim().is_empty())
                    .show(ui, self.plugin_editor.theme())
                    .clicked()
                {
                    save_requested = true;
                }
            });
        });

        if response.should_close() {
            self.save_preset_open = false;
            self.preset_error = None;
        }
        if save_requested {
            let name = self.save_preset_name.trim().to_owned();
            match self.presets.contains_name(&name) {
                Ok(true) => {
                    // Keep only one modal layer active. Leaving the save modal open while
                    // showing the overwrite confirmation causes both modal backdrops to be
                    // painted and lets the hidden modal process input as well.
                    self.save_preset_open = false;
                    self.overwrite_preset_name = Some(name);
                }
                Ok(false) => self.save_preset(name),
                Err(error) => self.preset_error = Some(error),
            }
        }
    }

    fn show_overwrite_preset_modal(&mut self, ctx: &egui::Context) {
        let Some(name) = self.overwrite_preset_name.clone() else {
            return;
        };

        let mut confirm = false;
        let response = egui::Modal::new(egui::Id::new("overwrite-preset-modal")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.set_max_width(320.0);
            app_ui::heading(ui, "Overwrite preset?");
            ui.add(app_ui::Label::new(format!("Replace the saved preset \"{name}\"?")).wrap());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if app_ui::button(ui, self.plugin_editor.theme(), "Cancel").clicked() {
                    ui.close();
                }
                if app_ui::button(ui, self.plugin_editor.theme(), "Overwrite").clicked() {
                    confirm = true;
                    ui.close();
                }
            });
        });

        if response.should_close() {
            self.overwrite_preset_name = None;
        }
        if confirm {
            self.overwrite_preset_name = None;
            self.save_preset(name);
        }
    }

    fn show_overwrite_live_preset_modal(&mut self, ctx: &egui::Context) {
        let Some((name, preset)) = self.overwrite_live_preset.clone() else {
            return;
        };

        let mut confirm = false;
        let response =
            egui::Modal::new(egui::Id::new("overwrite-live-preset-modal")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.set_max_width(360.0);
                app_ui::heading(ui, "Overwrite live preset?");
                ui.add(
                    app_ui::Label::new(format!(
                        "Replace the existing live preset \"{name}\" with the current stack?"
                    ))
                    .wrap(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if app_ui::button(ui, self.plugin_editor.theme(), "Cancel").clicked() {
                        ui.close();
                    }
                    if app_ui::button(ui, self.plugin_editor.theme(), "Overwrite").clicked() {
                        confirm = true;
                        ui.close();
                    }
                });
            });

        if response.should_close() {
            self.overwrite_live_preset = None;
        }
        if confirm {
            self.overwrite_live_preset = None;
            self.save_live_preset_contents(name, preset);
        }
    }

    fn show_manage_presets_modal(&mut self, ctx: &egui::Context) {
        if !self.manage_presets_open {
            return;
        }

        let loaded_preset_name = self.plugin_editor.loaded_preset_name().map(str::to_owned);
        let board_rows = self
            .presets
            .entries()
            .iter()
            .map(|entry| {
                let name = entry.segments.join("/");
                let loaded = loaded_preset_name.as_deref() == Some(name.as_str());
                (name, loaded, entry.preset.clone())
            })
            .collect::<Vec<_>>();
        let live_rows = self
            .live_presets
            .entries()
            .iter()
            .map(|entry| (entry.segments.join("/"), entry.preset.clone()))
            .collect::<Vec<_>>();
        let selected_live_name = self.selected_live_preset_name.clone();
        let selected_live = self.live_preset_draft.clone().or_else(|| {
            selected_live_name
                .as_deref()
                .and_then(|name| self.live_presets.get(name))
                .cloned()
        });
        let mut delete_requested = None;
        let mut export_requested = None;
        let mut delete_live_requested = None;
        let mut board_to_load = None;
        let mut live_save_requested = None;
        let mut new_board_requested = false;
        let mut import_live_requested = false;
        let mut export_current_live_requested = false;
        let response = egui::Modal::new(egui::Id::new("manage-presets-modal")).show(ctx, |ui| {
            ui.set_width(900.0);
            ui.set_max_width(900.0);
            ui.set_height(600.0);
            app_ui::heading(ui, "Manage presets");
            ui.add(app_ui::Label::new("Board presets are the sounds in the rack. Live presets are ordered playlists of those sounds.").wrap());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.manage_presets_tab == ManagePresetsTab::Board, "Board presets")
                    .clicked()
                {
                    self.manage_presets_tab = ManagePresetsTab::Board;
                }
                if ui
                    .selectable_label(self.manage_presets_tab == ManagePresetsTab::Live, "Live presets")
                    .clicked()
                {
                    self.manage_presets_tab = ManagePresetsTab::Live;
                }
            });
            ui.separator();

            match self.manage_presets_tab {
                ManagePresetsTab::Board => show_board_presets_tab(
                    ui,
                    self.plugin_editor.theme(),
                    &mut self.board_preset_search,
                    &board_rows,
                    &mut board_to_load,
                    &mut delete_requested,
                    &mut export_requested,
                    &mut new_board_requested,
                ),
                ManagePresetsTab::Live => show_live_presets_tab(
                    ui,
                    self.plugin_editor.theme(),
                    &mut self.live_board_search,
                    &mut self.live_preset_name,
                    &live_rows,
                    &self.presets,
                    selected_live_name.as_deref(),
                    selected_live.as_ref(),
                    loaded_preset_name.as_deref(),
                    &mut self.selected_live_preset_name,
                    &mut self.live_preset_draft,
                    &mut self.live_preset_edit_mode,
                    &mut live_save_requested,
                    &mut delete_live_requested,
                    &mut import_live_requested,
                    &mut export_current_live_requested,
                ),
            }

            if let Some(error) = &self.preset_error {
                ui.add_space(6.0);
                app_ui::error_label(ui, error);
            }
            ui.add_space(12.0);
            if app_ui::button(ui, self.plugin_editor.theme(), "Close").clicked() {
                ui.close();
            }
        });

        if response.should_close() {
            self.manage_presets_open = false;
            if delete_requested.is_none() && delete_live_requested.is_none() {
                self.preset_error = None;
            }
        }
        if let Some((name, preset)) = board_to_load {
            self.plugin_editor.request_load_preset(preset, name);
        }
        if new_board_requested {
            self.manage_presets_open = false;
            self.save_preset_name = self
                .plugin_editor
                .loaded_preset_name()
                .unwrap_or("")
                .to_owned();
            self.save_preset_is_new = self.save_preset_name.is_empty();
            self.save_preset_open = true;
        }
        if let Some(name) = delete_requested {
            self.manage_presets_open = false;
            self.delete_preset_name = Some(name);
        }
        if let Some((name, preset)) = export_requested {
            self.manage_presets_open = false;
            self.export_saved_preset(name, preset);
        }
        if let Some((name, preset)) = live_save_requested {
            if self.live_preset_edit_mode == LivePresetEditMode::TypedExistingNeedsConfirmation {
                self.manage_presets_open = false;
                self.overwrite_live_preset = Some((name, preset));
            } else {
                self.save_live_preset_contents(name, preset);
            }
        }
        if let Some(name) = delete_live_requested {
            self.manage_presets_open = false;
            self.delete_live_preset_name = Some(name);
        }
        if import_live_requested {
            self.manage_presets_open = false;
            self.import_live_preset();
        }
        if export_current_live_requested {
            self.manage_presets_open = false;
            self.export_current_live_preset();
        }
    }

    fn show_delete_preset_modal(&mut self, ctx: &egui::Context) {
        let Some(name) = self.delete_preset_name.clone() else {
            return;
        };

        let loaded = self.plugin_editor.loaded_preset_name() == Some(name.as_str());
        let mut confirm = false;
        let response = egui::Modal::new(egui::Id::new("delete-preset-modal")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.set_max_width(360.0);
            app_ui::heading(ui, "Delete preset?");
            ui.add(app_ui::Label::new(format!(
                "Permanently delete the saved preset \"{name}\"?"
            )));
            if loaded {
                ui.add_space(6.0);
                ui.add(
                    app_ui::Label::new(
                        "The current sound will remain loaded, but it will no longer be linked to this preset.",
                    )
                    .wrap(),
                );
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if app_ui::button(ui, self.plugin_editor.theme(), "Cancel").clicked() {
                    ui.close();
                }
                if app_ui::button(ui, self.plugin_editor.theme(), "Remove").clicked() {
                    confirm = true;
                    ui.close();
                }
            });
        });

        if response.should_close() {
            self.delete_preset_name = None;
        }
        if confirm {
            self.delete_preset_name = None;
            self.delete_preset(name);
        }
    }

    fn delete_preset(&mut self, name: String) {
        let was_loaded = self.plugin_editor.loaded_preset_name() == Some(name.as_str());
        match self.presets.delete(&name) {
            Ok(()) => {
                if was_loaded {
                    self.plugin_editor.clear_loaded_preset_name();
                }
                self.preset_error = None;
            }
            Err(error) => {
                self.preset_error = Some(error);
                self.manage_presets_open = true;
            }
        }
    }

    fn save_preset(&mut self, name: String) {
        let preset = match self.plugin_editor.capture_preset() {
            Ok(preset) => preset,
            Err(error) => {
                self.preset_error = Some(error);
                return;
            }
        };
        match self.presets.save(&name, &preset) {
            Ok(()) => {
                self.plugin_editor.set_loaded_preset_name(name);
                self.save_preset_open = false;
                self.preset_error = None;
            }
            Err(error) => self.preset_error = Some(error),
        }
    }

    fn export_preset(&mut self) {
        let preset = match self.plugin_editor.capture_preset() {
            Ok(preset) => preset,
            Err(error) => {
                self.plugin_editor.set_error(error);
                return;
            }
        };
        let default_name = self
            .plugin_editor
            .loaded_preset_name()
            .and_then(|name| name.rsplit('/').next())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("rack-preset")
            .to_owned();
        self.export_preset_file(&preset, &default_name);
    }

    fn export_saved_preset(&mut self, name: String, preset: Preset) {
        let default_name = name
            .rsplit('/')
            .next()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("rack-preset");
        self.export_preset_file(&preset, default_name);
    }

    fn export_preset_file(&mut self, preset: &Preset, default_name: &str) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export preset")
            .add_filter("Rack preset", &["rackpreset"])
            .set_file_name(format!("{default_name}.rackpreset"))
            .save_file()
        else {
            return;
        };
        if let Err(error) = PresetStore::export(&path, preset) {
            self.plugin_editor.set_error(error);
        }
    }

    fn import_preset(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import preset")
            .add_filter("Rack preset", &["rackpreset", "toml"])
            .pick_file()
        else {
            return;
        };
        let preset = match PresetStore::import(&path) {
            Ok(preset) => preset,
            Err(error) => {
                self.plugin_editor.set_error(error);
                return;
            }
        };
        let name = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Imported preset".to_owned());
        self.plugin_editor.request_load_preset(preset, name);
    }

    fn apply_pending_program_changes(&mut self) {
        for (_, program) in self.plugin_editor.take_program_changes() {
            let Some(live_name) = self.selected_live_preset_name.as_deref() else {
                continue;
            };
            let Some(live_preset) = self.live_presets.get(live_name) else {
                continue;
            };
            let Some(board_name) = live_preset.presets.get(program as usize) else {
                log::debug!(
                    "ignoring MIDI program change {program}: live preset {live_name} has no entry"
                );
                continue;
            };
            let Some(board_preset) = self.presets.get(board_name).cloned() else {
                self.plugin_editor.set_error(format!(
                    "Live preset {live_name} refers to missing board preset {board_name}."
                ));
                continue;
            };
            self.plugin_editor
                .request_load_preset(board_preset, board_name.clone());
        }
    }

    fn save_live_preset_contents(&mut self, name: String, preset: LivePreset) {
        match self.live_presets.save(&name, &preset) {
            Ok(()) => {
                self.selected_live_preset_name = Some(name.clone());
                self.live_preset_name = name;
                self.live_preset_draft = self
                    .selected_live_preset_name
                    .as_deref()
                    .and_then(|name| self.live_presets.get(name))
                    .cloned();
                self.live_preset_edit_mode = LivePresetEditMode::SidebarSelection;
                self.preset_error = None;
            }
            Err(error) => self.preset_error = Some(error),
        }
    }

    fn show_delete_live_preset_modal(&mut self, ctx: &egui::Context) {
        let Some(name) = self.delete_live_preset_name.clone() else {
            return;
        };
        let mut confirm = false;
        let response =
            egui::Modal::new(egui::Id::new("delete-live-preset-modal")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.set_max_width(360.0);
                app_ui::heading(ui, "Delete live preset?");
                ui.add(
                    app_ui::Label::new(format!("Permanently delete the live preset \"{name}\"?"))
                        .wrap(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if app_ui::button(ui, self.plugin_editor.theme(), "Cancel").clicked() {
                        ui.close();
                    }
                    if app_ui::button(ui, self.plugin_editor.theme(), "Remove").clicked() {
                        confirm = true;
                        ui.close();
                    }
                });
            });
        if response.should_close() {
            self.delete_live_preset_name = None;
        }
        if confirm {
            self.delete_live_preset_name = None;
            match self.live_presets.delete(&name) {
                Ok(()) => {
                    if self.selected_live_preset_name.as_deref() == Some(name.as_str()) {
                        self.selected_live_preset_name = self
                            .live_presets
                            .entries()
                            .first()
                            .map(|entry| entry.segments.join("/"));
                        self.live_preset_name =
                            self.selected_live_preset_name.clone().unwrap_or_default();
                        self.live_preset_draft = self
                            .selected_live_preset_name
                            .as_deref()
                            .and_then(|name| self.live_presets.get(name))
                            .cloned();
                        self.live_preset_edit_mode = if self.selected_live_preset_name.is_some() {
                            LivePresetEditMode::SidebarSelection
                        } else {
                            LivePresetEditMode::None
                        };
                    }
                    self.preset_error = None;
                }
                Err(error) => {
                    self.preset_error = Some(error);
                    self.manage_presets_tab = ManagePresetsTab::Live;
                    self.manage_presets_open = true;
                }
            }
        }
    }

    fn export_saved_live_preset(&mut self, name: String, preset: LivePreset) {
        let default_name = name
            .rsplit('/')
            .next()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("live-preset");
        self.export_live_preset_file(&preset, default_name);
    }

    fn export_current_live_preset(&mut self) {
        let Some(name) = self.selected_live_preset_name.clone() else {
            self.plugin_editor
                .set_error("Select a live preset to export.");
            return;
        };
        let Some(preset) = self.live_presets.get(&name).cloned() else {
            return;
        };
        self.export_saved_live_preset(name, preset);
    }

    fn export_live_preset_file(&mut self, preset: &LivePreset, default_name: &str) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export live preset")
            .add_filter("Rack live preset", &["racklive"])
            .set_file_name(format!("{default_name}.racklive"))
            .save_file()
        else {
            return;
        };
        if let Err(error) = LivePresetStore::export(&path, preset) {
            self.plugin_editor.set_error(error);
        }
    }

    fn import_live_preset(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import live preset")
            .add_filter("Rack live preset", &["racklive", "toml"])
            .pick_file()
        else {
            return;
        };
        let mut preset = match LivePresetStore::import(&path) {
            Ok(preset) => preset,
            Err(error) => {
                self.plugin_editor.set_error(error);
                return;
            }
        };
        let name = if preset.name.trim().is_empty() {
            path.file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Imported live preset".to_owned())
        } else {
            preset.name.trim().to_owned()
        };
        preset.name = name.clone();
        self.save_live_preset_contents(name, preset);
    }
}

fn square_icon_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    icon: Icon,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    app_ui::square_button(ui, theme, icon, enabled, tooltip)
}

#[allow(clippy::too_many_arguments)]
fn show_board_presets_tab(
    ui: &mut egui::Ui,
    theme: &Theme,
    search: &mut String,
    rows: &[(String, bool, Preset)],
    board_to_load: &mut Option<(String, Preset)>,
    delete_requested: &mut Option<String>,
    export_requested: &mut Option<(String, Preset)>,
    new_requested: &mut bool,
) {
    ui.horizontal(|ui| {
        app_ui::heading(ui, "Board presets");
        if app_ui::button(ui, theme, "New board preset...").clicked() {
            *new_requested = true;
        }
    });
    ui.horizontal(|ui| {
        app_ui::label(ui, "Search");
        ui.add(
            egui::TextEdit::singleline(search)
                .hint_text("Filter board presets")
                .desired_width(ui.available_width()),
        );
    });
    ui.add_space(6.0);

    let search = search.trim().to_ascii_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("board-preset-directory")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut visible = 0;
            for (name, loaded, preset) in rows {
                if !search.is_empty() && !name.to_ascii_lowercase().contains(&search) {
                    continue;
                }
                visible += 1;
                ui.push_id(name, |ui| {
                    ui.horizontal(|ui| {
                        if app_ui::button(ui, theme, "Load").clicked() {
                            *board_to_load = Some((name.clone(), preset.clone()));
                            ui.close();
                        }
                        app_ui::label(ui, name);
                        if *loaded {
                            app_ui::muted(ui, "Loaded");
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if app_ui::button_with_type(
                                ui,
                                theme,
                                "Remove",
                                app_ui::ButtonKind::Danger,
                            )
                            .clicked()
                            {
                                *delete_requested = Some(name.clone());
                                ui.close();
                            }
                            if app_ui::button(ui, theme, "Export").clicked() {
                                *export_requested = Some((name.clone(), preset.clone()));
                                ui.close();
                            }
                        });
                    });
                });
            }
            if visible == 0 {
                app_ui::muted(
                    ui,
                    if rows.is_empty() {
                        "No board presets saved."
                    } else {
                        "No board presets match the search."
                    },
                );
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn show_live_presets_tab(
    ui: &mut egui::Ui,
    theme: &Theme,
    board_search: &mut String,
    live_preset_name: &mut String,
    live_rows: &[(String, LivePreset)],
    board_store: &PresetStore,
    _selected_name: Option<&str>,
    selected_live: Option<&LivePreset>,
    loaded_board_name: Option<&str>,
    selected_name_output: &mut Option<String>,
    live_preset_draft: &mut Option<LivePreset>,
    edit_mode: &mut LivePresetEditMode,
    save_requested: &mut Option<(String, LivePreset)>,
    delete_requested: &mut Option<String>,
    _import_requested: &mut bool,
    _export_current_requested: &mut bool,
) {
    let mut working = selected_live
        .cloned()
        .or_else(|| live_preset_draft.clone())
        .unwrap_or(LivePreset {
            name: String::new(),
            presets: Vec::new(),
        });
    let mut save_clicked = false;
    let mut selected_from_directory = None;
    let mut board_to_add = None;
    let mut stack_action = None;

    ui.horizontal_top(|ui| {
        let available_height = ui.available_height();
        let left_width = 220.0;
        ui.allocate_ui_with_layout(
            egui::vec2(left_width, available_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_height(available_height);
                    app_ui::heading(ui, "Live presets");
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_salt("live-preset-directory")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (name, preset) in live_rows {
                                ui.push_id(("live-directory", name), |ui| {
                                    ui.horizontal(|ui| {
                                        let preset_link = ui
                                            .add(
                                                app_ui::Label::new(
                                                    egui::RichText::new(name)
                                                        .underline()
                                                        .color(ui.visuals().hyperlink_color),
                                                )
                                                .sense(egui::Sense::click()),
                                            )
                                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                                        if preset_link.clicked() {
                                            selected_from_directory =
                                                Some((name.clone(), preset.clone()));
                                        }
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if square_icon_button(
                                                    ui,
                                                    theme,
                                                    Icon::Trash2,
                                                    true,
                                                    "Delete live preset",
                                                )
                                                .clicked()
                                                {
                                                    *delete_requested = Some(name.clone());
                                                    ui.close();
                                                }
                                            },
                                        );
                                    });
                                });
                            }
                            if live_rows.is_empty() {
                                app_ui::muted(ui, "No live presets saved.");
                            }
                        });
                });
            },
        );

        ui.add_space(ui.spacing().item_spacing.x);
        let editor_width = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(editor_width, available_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                app_ui::heading(ui, "Edit preset");
                let mut name_finished = false;
                ui.horizontal(|ui| {
                    let save_width = 72.0;
                    let input_width =
                        (ui.available_width() - save_width - ui.spacing().item_spacing.x).max(0.0);
                    let name_response = ui.add(
                        egui::TextEdit::singleline(live_preset_name)
                            .desired_width(input_width)
                            .hint_text("e.g. My live preset"),
                    );
                    name_finished = name_response.changed()
                        && (name_response.lost_focus()
                            || ui.input(|input| input.key_pressed(egui::Key::Enter)));
                    if app_ui::Button::new("Save")
                        .enabled(!live_preset_name.trim().is_empty())
                        .show(ui, theme)
                        .clicked()
                    {
                        save_clicked = true;
                    }
                });
                if name_finished {
                    let name = live_preset_name.trim().to_owned();
                    if !name.is_empty() {
                        if let Some((_, existing)) = live_rows
                            .iter()
                            .find(|(existing_name, _)| existing_name == &name)
                        {
                            if working.presets.is_empty() {
                                working = LivePreset {
                                    name: name.clone(),
                                    presets: Vec::new(),
                                };
                                *edit_mode = LivePresetEditMode::TypedExistingNeedsConfirmation;
                            } else {
                                working = existing.clone();
                                *edit_mode = LivePresetEditMode::SidebarSelection;
                            }
                        } else {
                            working = LivePreset {
                                name: name.clone(),
                                presets: Vec::new(),
                            };
                            *edit_mode = LivePresetEditMode::TypedNew;
                        }
                        *selected_name_output = Some(name);
                        *live_preset_draft = Some(working.clone());
                    }
                }
                ui.add_space(4.0);
                let panel_height = ui.available_height();
                ui.columns(2, |columns| {
                    let middle = &mut columns[0];
                    egui::Frame::group(middle.style()).show(middle, |ui| {
                        ui.set_height(panel_height);
                        app_ui::heading(ui, "Board presets");
                        ui.add_space(4.0);
                        let board_search_value = board_search.trim().to_ascii_lowercase();
                        egui::ScrollArea::vertical()
                            .id_salt("live-board-directory")
                            .max_height(370.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let mut visible = 0;
                                for entry in board_store.entries() {
                                    let name = entry.segments.join("/");
                                    if !board_search_value.is_empty()
                                        && !name.to_ascii_lowercase().contains(&board_search_value)
                                    {
                                        continue;
                                    }
                                    visible += 1;
                                    ui.push_id(("live-board", &name), |ui| {
                                        ui.horizontal(|ui| {
                                            app_ui::label(ui, name.as_str());
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if square_icon_button(
                                                        ui,
                                                        theme,
                                                        Icon::Plus,
                                                        true,
                                                        "Add to the stack",
                                                    )
                                                    .clicked()
                                                    {
                                                        board_to_add = Some(name.clone());
                                                    }
                                                },
                                            );
                                        });
                                    });
                                }
                                if visible == 0 {
                                    app_ui::muted(ui, "No board presets match the search.");
                                }
                            });
                        ui.add(
                            egui::TextEdit::singleline(board_search)
                                .hint_text("e.g. Chorus clean")
                                .desired_width(f32::INFINITY),
                        );
                    });

                    let right = &mut columns[1];
                    egui::Frame::group(right.style()).show(right, |ui| {
                        ui.set_height(panel_height);
                        app_ui::heading(ui, "Stack");
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .id_salt("live-preset-contents")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for index in 0..working.presets.len() {
                                    let preset_name = working.presets[index].clone();
                                    let is_loaded = loaded_board_name == Some(preset_name.as_str());
                                    ui.push_id(("live-entry", index), |ui| {
                                        ui.horizontal(|ui| {
                                            app_ui::label(ui, format!("{}.", index + 1));
                                            ui.add(app_ui::Label::new(&preset_name).truncate());
                                            if board_store.get(&preset_name).is_none() {
                                                app_ui::warning_label(ui, "Missing");
                                            } else if is_loaded {
                                                app_ui::muted(ui, "Active");
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if false
                                                        && ui
                                                            .button("🗑")
                                                            .on_hover_text("Remove")
                                                            .clicked()
                                                    {
                                                        stack_action =
                                                            Some(LiveEntryAction::Remove(index));
                                                    }
                                                    if square_icon_button(
                                                        ui,
                                                        theme,
                                                        Icon::Trash2,
                                                        true,
                                                        "Remove",
                                                    )
                                                    .clicked()
                                                    {
                                                        stack_action =
                                                            Some(LiveEntryAction::Remove(index));
                                                    }
                                                    if false
                                                        && ui
                                                            .add_enabled(
                                                                index + 1 < working.presets.len(),
                                                                egui::Button::new("⌄"),
                                                            )
                                                            .on_hover_text("Move down")
                                                            .clicked()
                                                    {
                                                        stack_action = Some(LiveEntryAction::Move(
                                                            index,
                                                            index + 2,
                                                        ));
                                                    }
                                                    if square_icon_button(
                                                        ui,
                                                        theme,
                                                        Icon::ChevronDown,
                                                        index + 1 < working.presets.len(),
                                                        "Move down",
                                                    )
                                                    .clicked()
                                                    {
                                                        stack_action = Some(LiveEntryAction::Move(
                                                            index,
                                                            index + 2,
                                                        ));
                                                    }
                                                    if false
                                                        && ui
                                                            .add_enabled(
                                                                index > 0,
                                                                egui::Button::new("⌃"),
                                                            )
                                                            .on_hover_text("Move up")
                                                            .clicked()
                                                    {
                                                        stack_action = Some(LiveEntryAction::Move(
                                                            index,
                                                            index - 1,
                                                        ));
                                                    }
                                                    if square_icon_button(
                                                        ui,
                                                        theme,
                                                        Icon::ChevronUp,
                                                        index > 0,
                                                        "Move up",
                                                    )
                                                    .clicked()
                                                    {
                                                        stack_action = Some(LiveEntryAction::Move(
                                                            index,
                                                            index - 1,
                                                        ));
                                                    }
                                                },
                                            );
                                        });
                                    });
                                }
                                if working.presets.is_empty() {
                                    app_ui::muted(ui, "Add board presets from the middle panel.");
                                }
                            });
                    });
                });
            },
        );
    });

    if let Some((name, preset)) = selected_from_directory {
        *selected_name_output = Some(name.clone());
        *live_preset_name = name;
        *live_preset_draft = Some(preset);
        *edit_mode = LivePresetEditMode::SidebarSelection;
    }
    let contents_changed = board_to_add.is_some() || stack_action.is_some();
    if let Some(name) = board_to_add
        && !working.presets.contains(&name)
    {
        working.presets.push(name);
    }
    if let Some(action) = stack_action {
        match action {
            LiveEntryAction::Remove(index) => {
                working.presets.remove(index);
            }
            LiveEntryAction::Move(from, to) => move_item(&mut working.presets, from, to),
        }
    }
    if contents_changed {
        *live_preset_draft = Some(working.clone());
    }
    if save_clicked {
        *save_requested = Some((live_preset_name.trim().to_owned(), working));
    }
}

enum LiveEntryAction {
    Remove(usize),
    Move(usize, usize),
}

fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from >= items.len() || to > items.len() {
        return;
    }
    let item = items.remove(from);
    let adjusted_to = if from < to { to - 1 } else { to };
    items.insert(adjusted_to, item);
}

#[derive(Default)]
struct PresetMenuNode {
    categories: BTreeMap<String, PresetMenuNode>,
    presets: Vec<(String, String, Preset)>,
}

fn build_preset_menu_items(
    entries: &[PresetEntry],
) -> Vec<app_ui::MenuButtonItem<(String, Preset)>> {
    let mut root = PresetMenuNode::default();
    for entry in entries {
        let Some((name, categories)) = entry.segments.split_last() else {
            continue;
        };
        let mut node = &mut root;
        for category in categories {
            node = node.categories.entry(category.clone()).or_default();
        }
        node.presets
            .push((name.clone(), entry.segments.join("/"), entry.preset.clone()));
    }
    root.into_items()
}

impl PresetMenuNode {
    fn into_items(self) -> Vec<app_ui::MenuButtonItem<(String, Preset)>> {
        let categories = self
            .categories
            .into_iter()
            .map(|(label, node)| app_ui::MenuButtonItem::submenu(label, node.into_items()));
        let presets = self
            .presets
            .into_iter()
            .map(|(label, name, preset)| app_ui::MenuButtonItem::item(label, (name, preset)));
        categories.chain(presets).collect()
    }
}

struct StartupTask {
    receiver: Receiver<Result<StartupResult, String>>,
    progress: Arc<AtomicU8>,
    plugins_loaded: Arc<AtomicUsize>,
    plugins_total: Arc<AtomicUsize>,
}

struct StartupResult {
    settings: SettingsStore,
    discovered_plugins: Vec<DiscoveredPlugin>,
}

impl StartupTask {
    fn spawn() -> Self {
        let (sender, receiver) = mpsc::channel();
        let progress = Arc::new(AtomicU8::new(0));
        let thread_progress = Arc::clone(&progress);
        let plugins_loaded = Arc::new(AtomicUsize::new(0));
        let thread_plugins_loaded = Arc::clone(&plugins_loaded);
        let plugins_total = Arc::new(AtomicUsize::new(0));
        let thread_plugins_total = Arc::clone(&plugins_total);

        thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                thread_progress.store(5, Ordering::Release);
                let settings = SettingsStore::load();
                thread_progress.store(20, Ordering::Release);
                let discovered_plugins = discover_plugins_with_progress(
                    &settings.values.plugin_discovery,
                    |value, loaded, total| {
                        thread_plugins_loaded.store(loaded, Ordering::Release);
                        thread_plugins_total.store(total, Ordering::Release);
                        thread_progress.store(value.round() as u8, Ordering::Release);
                    },
                );
                thread_progress.store(100, Ordering::Release);
                StartupResult {
                    settings,
                    discovered_plugins,
                }
            })
            .map_err(|panic| {
                panic
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown startup error")
                    .to_owned()
            });
            let _ = sender.send(result);
        });

        Self {
            receiver,
            progress,
            plugins_loaded,
            plugins_total,
        }
    }
}
