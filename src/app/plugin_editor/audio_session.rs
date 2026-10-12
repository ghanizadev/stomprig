use super::super::audio::AudioDevices;
use super::super::audio::{AudioConfig, AudioEngine, ChannelRouting, ShutdownMode, SignalLevels};
use super::super::diagnostics::{FixedText, SUBJECT_BYTES};
use super::super::plugin_runtime::PluginRuntimeStatus;
use super::PluginEditor;
use std::sync::Arc;

impl PluginEditor {
    pub(super) fn start_audio(&mut self) {
        if self.engine.is_some() {
            return;
        }
        let Some(config) = self.selected_audio_config() else {
            self.state = super::EditorState::AudioUnavailable;
            self.error = Some("Editors loaded, but no audio devices are selected.".to_string());
            return;
        };
        let Some(sample_rate) =
            AudioDevices::sample_rate(config.backend, &config.output_device_id).ok()
        else {
            self.state = super::EditorState::AudioUnavailable;
            self.error = Some("Could not read the selected output sample rate.".to_owned());
            return;
        };
        let block_size = config.buffer_size.frames() as usize;
        let mut activation_error = None;
        for index in 0..self.rack.slots.len() {
            let plugin_name = self.rack.slots[index].snapshot.name.clone();
            let activation = self.rack.slots[index].clap.as_mut().map(|plugin| {
                plugin
                    .deactivate()
                    .and_then(|()| plugin.activate(sample_rate, block_size))
            });
            let Some(activation) = activation else {
                continue;
            };
            match activation {
                Ok(processor) => {
                    if let Err(error) = self
                        .runtime
                        .handle()
                        .replace_clap_processor(index, processor)
                    {
                        activation_error =
                            Some(format!("Could not prepare CLAP processor: {error}"));
                        break;
                    }
                }
                Err(error) => {
                    activation_error = Some(format!("Could not activate {plugin_name}: {error}"));
                    break;
                }
            }
        }
        if let Some(error) = activation_error {
            let _ = self.runtime.handle().stop_audio();
            if self.runtime.status() == PluginRuntimeStatus::Running {
                self.deactivate_clap_plugins();
            }
            self.state = super::EditorState::AudioUnavailable;
            self.error = Some(error);
            return;
        }
        match AudioEngine::start(
            self.runtime.handle(),
            config.clone(),
            Arc::clone(&self.playback),
            Arc::clone(&self.tuner),
            self.rack
                .slots
                .iter()
                .map(|slot| FixedText::<SUBJECT_BYTES>::new(&slot.snapshot.name))
                .collect(),
        ) {
            Ok(mut engine) => {
                self.diagnostic_consumers
                    .extend(engine.take_diagnostic_consumers());
                self.engine = Some(engine);
                self.error = None;
            }
            Err(error) => {
                let _ = self.runtime.handle().stop_audio();
                if self.runtime.status() == PluginRuntimeStatus::Running {
                    self.deactivate_clap_plugins();
                }
                self.state = super::EditorState::AudioUnavailable;
                self.error = Some(format!("Editors loaded, but audio is unavailable: {error}"));
            }
        }
    }

    pub(super) fn restart_audio(&mut self) {
        if self.selected_audio_config().is_none() {
            self.error = Some("Select input and output devices and channels.".to_string());
            return;
        }
        if let Some(engine) = self.engine.take()
            && let Err(error) = engine.shutdown(ShutdownMode::UnloadPlugins)
        {
            self.state = super::EditorState::AudioUnavailable;
            self.error = Some(format!("Could not stop audio before restart: {error}"));
            return;
        }
        self.deactivate_clap_plugins();
        self.start_audio();
    }

    pub(super) fn restart_audio_for_clap_request(&mut self) {
        if let Some(engine) = self.engine.take()
            && let Err(error) = engine.shutdown(ShutdownMode::UnloadPlugins)
        {
            self.state = super::EditorState::AudioUnavailable;
            self.error = Some(format!("Could not stop audio before CLAP restart: {error}"));
            return;
        }
        self.deactivate_clap_plugins();
        self.start_audio();
    }

    pub(super) fn stop_audio_for_plugin_change(&mut self) -> Option<bool> {
        let Some(engine) = self.engine.take() else {
            return Some(false);
        };
        if let Err(error) = engine.shutdown(ShutdownMode::UnloadPlugins) {
            self.state = super::EditorState::AudioUnavailable;
            self.error = Some(format!(
                "Could not stop audio before changing plugins: {error}"
            ));
            return None;
        }
        if self.runtime.status() == PluginRuntimeStatus::Running {
            self.deactivate_clap_plugins();
        }
        Some(true)
    }

    pub(super) fn restart_audio_after_plugin_change(
        &mut self,
        was_running: bool,
    ) -> Result<(), String> {
        if was_running {
            if self.runtime.status() != PluginRuntimeStatus::Running {
                self.state = super::EditorState::AudioUnavailable;
                return Err(
                    "Audio could not restart because the plugin runtime is no longer available."
                        .to_owned(),
                );
            }
            self.start_audio();
            if self.state == super::EditorState::AudioUnavailable {
                return Err(self
                    .error
                    .clone()
                    .unwrap_or_else(|| "Audio could not be restarted.".to_owned()));
            }
        }
        Ok(())
    }

    fn selected_audio_config(&self) -> Option<AudioConfig> {
        Some(AudioConfig {
            backend: self.settings.backend,
            input_device_id: self.settings.input_device_id.clone()?,
            output_device_id: self.settings.output_device_id.clone()?,
            buffer_size: self.settings.buffer_size,
            routing: ChannelRouting {
                input_left: self.settings.input_left_channel?,
                input_right: self.settings.input_right_channel?,
                output_left: self.settings.output_left_channel?,
                output_right: self.settings.output_right_channel?,
            },
        })
    }

    pub(crate) fn audio_levels(&self) -> Option<SignalLevels> {
        self.engine.as_ref().map(AudioEngine::levels)
    }

    pub(crate) fn estimated_latency_ms(&self) -> Option<f64> {
        let engine = self.engine.as_ref()?;
        let plugin_latency_samples = self
            .rack
            .slots
            .iter()
            // Status-bar latency is refreshed every UI frame; avoid taking a plugin lock here.
            .map(|slot| f64::from(slot.snapshot.latency_samples))
            .sum();
        engine.latency_info().estimate_ms(plugin_latency_samples)
    }
}
