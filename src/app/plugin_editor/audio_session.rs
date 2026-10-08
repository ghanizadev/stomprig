use super::super::audio::{AudioConfig, AudioEngine, ChannelRouting, ShutdownMode, SignalLevels};
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
        match AudioEngine::start(
            self.runtime.handle(),
            config,
            Arc::clone(&self.playback),
            Arc::clone(&self.tuner),
        ) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.active_audio_config = Some(config);
                self.error = None;
            }
            Err(error) => {
                self.state = super::EditorState::AudioUnavailable;
                self.error = Some(format!("Editors loaded, but audio is unavailable: {error}"));
            }
        }
    }

    pub(super) fn restart_audio(&mut self) {
        let Some(config) = self.selected_audio_config() else {
            self.error = Some("Select input and output devices and channels.".to_string());
            return;
        };
        if self.engine.is_some() && self.active_audio_config == Some(config) {
            return;
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown(ShutdownMode::UnloadPlugins);
        }
        self.active_audio_config = None;
        self.start_audio();
    }

    pub(super) fn stop_audio_for_plugin_change(&mut self) -> bool {
        let Some(engine) = self.engine.take() else {
            return false;
        };
        engine.shutdown(ShutdownMode::UnloadPlugins);
        self.active_audio_config = None;
        true
    }

    pub(super) fn restart_audio_after_plugin_change(&mut self, was_running: bool) {
        if was_running {
            self.start_audio();
        }
    }

    fn selected_audio_config(&self) -> Option<AudioConfig> {
        Some(AudioConfig {
            backend: self.settings.backend,
            input_index: self.settings.input_index?,
            output_index: self.settings.output_index?,
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
            .enumerate()
            .map(|(index, slot)| {
                slot.instance
                    .try_lock()
                    .ok()
                    .map(|plugin| f64::from(plugin.latency_samples()))
                    .or_else(|| {
                        self.rack
                            .slots
                            .get(index)
                            .map(|slot| f64::from(slot.snapshot.latency_samples))
                    })
                    .unwrap_or_default()
            })
            .sum();
        engine.latency_info().estimate_ms(plugin_latency_samples)
    }
}
