use serde::{Deserialize, Serialize};
use vst3_host::MidiEvent;

pub(crate) const DEFAULT_CC_THRESHOLD: u8 = 64;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PresetMidiMapping {
    #[serde(flatten)]
    pub(crate) trigger: MidiTrigger,
    pub(crate) action: MidiAction,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum MidiTrigger {
    ControlChange {
        channel: u8,
        controller: u8,
        #[serde(default = "default_cc_threshold")]
        threshold: u8,
    },
    NoteOn {
        channel: u8,
        note: u8,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MidiAction {
    ToggleBypass,
    SetClapParameter { parameter_id: u32 },
}

fn default_cc_threshold() -> u8 {
    DEFAULT_CC_THRESHOLD
}

impl PresetMidiMapping {
    pub(crate) fn from_event(event: &MidiEvent) -> Option<Self> {
        let trigger = match event {
            MidiEvent::ControlChange {
                channel,
                controller,
                ..
            } => MidiTrigger::ControlChange {
                channel: channel.as_index() + 1,
                controller: *controller,
                threshold: DEFAULT_CC_THRESHOLD,
            },
            MidiEvent::NoteOn {
                channel,
                note,
                velocity,
            } if *velocity > 0 => MidiTrigger::NoteOn {
                channel: channel.as_index() + 1,
                note: *note,
            },
            _ => return None,
        };
        Some(Self {
            trigger,
            action: MidiAction::ToggleBypass,
        })
    }

    pub(crate) fn from_parameter_event(event: &MidiEvent, parameter_id: u32) -> Option<Self> {
        let MidiEvent::ControlChange {
            channel,
            controller,
            ..
        } = event
        else {
            return None;
        };
        Some(Self {
            trigger: MidiTrigger::ControlChange {
                channel: channel.as_index() + 1,
                controller: *controller,
                threshold: DEFAULT_CC_THRESHOLD,
            },
            action: MidiAction::SetClapParameter { parameter_id },
        })
    }

    pub(crate) fn label(&self) -> String {
        match self.trigger {
            MidiTrigger::ControlChange {
                channel,
                controller,
                ..
            } => format!("CC {controller} - Channel {channel}"),
            MidiTrigger::NoteOn { channel, note } => {
                format!("Note {note} - Channel {channel}")
            }
        }
    }
}

pub(crate) fn validate_midi_mapping(mapping: &PresetMidiMapping) -> Result<(), String> {
    if matches!(mapping.action, MidiAction::SetClapParameter { .. })
        && !matches!(mapping.trigger, MidiTrigger::ControlChange { .. })
    {
        return Err("CLAP parameter mappings require a MIDI control change".to_owned());
    }
    match mapping.trigger {
        MidiTrigger::ControlChange {
            channel,
            controller,
            threshold,
        } => {
            if !(1..=16).contains(&channel) {
                return Err(format!(
                    "MIDI channel must be between 1 and 16, got {channel}"
                ));
            }
            if controller > 127 {
                return Err(format!(
                    "MIDI controller must be between 0 and 127, got {controller}"
                ));
            }
            if threshold > 127 {
                return Err(format!(
                    "MIDI threshold must be between 0 and 127, got {threshold}"
                ));
            }
        }
        MidiTrigger::NoteOn { channel, note } => {
            if !(1..=16).contains(&channel) {
                return Err(format!(
                    "MIDI channel must be between 1 and 16, got {channel}"
                ));
            }
            if note > 127 {
                return Err(format!("MIDI note must be between 0 and 127, got {note}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MidiAction, MidiTrigger, PresetMidiMapping, validate_midi_mapping};
    use vst3_host::{MidiChannel, MidiEvent};

    #[test]
    fn parameter_learn_records_control_change_and_parameter_id() {
        let event = MidiEvent::ControlChange {
            channel: MidiChannel::Ch3,
            controller: 74,
            value: 96,
        };

        let mapping = PresetMidiMapping::from_parameter_event(&event, 42)
            .expect("parameter MIDI learn should capture control changes");

        assert_eq!(
            mapping.trigger,
            MidiTrigger::ControlChange {
                channel: 3,
                controller: 74,
                threshold: 64,
            }
        );
        assert_eq!(
            mapping.action,
            MidiAction::SetClapParameter { parameter_id: 42 }
        );
        validate_midi_mapping(&mapping).expect("learned mapping should be valid");
    }

    #[test]
    fn parameter_learn_ignores_note_events() {
        let event = MidiEvent::NoteOn {
            channel: MidiChannel::Ch1,
            note: 60,
            velocity: 100,
        };

        assert!(PresetMidiMapping::from_parameter_event(&event, 42).is_none());
    }

    #[test]
    fn midi_parameter_action_round_trips_through_serde() {
        let mapping = PresetMidiMapping {
            trigger: MidiTrigger::ControlChange {
                channel: 1,
                controller: 7,
                threshold: 64,
            },
            action: MidiAction::SetClapParameter { parameter_id: 123 },
        };

        let serialized = toml::to_string(&mapping).expect("mapping should serialize");
        let restored: PresetMidiMapping =
            toml::from_str(&serialized).expect("mapping should deserialize");

        assert_eq!(restored, mapping);
    }
}
