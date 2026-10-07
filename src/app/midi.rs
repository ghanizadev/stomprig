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

    pub(crate) fn label(&self) -> String {
        match self.trigger {
            MidiTrigger::ControlChange {
                channel,
                controller,
                ..
            } => format!("CC {controller} · Channel {channel}"),
            MidiTrigger::NoteOn { channel, note } => {
                format!("Note {note} · Channel {channel}")
            }
        }
    }
}

pub(crate) fn validate_midi_mapping(mapping: &PresetMidiMapping) -> Result<(), String> {
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
