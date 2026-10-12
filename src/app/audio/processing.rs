use super::channel_compat::{route_input_sample, route_output_sample};
use super::conversion::{f32_to_i32, i32_to_f32};
use super::metering::{LevelAtomics, update_channel_levels, update_levels};
use super::playback::{AudioClip, PlaybackState};
use super::tuner::{TunerProcessor, TunerState};
use crate::app::diagnostics::{
    DiagnosticEvent, DiagnosticKind, DiagnosticProducer, DiagnosticSeverity, FixedText,
    SUBJECT_BYTES,
};
use crate::app::diagnostics::{Stage, audio_callback, audio_plugin, audio_stage};
use crate::app::plugin::{AudioProcessError, RuntimePlugin};
use rtrb::{Consumer, Producer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use vst3_host::AudioBuffers;

pub fn push_input_samples(samples: impl Iterator<Item = f32>, producer: &mut Producer<f32>) {
    for sample in samples {
        if producer.push(sample).is_err() {
            break;
        }
    }
}

pub fn push_i32_input_samples(
    samples: &[i32],
    producer: &mut Producer<f32>,
    levels: &LevelAtomics,
) {
    let mut peak = 0.0_f32;
    let mut queue_full = false;
    for &sample in samples {
        let sample = i32_to_f32(sample);
        peak = peak.max(sample.abs());
        if !queue_full && producer.push(sample).is_err() {
            queue_full = true;
        }
    }
    levels.input_peak.store(peak.to_bits(), Ordering::Relaxed);
}

pub fn write_i32_output_samples(data: &mut [i32], consumer: &mut Consumer<f32>) {
    for target in data {
        *target = f32_to_i32(consumer.pop().unwrap_or(0.0));
    }
}

pub struct ProcessContext<'a> {
    pub input_channels: usize,
    pub output_channels: usize,
    pub input_left: usize,
    pub input_right: usize,
    pub output_left: usize,
    pub output_right: usize,
    pub input_consumer: &'a mut Consumer<f32>,
    pub plugins: &'a mut [RuntimePlugin],
    pub bypass_states: &'a [Arc<AtomicBool>],
    pub buffers: &'a mut AudioBuffers,
    pub levels: &'a LevelAtomics,
    pub sample_rate: f64,
    pub tuner: &'a mut TunerProcessor,
    pub tuner_state: &'a TunerState,
    pub diagnostics: &'a mut DiagnosticProducer,
    pub plugin_names: &'a [FixedText<SUBJECT_BYTES>],
}

