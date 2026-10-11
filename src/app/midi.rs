use serde::{Deserialize, Serialize};
use vst3_host::MidiEvent;

pub(crate) const DEFAULT_CC_THRESHOLD: u8 = 64;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PresetMidiMapping {
    #[serde(flatten)]
    pub(crate) trigger: MidiTrigger,
    #[serde(default)]
    pub(crate) control_mode: MidiControlMode,
    pub(crate) action: MidiAction,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MidiControlMode {
    #[default]
    PressAndRelease,
    Latching,
    Value,
}

impl MidiControlMode {
    pub(crate) const ALL: [Self; 3] = [Self::PressAndRelease, Self::Latching, Self::Value];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::PressAndRelease => "Press-and-release",
            Self::Latching => "Latching",
            Self::Value => "Value",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "press_and_release" => Some(Self::PressAndRelease),
            "latching" => Some(Self::Latching),
            "value" => Some(Self::Value),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::PressAndRelease => "press_and_release",
            Self::Latching => "latching",
            Self::Value => "value",
        }
    }
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
    #[serde(alias = "set_clap_parameter")]
    SetParameter {
        parameter_id: u32,
    },
}

fn default_cc_threshold() -> u8 {
    DEFAULT_CC_THRESHOLD
}

impl PresetMidiMapping {
    pub(crate) fn from_event(event: &MidiEvent, control_mode: MidiControlMode) -> Option<Self> {
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
            control_mode,
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
            control_mode: MidiControlMode::default(),
            action: MidiAction::SetParameter { parameter_id },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BypassControlAction {
    None,
    Toggle,
    Set(bool),
}

pub(crate) fn control_change_action(
    mode: MidiControlMode,
    value: u8,
    threshold: u8,
    was_active: Option<bool>,
) -> (bool, BypassControlAction) {
    let active = value >= threshold;
    let action = match mode {
        MidiControlMode::PressAndRelease if active && was_active != Some(true) => {
            BypassControlAction::Toggle
        }
        MidiControlMode::Latching if was_active != Some(active) => BypassControlAction::Toggle,
        MidiControlMode::Value => BypassControlAction::Set(active),
        MidiControlMode::PressAndRelease | MidiControlMode::Latching => BypassControlAction::None,
    };
    (active, action)
}

#[cfg(test)]
mod control_mode_tests {
    use super::{BypassControlAction, MidiControlMode, control_change_action};

    #[test]
    fn press_and_release_only_toggles_on_the_high_edge() {
        assert_eq!(
            control_change_action(MidiControlMode::PressAndRelease, 127, 64, Some(false)),
            (true, BypassControlAction::Toggle)
        );
        assert_eq!(
            control_change_action(MidiControlMode::PressAndRelease, 0, 64, Some(true)),
            (false, BypassControlAction::None)
        );
    }

    #[test]
    fn latching_toggles_on_each_binary_value_change() {
        assert_eq!(
            control_change_action(MidiControlMode::Latching, 127, 64, Some(true)),
            (true, BypassControlAction::None)
        );
        assert_eq!(
            control_change_action(MidiControlMode::Latching, 0, 64, Some(true)),
            (false, BypassControlAction::Toggle)
        );
        assert_eq!(
            control_change_action(MidiControlMode::Latching, 0, 64, None),
            (false, BypassControlAction::Toggle)
        );
    }

    #[test]
    fn value_mode_sets_bypass_from_the_threshold() {
        assert_eq!(
            control_change_action(MidiControlMode::Value, 63, 64, Some(true)),
            (false, BypassControlAction::Set(false))
        );
        assert_eq!(
            control_change_action(MidiControlMode::Value, 64, 64, Some(false)),
            (true, BypassControlAction::Set(true))
        );
    }

    #[test]
    fn legacy_midi_mapping_defaults_to_press_and_release() {
        let mut mapping: super::PresetMidiMapping = toml::from_str(
            r#"
event = "control_change"
channel = 1
controller = 12
threshold = 64
action = "toggle_bypass"
"#,
        )
        .unwrap();

        assert_eq!(mapping.control_mode, MidiControlMode::PressAndRelease);

        mapping.control_mode = MidiControlMode::Latching;
        let serialized = toml::to_string(&mapping).unwrap();
        let restored: super::PresetMidiMapping = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.control_mode, MidiControlMode::Latching);
    }
}

pub(crate) fn validate_midi_mapping(mapping: &PresetMidiMapping) -> Result<(), String> {
    if matches!(mapping.action, MidiAction::SetParameter { .. })
        && !matches!(mapping.trigger, MidiTrigger::ControlChange { .. })
    {
        return Err("parameter MIDI mappings require a control change".to_owned());
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
            MidiAction::SetParameter { parameter_id: 42 }
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
            control_mode: super::MidiControlMode::default(),
            action: MidiAction::SetParameter { parameter_id: 123 },
        };

        let serialized = toml::to_string(&mapping).expect("mapping should serialize");
        let restored: PresetMidiMapping =
            toml::from_str(&serialized).expect("mapping should deserialize");

        assert_eq!(restored, mapping);

        let legacy_serialized = serialized.replace("set_parameter", "set_clap_parameter");
        let restored_legacy: PresetMidiMapping =
            toml::from_str(&legacy_serialized).expect("legacy CLAP mappings should deserialize");
        assert_eq!(restored_legacy, mapping);
    }
}
