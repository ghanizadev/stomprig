use super::metering::{LevelAtomics, update_channel_levels, update_levels};
use rtrb::{Consumer, Producer};
use std::sync::{Arc, Mutex};
use vst3_host::{AudioBuffers, Plugin};

pub const CHANNELS: usize = 2;
pub const BLOCK_SIZE: usize = 512;

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
    pub output_producer: &'a mut Producer<f32>,
    pub plugin: &'a Arc<Mutex<Plugin>>,
    pub buffers: &'a mut AudioBuffers,
    pub levels: &'a LevelAtomics,
}

pub fn process_output(data: &mut [f32], context: &mut ProcessContext<'_>) {
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
    for frame in 0..frames {
        let mut left = 0.0;
        let mut right = 0.0;
        for channel in 0..context.input_channels {
            let sample = context.input_consumer.pop().unwrap_or(0.0);
            route_input_sample(
                channel,
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
    if context
        .plugin
        .lock()
        .map(|mut plugin| plugin.process_audio(context.buffers).is_err())
        .unwrap_or(true)
    {
        return;
    }
    for frame in 0..frames {
        let left = context.buffers.outputs[0][frame];
        let right = context.buffers.outputs[1][frame];
        route_output_sample(
            &mut data[frame * context.output_channels..(frame + 1) * context.output_channels],
            context.output_left,
            context.output_right,
            left,
            right,
        );
        let _ = context.output_producer.push(left);
        let _ = context.output_producer.push(right);
    }
    update_channel_levels(
        &context.buffers.outputs,
        frames,
        &context.levels.output_peak,
        &context.levels.output_rms,
    );
}

pub fn route_input_sample(
    channel: usize,
    sample: f32,
    input_left: usize,
    input_right: usize,
    left: &mut f32,
    right: &mut f32,
) {
    if channel == input_left {
        *left = sample;
    }
    if channel == input_right {
        *right = sample;
    }
}
pub fn route_output_sample(
    frame: &mut [f32],
    output_left: usize,
    output_right: usize,
    left: f32,
    right: f32,
) {
    frame[output_left] = left;
    frame[output_right] = right;
}

pub fn update_input_levels(samples: &[f32], levels: &LevelAtomics) {
    update_levels(samples, &levels.input_peak, &levels.input_rms);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_input_channels() {
        let mut left = 0.0;
        let mut right = 0.0;
        for (channel, sample) in [0.1, 0.2, 0.3, 0.4].into_iter().enumerate() {
            route_input_sample(channel, sample, 2, 0, &mut left, &mut right);
        }
        assert_eq!(left, 0.3);
        assert_eq!(right, 0.1);
    }
    #[test]
    fn routes_output_channels() {
        let mut frame = [0.0; 4];
        route_output_sample(&mut frame, 2, 0, 0.25, -0.5);
        assert_eq!(frame, [-0.5, 0.0, 0.25, 0.0]);
    }

    #[test]
    fn allows_one_input_channel_for_both_plugin_inputs() {
        let mut left = 0.0;
        let mut right = 0.0;
        route_input_sample(1, 0.75, 1, 1, &mut left, &mut right);
        assert_eq!(left, 0.75);
        assert_eq!(right, 0.75);
    }
}