pub fn process_output(
    data: &mut [f32],
    context: &mut ProcessContext<'_>,
    playback: Option<&PlaybackState>,
    clip: Option<&AudioClip>,
) {
    audio_stage(Stage::CallbackEnter);
    data.fill(0.0);
    let frames = data.len() / context.output_channels;
    for channel in context
        .buffers
        .inputs
        .iter_mut()
        .chain(context.buffers.outputs.iter_mut())
    {
        if channel.len() != frames {
            channel.resize(frames, 0.0);
        }
    }
    context.buffers.block_size = frames;
    audio_callback(frames);
    audio_stage(Stage::RoutingInput);
    let mut playback_position = playback
        .map(|state| state.position_frames() as f64)
        .unwrap_or_default();
    if let Some(seek_frame) = playback.and_then(PlaybackState::take_seek) {
        playback_position = seek_frame as f64;
    }
    let playback_ratio = clip
        .filter(|_| context.sample_rate.is_finite() && context.sample_rate > 0.0)
        .map(|clip| f64::from(clip.sample_rate) / context.sample_rate)
        .unwrap_or(1.0);
    if let (Some(playback), Some(clip)) = (playback, clip)
        && playback.is_playing()
        && playback_position >= clip.frames() as f64
    {
        playback.stop_at_end();
    }
    for frame in 0..frames {
        let mut left = 0.0;
        let mut right = 0.0;
        for channel in 0..context.input_channels {
            let sample = context.input_consumer.pop().unwrap_or(0.0);
            route_input_sample(
                channel,
                context.input_channels,
                sample,
                context.input_left,
                context.input_right,
                &mut left,
                &mut right,
            );
        }
        context.buffers.inputs[0][frame] = left;
        context.buffers.inputs[1][frame] = right;
    }
    context
        .tuner
        .process(&context.buffers.inputs, frames, context.tuner_state);

    if context.plugins.is_empty() {
        audio_stage(Stage::RoutingOutput);
        copy_inputs_to_outputs(context.buffers);
    } else {
        let plugin_count = context.plugins.len();
        for (index, plugin) in context.plugins.iter_mut().enumerate() {
            for output in &mut context.buffers.outputs {
                if output.len() != frames {
                    output.resize(frames, 0.0);
                }
                output.fill(0.0);
            }
            audio_plugin(index);
            audio_stage(Stage::PluginLock);
            let process_result = if context
                .bypass_states
                .get(index)
                .is_some_and(|bypass| bypass.load(Ordering::Relaxed))
            {
                copy_inputs_to_outputs(context.buffers);
                None
            } else {
                audio_stage(Stage::PluginProcess);
                Some(plugin.process_audio(context.buffers))
            };
            if let Some(result) = process_result {
                handle_plugin_result(
                    context.buffers,
                    index,
                    context.plugin_names.get(index),
                    context.diagnostics,
                    result,
                );
            }
            audio_stage(Stage::RoutingOutput);
            if index + 1 < plugin_count {
                copy_outputs_to_inputs(context.buffers);
            }
        }
    }
    for frame in 0..frames {
        let mut left = context.buffers.outputs[0][frame];
        let mut right = context.buffers.outputs[1][frame];
        if let (Some(playback), Some(clip)) = (playback, clip)
            && playback.is_playing()
            && playback_position < clip.frames() as f64
        {
            let volume = if playback.is_bypassed() {
                0.0
            } else {
                playback.volume()
            };
            left += clip_sample(clip, playback_position, 0) * volume;
            right += clip_sample(clip, playback_position, 1) * volume;
            playback_position += playback_ratio;
            if playback_position >= clip.frames() as f64 {
                playback_position = clip.frames() as f64;
                playback.stop_at_end();
            }
        }
        route_output_sample(
            &mut data[frame * context.output_channels..(frame + 1) * context.output_channels],
            context.output_left,
            context.output_right,
            left,
            right,
        );
    }
    if let Some(playback) = playback {
        playback.set_position(playback_position as u64);
    }
    audio_stage(Stage::CallbackExit);
    update_channel_levels(
        &context.buffers.outputs,
        frames,
        &context.levels.output_peak,
    );
}

fn handle_plugin_result(
    buffers: &mut AudioBuffers,
    plugin_index: usize,
    plugin_name: Option<&FixedText<SUBJECT_BYTES>>,
    diagnostics: &mut DiagnosticProducer,
    result: Result<(), AudioProcessError>,
) -> bool {
    let Err(error) = result else {
        return true;
    };

    copy_inputs_to_outputs(buffers);
    diagnostics.send(DiagnosticEvent::new(
        DiagnosticSeverity::Error,
        DiagnosticKind::PluginProcessing,
        plugin_name.map_or("Plug-in", FixedText::as_str),
        error.diagnostic_message(),
    ));
    audio_plugin(plugin_index);
    false
}

fn clip_sample(clip: &AudioClip, position: f64, channel: usize) -> f32 {
    let frames = clip.frames() as usize;
    let frame = (position.floor() as usize).min(frames.saturating_sub(1));
    let next_frame = (frame + 1).min(frames.saturating_sub(1));
    let channel = channel.min(clip.channels.saturating_sub(1));
    let first = clip.samples[frame * clip.channels + channel];
    let second = clip.samples[next_frame * clip.channels + channel];
    let fraction = position.fract() as f32;
    first + (second - first) * fraction
}

fn copy_outputs_to_inputs(buffers: &mut AudioBuffers) {
    for (input, output) in buffers.inputs.iter_mut().zip(buffers.outputs.iter()) {
        input.copy_from_slice(output);
    }
}

fn copy_inputs_to_outputs(buffers: &mut AudioBuffers) {
    for (output, input) in buffers.outputs.iter_mut().zip(buffers.inputs.iter()) {
        output.copy_from_slice(input);
    }
}

