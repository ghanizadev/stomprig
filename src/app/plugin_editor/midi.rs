use super::super::midi::PresetMidiMapping;
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
            let Some(mapping) = PresetMidiMapping::from_event(&event) else {
                continue;
            };
            if let Some(mappings) = self
                .rack
                .slots
                .get_mut(plugin_index)
                .map(|slot| &mut slot.midi_mappings)
            {
                mappings.clear();
                mappings.push(mapping);
                self.midi_learn_plugin = None;
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

    pub(super) fn start_midi_learn(&mut self, plugin_index: usize) {
        match self.runtime.handle().start_midi_learn(plugin_index) {
            Ok(()) => {
                self.midi_learn_plugin = Some(plugin_index);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Could not start MIDI learn: {error}")),
        }
    }

    pub(super) fn cancel_midi_learn(&mut self) {
        self.runtime.handle().cancel_midi_learn();
        self.midi_learn_plugin = None;
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
