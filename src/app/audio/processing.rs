use super::channel_compat::{route_input_sample, route_output_sample};
use super::metering::{LevelAtomics, update_channel_levels, update_levels};
use super::playback::{AudioClip, PlaybackState};
use super::tuner::{TunerProcessor, TunerState};
use crate::app::diagnostics::{Stage, audio_callback, audio_plugin, audio_stage};
use crate::app::plugin::RuntimePlugin;
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
            let processed = if context
                .bypass_states
                .get(index)
                .is_some_and(|bypass| bypass.load(Ordering::Relaxed))
            {
                copy_inputs_to_outputs(context.buffers);
                true
            } else {
                audio_stage(Stage::PluginProcess);
                plugin.process_audio(context.buffers).is_ok()
            };
            audio_stage(Stage::RoutingOutput);
            if !processed {
                return;
            }
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
        };
        let mut data = [0.0; 4];

        process_output(&mut data, &mut context, None, None);

        assert_eq!(data, [0.25, -0.25, 0.5, -0.5]);
    }
}
