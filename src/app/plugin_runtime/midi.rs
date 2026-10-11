use super::super::midi::{
    BypassControlAction, DEFAULT_CC_THRESHOLD, MidiAction, MidiControlMode, MidiTrigger,
    PresetMidiMapping, control_change_action,
};
use super::super::plugin::RuntimePlugin;
use super::MidiLearnTarget;
use rtrb::{Consumer, Producer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use vst3_host::MidiEvent;

pub(super) struct MidiRuntimeState<'a> {
    pub(super) learn_target: &'a mut Option<MidiLearnTarget>,
    pub(super) mappings: &'a mut [Vec<PresetMidiMapping>],
    pub(super) mapping_active: &'a mut [Vec<Option<bool>>],
    pub(super) cc_values: &'a mut [Option<u8>],
    pub(super) bypass_states: &'a [Arc<AtomicBool>],
    pub(super) plugins: &'a mut [RuntimePlugin],
}

pub(super) fn drain_midi_events(
    consumer: &mut Option<Consumer<MidiEvent>>,
    feedback: &mut Producer<MidiEvent>,
    state: MidiRuntimeState<'_>,
) {
    let MidiRuntimeState {
        learn_target,
        mappings,
        mapping_active,
        cc_values,
        bypass_states,
        plugins,
    } = state;
    let Some(consumer) = consumer.as_mut() else {
        return;
    };
    let budget = consumer.buffer().capacity();
    for _ in 0..budget {
        let Ok(event) = consumer.pop() else {
            break;
        };
        if let MidiEvent::ControlChange {
            channel,
            controller,
            value,
        } = event
            && let Some(last_value) =
                cc_values.get_mut(channel.as_index() as usize * 128 + controller as usize)
        {
            *last_value = Some(value);
        }
        if matches!(event, MidiEvent::ProgramChange { .. }) {
            if feedback.push(event).is_err() {
                log::warn!("MIDI feedback buffer is full; dropping program change");
            }
            continue;
        }
        if let Some(target) = learn_target.take() {
            let mapping = match target.parameter_id {
                Some(parameter_id) => PresetMidiMapping::from_parameter_event(&event, parameter_id),
                None => PresetMidiMapping::from_event(&event, MidiControlMode::default()),
            };
            if mapping.is_some() {
                if feedback.push(event).is_err() {
                    *learn_target = Some(target);
                }
            } else {
                *learn_target = Some(target);
            }
            continue;
        }
        for (plugin_index, plugin_mappings) in mappings.iter().enumerate() {
            let Some(bypass) = bypass_states.get(plugin_index) else {
                continue;
            };
            for (mapping_index, mapping) in plugin_mappings.iter().enumerate() {
                let was_active = mapping_active
                    .get(plugin_index)
                    .and_then(|states| states.get(mapping_index))
                    .copied()
                    .flatten();
                let action = match mapping.trigger {
                    MidiTrigger::ControlChange {
                        channel,
                        controller,
                        threshold,
                    } => {
                        if let MidiEvent::ControlChange {
                            channel: event_channel,
                            controller: event_controller,
                            value,
                        } = event
                            && event_channel.as_index() + 1 == channel
                            && event_controller == controller
                        {
                            let (active, action) = control_change_action(
                                mapping.control_mode,
                                value,
                                threshold,
                                was_active,
                            );
                            if let Some(state) = mapping_active
                                .get_mut(plugin_index)
                                .and_then(|states| states.get_mut(mapping_index))
                            {
                                *state = Some(active);
                            }
                            action
                        } else {
                            BypassControlAction::None
                        }
                    }
                    MidiTrigger::NoteOn { channel, note } => match event {
                        MidiEvent::NoteOn {
                            channel: event_channel,
                            note: event_note,
                            velocity,
                        } if event_channel.as_index() + 1 == channel
                            && event_note == note
                            && velocity > 0 =>
                        {
                            if let Some(state) = mapping_active
                                .get_mut(plugin_index)
                                .and_then(|states| states.get_mut(mapping_index))
                            {
                                *state = Some(true);
                            }
                            if mapping.control_mode == MidiControlMode::Value {
                                BypassControlAction::Set(velocity >= DEFAULT_CC_THRESHOLD)
                            } else if was_active != Some(true) {
                                BypassControlAction::Toggle
                            } else {
                                BypassControlAction::None
                            }
                        }
                        MidiEvent::NoteOff {
                            channel: event_channel,
                            note: event_note,
                            ..
                        } if event_channel.as_index() + 1 == channel && event_note == note => {
                            if let Some(state) = mapping_active
                                .get_mut(plugin_index)
                                .and_then(|states| states.get_mut(mapping_index))
                            {
                                *state = Some(false);
                            }
                            if mapping.control_mode == MidiControlMode::Value {
                                BypassControlAction::Set(false)
                            } else {
                                BypassControlAction::None
                            }
                        }
                        _ => BypassControlAction::None,
                    },
                };
                match mapping.action {
                    MidiAction::ToggleBypass => match action {
                        BypassControlAction::None => {}
                        BypassControlAction::Toggle => {
                            bypass.fetch_xor(true, Ordering::AcqRel);
                        }
                        BypassControlAction::Set(bypassed) => {
                            bypass.store(bypassed, Ordering::Release);
                        }
                    },
                    MidiAction::SetClapParameter { parameter_id } => {
                        if let Some(value) = matching_control_change_value(&event, &mapping.trigger)
                            && let Some(plugin) = plugins.get_mut(plugin_index)
                        {
                            plugin.set_clap_parameter_from_midi(parameter_id, value);
                        }
                    }
                }
            }
        }
    }
}

