use super::super::plugin::service_run_loop;
use super::PluginEditor;

impl PluginEditor {
    pub(super) fn service_editor_windows(&mut self) {
        let mut restart_requested = false;
        let mut parameter_rescan_indices = Vec::new();
        for index in 0..self.rack.slots.len() {
            let slot = &mut self.rack.slots[index];
            if let Err(error) = slot.editor_window.service_platform_events() {
                self.state = super::EditorState::Error;
                self.error = Some(format!(
                    "Could not service {} editor: {error}",
                    slot.snapshot.name
                ));
            }
            if slot.editor_window.closed_by_user() {
                slot.editor_window.close();
            }
            if let Some(plugin) = &mut slot.clap {
                let (restart, latency_changed, parameter_rescan_requested) =
                    plugin.service_main_thread_callbacks();
                restart_requested |= restart;
                if parameter_rescan_requested {
                    parameter_rescan_indices.push(index);
                }
                if latency_changed {
                    slot.snapshot.latency_samples = plugin.latency_samples();
                }
            }
            if let Some(instance) = &slot.instance {
                service_run_loop(instance);
            }
        }
        if restart_requested {
            self.restart_audio_for_clap_request();
        }
        for index in parameter_rescan_indices {
            let Some(slot) = self.rack.slots.get_mut(index) else {
                continue;
            };
            let Some(plugin) = &mut slot.clap else {
                continue;
            };
            match super::clap_plugin_snapshot(plugin) {
                Ok(snapshot) => {
                    slot.snapshot.parameters = snapshot.parameters;
                    slot.snapshot.latency_samples = snapshot.latency_samples;
                    slot.midi_mappings.retain(|mapping| match mapping.action {
                        super::super::midi::MidiAction::SetClapParameter { parameter_id } => slot
                            .snapshot
                            .parameters
                            .iter()
                            .any(|parameter| parameter.id == parameter_id),
                        super::super::midi::MidiAction::ToggleBypass => true,
                    });
                    self.sync_midi_mappings();
                }
                Err(error) => {
                    self.error = Some(format!(
                        "Could not refresh {} parameters: {error}",
                        slot.snapshot.name
                    ));
                }
            }
        }
    }
}
