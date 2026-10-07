mod audio;
mod cpu;
pub mod diagnostics;
mod loading;
mod plugin;
mod plugin_editor;
mod plugin_runtime;
mod presets;
mod settings;
mod splash;
mod status_bar;
mod windows;

use audio::ShutdownMode;
use cpu::CpuUsage;
use eframe::Frame;
use egui_shadcn::Theme;
use plugin_editor::{DiscoveredPlugin, PluginEditor, discover_plugins_with_progress};
use presets::{Preset, PresetEntry, PresetStore};
use settings::SettingsStore;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

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
    save_preset_open: bool,
    save_preset_name: String,
    save_preset_is_new: bool,
    preset_error: Option<String>,
    overwrite_preset_name: Option<String>,
}

impl Default for RackApp {
    fn default() -> Self {
        let startup = StartupTask::spawn();
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
            save_preset_open: false,
            save_preset_name: String::new(),
            save_preset_is_new: true,
            preset_error: None,
            overwrite_preset_name: None,
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
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            ui.add_enabled_ui(main_window_enabled, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Close").clicked() {
                            ui.close();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });

                    ui.menu_button("Edit", |ui| {
                        if ui.button("Settings").clicked() {
                            ui.close();
                            self.plugin_editor.open_audio_configuration();
                        }
                        if ui
                            .add_enabled(
                                self.plugin_editor.has_plugin_stack(),
                                egui::Button::new("Clear Plugin Stack"),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.plugin_editor.clear_plugin_stack();
                        }
                    });

                    ui.menu_button("Presets", |ui| {
                        if ui
                            .add_enabled(
                                self.plugin_editor.has_loaded_plugin_stack(),
                                egui::Button::new("Save preset"),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.save_preset_name.clear();
                            self.save_preset_is_new = true;
                            self.preset_error = None;
                            self.overwrite_preset_name = None;
                            self.save_preset_open = true;
                        }

                        ui.menu_button("Load preset", |ui| {
                            if self.presets.entries().is_empty() {
                                ui.label("No presets saved.");
                            } else {
                                let tree = build_preset_menu_tree(self.presets.entries());
                                show_preset_menu(ui, &tree, &mut preset_to_load);
                            }
                        });
                    });
                });
            });
        });

        if let Some((name, preset)) = preset_to_load {
            self.plugin_editor.request_load_preset(preset, name);
        }

        egui::TopBottomPanel::bottom("status_bar")
            .resizable(false)
            .show(ctx, |ui| {
                status_bar::show(
                    ui,
                    &mut self.cpu_usage,
                    self.plugin_editor.edit_state_label(),
                    self.plugin_editor.loaded_preset_name(),
                    self.plugin_editor.audio_levels(),
                    self.plugin_editor.estimated_latency_ms(),
                    &mut self.audio_meters,
                    &mut self.latency,
                );
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(main_window_enabled, |ui| {
                self.plugin_editor.ui(ui);
                let values = self.current_settings();
                if let Some(settings) = self.settings.as_mut() {
                    settings.save_if_changed(&values);
                    if let Some(error) = &settings.error {
                        ui.colored_label(egui::Color32::RED, error);
                    }
                }
            });
        });

        if loading || self.plugin_editor.is_loading() {
            ctx.set_cursor_icon(egui::CursorIcon::Progress);
            self.plugin_editor.show_loading(ctx);
        }
        self.show_save_preset_modal(ctx);
        self.show_overwrite_preset_modal(ctx);
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
                self.plugin_editor = PluginEditor::from_settings(&result.settings.values.audio);
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

    fn current_settings(&self) -> settings::AppSettings {
        settings::AppSettings {
            audio: self.plugin_editor.audio_settings(),
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
        let mut save_requested = false;
        let response = egui::Modal::new(egui::Id::new("save-preset-modal")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.set_max_width(360.0);
            ui.heading("Save preset");
            ui.add(
                egui::Label::new(
                    "Select an existing preset to overwrite it, or create a new preset.",
                )
                .wrap(),
            );
            ui.add_space(8.0);
            egui::ComboBox::from_label("Preset name")
                .selected_text(if self.save_preset_is_new {
                    "New preset..."
                } else {
                    self.save_preset_name.as_str()
                })
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.save_preset_is_new, "New preset...")
                        .clicked()
                    {
                        self.save_preset_is_new = true;
                        self.save_preset_name.clear();
                        ui.close();
                    }
                    for name in &existing_names {
                        if ui
                            .selectable_label(
                                !self.save_preset_is_new && self.save_preset_name == *name,
                                name,
                            )
                            .clicked()
                        {
                            self.save_preset_is_new = false;
                            self.save_preset_name = name.clone();
                            ui.close();
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
                let use_loaded = ui.button("Use loaded preset name");
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
                ui.colored_label(egui::Color32::RED, error);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !self.save_preset_name.trim().is_empty(),
                        egui::Button::new("Save"),
                    )
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
            ui.heading("Overwrite preset?");
            ui.add(egui::Label::new(format!("Replace the saved preset \"{name}\"?")).wrap());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
                if ui.button("Overwrite").clicked() {
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

    fn save_preset(&mut self, name: String) {
        match self
            .plugin_editor
            .capture_preset()
            .and_then(|preset| self.presets.save(&name, &preset))
        {
            Ok(()) => {
                self.save_preset_open = false;
                self.preset_error = None;
            }
            Err(error) => self.preset_error = Some(error),
        }
    }
}

#[derive(Default)]
struct PresetMenuNode {
    categories: BTreeMap<String, PresetMenuNode>,
    presets: Vec<(String, String, Preset)>,
}

fn build_preset_menu_tree(entries: &[PresetEntry]) -> PresetMenuNode {
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
    root
}

fn show_preset_menu(
    ui: &mut egui::Ui,
    node: &PresetMenuNode,
    selected: &mut Option<(String, Preset)>,
) {
    for (category, child) in &node.categories {
        ui.menu_button(category, |ui| show_preset_menu(ui, child, selected));
    }
    for (name, full_name, preset) in &node.presets {
        if ui.button(name).clicked() {
            *selected = Some((full_name.clone(), preset.clone()));
            ui.close();
        }
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
                let discovered_plugins = discover_plugins_with_progress(|value, loaded, total| {
                    thread_plugins_loaded.store(loaded, Ordering::Release);
                    thread_plugins_total.store(total, Ordering::Release);
                    thread_progress.store(value.round() as u8, Ordering::Release);
                });
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