fn matching_control_change_value(event: &MidiEvent, trigger: &MidiTrigger) -> Option<u8> {
    let MidiTrigger::ControlChange {
        channel,
        controller,
        ..
    } = trigger
    else {
        return None;
    };
    match event {
        MidiEvent::ControlChange {
            channel: event_channel,
            controller: event_controller,
            value,
        } if event_channel.as_index() + 1 == *channel && event_controller == controller => {
            Some(*value)
        }
        _ => None,
    }
}

pub(super) fn mapping_active_states(
    mappings: &[Vec<PresetMidiMapping>],
    cc_values: &[Option<u8>],
) -> Vec<Vec<Option<bool>>> {
    mappings
        .iter()
        .map(|plugin_mappings| {
            plugin_mappings
                .iter()
                .map(|mapping| match mapping.trigger {
                    MidiTrigger::ControlChange {
                        channel,
                        controller,
                        threshold,
                    } => cc_values
                        .get((channel as usize - 1) * 128 + controller as usize)
                        .copied()
                        .flatten()
                        .map(|value| value >= threshold),
                    MidiTrigger::NoteOn { .. } => None,
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::mapping_active_states;
    use crate::app::midi::{
        BypassControlAction, MidiAction, MidiControlMode, MidiTrigger, PresetMidiMapping,
        control_change_action,
    };

    #[test]
    fn learned_latching_mapping_starts_from_the_learned_cc_value() {
        let mapping = PresetMidiMapping {
            trigger: MidiTrigger::ControlChange {
                channel: 1,
                controller: 12,
                threshold: 64,
            },
            control_mode: MidiControlMode::Latching,
            action: MidiAction::ToggleBypass,
        };
        let mut cc_values = vec![None; 16 * 128];
        cc_values[12] = Some(127);

        let states = mapping_active_states(&[vec![mapping]], &cc_values);
        assert_eq!(states, vec![vec![Some(true)]]);
        assert_eq!(
            control_change_action(MidiControlMode::Latching, 0, 64, states[0][0]),
            (false, BypassControlAction::Toggle)
        );
    }
}
