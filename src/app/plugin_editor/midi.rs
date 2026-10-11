use super::super::midi::{MidiAction, PresetMidiMapping};
use super::PluginEditor;
use rtrb::RingBuffer;
use vst3_host::MidiEvent;
use vst3_host::midi_input::connect;

impl PluginEditor {
    pub(super) fn sync_midi_connection(&mut self) {
        let desired = if self.settings.midi_enabled {
            self.settings.midi_input_name.clone()
        } else {
            None
        };
        let available = self
            .settings
            .midi_input_devices
            .iter()
            .map(|device| device.name().to_owned())
            .collect::<Vec<_>>();
        if self.midi_connection_target == desired
            && self.midi_device_snapshot == available
            && self.midi_devices_revision == self.settings.midi_devices_revision
        {
            return;
        }

        self.midi_connection.take();
        self.midi_connection_error = None;
        if let Err(error) = self.runtime.handle().set_midi_ingress(None) {
            self.midi_connection_error = Some(format!("Could not reset MIDI input: {error}"));
        }

        if let Some(device_name) = desired.as_deref() {
            let Some(port) = self
                .settings
                .midi_input_devices
                .iter()
                .find(|device| device.name() == device_name)
                .cloned()
            else {
                self.midi_connection_error = Some(format!(
                    "MIDI input device \"{device_name}\" is unavailable."
                ));
                self.midi_connection_target = desired;
                self.midi_device_snapshot = available;
                return;
            };

            let (mut producer, consumer) = RingBuffer::new(256);
            match connect(&port, move |event| {
                let _ = producer.push(event);
            }) {
                Ok(connection) => {
                    if let Err(error) = self.runtime.handle().set_midi_ingress(Some(consumer)) {
                        self.midi_connection_error = Some(error);
                    } else {
                        self.midi_connection = Some(connection);
                    }
                }
                Err(error) => {
                    self.midi_connection_error =
                        Some(format!("Could not connect MIDI input: {error}"));
                }
            }
        }
        self.midi_connection_target = desired;
        self.midi_device_snapshot = available;
        self.midi_devices_revision = self.settings.midi_devices_revision;
    }

    pub(super) fn poll_midi_feedback(&mut self) {
        while let Ok(event) = self.midi_feedback.pop() {
            if let MidiEvent::ProgramChange { channel, program } = event {
                self.pending_program_changes
                    .push((channel.as_index() + 1, program));
                continue;
            }
            let Some(plugin_index) = self.midi_learn_plugin else {
                continue;
            };
            let mapping = match self.midi_learn_parameter {
                Some(parameter_id) => PresetMidiMapping::from_parameter_event(&event, parameter_id),
                None => PresetMidiMapping::from_event(&event, self.settings.midi_controller_mode),
            };
            let Some(mapping) = mapping else {
                continue;
            };
            if let Some(mappings) = self
                .rack
                .slots
                .get_mut(plugin_index)
                .map(|slot| &mut slot.midi_mappings)
            {
                match mapping.action {
                    MidiAction::ToggleBypass => {
                        mappings.retain(|existing| {
                            !matches!(existing.action, MidiAction::ToggleBypass)
                        });
                    }
                    MidiAction::SetParameter { parameter_id } => {
                        mappings.retain(|existing| {
                            !matches!(
                                existing.action,
                                MidiAction::SetParameter {
                                    parameter_id: existing_id
                                } if existing_id == parameter_id
                            )
                        });
                    }
                }
                mappings.push(mapping);
                self.midi_learn_plugin = None;
                self.midi_learn_parameter = None;
                if let Err(error) = self
                    .runtime
                    .handle()
                    .set_midi_mappings(self.midi_mapping_sets())
                {
                    self.error = Some(format!("Could not install MIDI mapping: {error}"));
                }
            }
        }
    }

    pub(super) fn show_midi_learn_modal(&mut self, ctx: &egui::Context) {
        if self.midi_learn_plugin.is_none() {
            return;
        }

        let mut cancel_requested = false;
        egui::Modal::new(egui::Id::new("midi-learn-modal")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.vertical_centered(|ui| {
                ui.heading("MIDI Learn");
                ui.add_space(8.0);
                ui.label("Waiting for control");
                ui.add_space(12.0);
                if ui.button("Cancel").clicked() {
                    cancel_requested = true;
                }
            });
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
        if cancel_requested {
            self.cancel_midi_learn();
        }
    }

    pub(super) fn start_midi_learn(&mut self, plugin_index: usize) {
        self.start_midi_learn_target(plugin_index, None);
    }

    pub(super) fn start_midi_parameter_learn(&mut self, plugin_index: usize, parameter_id: u32) {
        self.start_midi_learn_target(plugin_index, Some(parameter_id));
    }

    fn start_midi_learn_target(&mut self, plugin_index: usize, parameter_id: Option<u32>) {
        match self
            .runtime
            .handle()
            .start_midi_learn(plugin_index, parameter_id)
        {
            Ok(()) => {
                self.midi_learn_plugin = Some(plugin_index);
                self.midi_learn_parameter = parameter_id;
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Could not start MIDI learn: {error}")),
        }
    }

    pub(super) fn cancel_midi_learn(&mut self) {
        self.runtime.handle().cancel_midi_learn();
        self.midi_learn_plugin = None;
        self.midi_learn_parameter = None;
    }

    pub(super) fn sync_midi_mappings(&mut self) {
        if let Err(error) = self
            .runtime
            .handle()
            .set_midi_mappings(self.midi_mapping_sets())
        {
            self.error = Some(format!("Could not install MIDI mappings: {error}"));
        }
    }

    fn midi_mapping_sets(&self) -> Vec<Vec<PresetMidiMapping>> {
        self.rack
            .slots
            .iter()
            .map(|slot| slot.midi_mappings.clone())
            .collect()
    }
}
