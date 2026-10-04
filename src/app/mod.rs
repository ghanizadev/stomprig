mod audio;
pub mod diagnostics;
mod plugin;
mod plugin_editor;
mod plugin_runtime;
mod settings;
mod splash;
mod windows;

use audio::ShutdownMode;
use eframe::Frame;
use egui_shadcn::Theme;
use plugin_editor::{DiscoveredPlugin, PluginEditor, discover_plugins_with_progress};
use settings::SettingsStore;
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

        let main_window_enabled = !self.plugin_editor.audio_configuration_open();
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
                    });
                });
            });
        });

        egui::TopBottomPanel::bottom("audio_monitor_footer")
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_enabled_ui(main_window_enabled, |ui| {
                    self.plugin_editor.audio_ui(ui);
                });
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