pub fn update_input_levels(samples: &[f32], levels: &LevelAtomics) {
    update_levels(samples, &levels.input_peak);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hands_each_plugin_output_to_the_next_plugin_input() {
        let mut buffers = AudioBuffers::new(2, 2, 2, 48_000.0);
        buffers.outputs[0].copy_from_slice(&[0.25, 0.5]);
        buffers.outputs[1].copy_from_slice(&[-0.25, -0.5]);

        copy_outputs_to_inputs(&mut buffers);

        assert_eq!(buffers.inputs[0], [0.25, 0.5]);
        assert_eq!(buffers.inputs[1], [-0.25, -0.5]);
    }

    #[test]
    fn hands_each_input_to_the_matching_bypass_output() {
        let mut buffers = AudioBuffers::new(2, 2, 2, 48_000.0);
        buffers.inputs[0].copy_from_slice(&[0.25, 0.5]);
        buffers.inputs[1].copy_from_slice(&[-0.25, -0.5]);

        copy_inputs_to_outputs(&mut buffers);

        assert_eq!(buffers.outputs[0], [0.25, 0.5]);
        assert_eq!(buffers.outputs[1], [-0.25, -0.5]);
    }

    #[test]
    fn process_output_bypasses_empty_plugin_stack() {
        let (mut input_producer, mut input_consumer) = rtrb::RingBuffer::new(8);
        for sample in [0.25, -0.25, 0.5, -0.5] {
            input_producer.push(sample).unwrap();
        }
        let mut buffers = AudioBuffers::new(2, 2, 2, 48_000.0);
        let mut plugins = Vec::new();
        let levels = LevelAtomics::new();
        let (mut diagnostics, _diagnostic_consumer) = crate::app::diagnostics::channel(4);
        let mut context = ProcessContext {
            input_channels: 2,
            output_channels: 2,
            input_left: 0,
            input_right: 1,
            output_left: 0,
            output_right: 1,
            input_consumer: &mut input_consumer,
            plugins: &mut plugins,
            bypass_states: &[],
            buffers: &mut buffers,
            levels: &levels,
            sample_rate: 48_000.0,
            tuner: &mut TunerProcessor::new(48_000.0),
            tuner_state: &TunerState::new(),
            diagnostics: &mut diagnostics,
            plugin_names: &[],
        };
        let mut data = [0.0; 4];

        process_output(&mut data, &mut context, None, None);

        assert_eq!(data, [0.25, -0.25, 0.5, -0.5]);
    }

    #[test]
    fn processing_error_is_reported_and_preserves_the_dry_signal() {
        let mut buffers = AudioBuffers::new(2, 2, 2, 48_000.0);
        buffers.inputs[0].copy_from_slice(&[0.25, 0.5]);
        buffers.inputs[1].copy_from_slice(&[-0.25, -0.5]);
        buffers.outputs[0].fill(0.0);
        buffers.outputs[1].fill(0.0);
        let name = FixedText::<SUBJECT_BYTES>::new("Compressor");
        let (mut producer, mut consumer) = crate::app::diagnostics::channel(4);

        let processed = handle_plugin_result(
            &mut buffers,
            0,
            Some(&name),
            &mut producer,
            Err(AudioProcessError::PluginFailure),
        );

        assert!(!processed);
        assert_eq!(buffers.outputs[0], [0.25, 0.5]);
        assert_eq!(buffers.outputs[1], [-0.25, -0.5]);
        let event = consumer.try_recv().unwrap();
        assert_eq!(event.kind, DiagnosticKind::PluginProcessing);
        assert_eq!(event.subject.as_str(), "Compressor");
        assert!(event.message().contains("dry signal"));
    }

    #[test]
    fn i32_input_conversion_preserves_levels_when_the_queue_is_full() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(1);
        let levels = LevelAtomics::new();

        push_i32_input_samples(&[i32::MIN, i32::MAX], &mut producer, &levels);

        assert_eq!(consumer.pop().unwrap(), -1.0);
        assert_eq!(consumer.pop(), Err(rtrb::PopError::Empty));
        assert_eq!(
            f32::from_bits(levels.input_peak.load(Ordering::Relaxed)),
            1.0
        );
    }

    #[test]
    fn i32_output_conversion_drains_available_samples_and_zero_fills() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(2);
        producer.push(-1.0).unwrap();
        producer.push(1.0).unwrap();
        let mut output = [0; 3];

        write_i32_output_samples(&mut output, &mut consumer);

        assert_eq!(output, [i32::MIN, i32::MAX, 0]);
    }
}
