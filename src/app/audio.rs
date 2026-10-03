use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use vst3_host::{AudioBuffers, Plugin};

const CHANNELS: usize = 2;
const BLOCK_SIZE: usize = 512;
const TAP_CAPACITY: usize = 16_384;

#[derive(Clone, Copy, Default)]
pub struct SignalLevels {
    pub input_peak: f32,
    pub input_rms: f32,
    pub output_peak: f32,
    pub output_rms: f32,
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub index: usize,
    pub name: String,
    pub channels: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioBackend {
    Wasapi,
    Asio,
}

impl AudioBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wasapi => "WASAPI",
            Self::Asio => "ASIO",
        }
    }
}

struct LevelAtomics {
    input_peak: AtomicU32,
    input_rms: AtomicU32,
    output_peak: AtomicU32,
    output_rms: AtomicU32,
}

impl LevelAtomics {
    fn new() -> Self {
        Self {
            input_peak: AtomicU32::new(0),
            input_rms: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
            output_rms: AtomicU32::new(0),
        }
    }

    fn read(&self) -> SignalLevels {
        SignalLevels {
            input_peak: f32::from_bits(self.input_peak.load(Ordering::Relaxed)),
            input_rms: f32::from_bits(self.input_rms.load(Ordering::Relaxed)),
            output_peak: f32::from_bits(self.output_peak.load(Ordering::Relaxed)),
            output_rms: f32::from_bits(self.output_rms.load(Ordering::Relaxed)),
        }
    }
}

pub struct AudioEngine {
    _input_stream: Stream,
    _output_stream: Stream,
    plugin: Arc<Mutex<Plugin>>,
    output_tap: Consumer<f32>,
    levels: Arc<LevelAtomics>,
    error: Arc<Mutex<Option<String>>>,
}

impl AudioEngine {
    pub fn available_backends() -> Vec<AudioBackend> {
        let mut backends = vec![AudioBackend::Wasapi];
        if cpal::available_hosts().contains(&cpal::HostId::Asio) {
            backends.push(AudioBackend::Asio);
        }
        backends
    }

    fn host(backend: AudioBackend) -> Result<cpal::Host, String> {
        let host_id = match backend {
            AudioBackend::Wasapi => cpal::HostId::Wasapi,
            AudioBackend::Asio => cpal::HostId::Asio,
        };
        cpal::host_from_id(host_id)
            .map_err(|e| format!("Could not open {} host: {e}", backend.label()))
    }

    pub fn input_devices(backend: AudioBackend) -> Result<Vec<DeviceInfo>, String> {
        let host = Self::host(backend)?;
        host.input_devices()
            .map_err(|e| format!("Could not enumerate input devices: {e}"))?
            .enumerate()
            .map(|(index, device)| {
                let name = device.to_string();
                let channels = device
                    .default_input_config()
                    .map_err(|e| format!("Could not read input configuration for {name}: {e}"))?
                    .channels();
                Ok(DeviceInfo {
                    index,
                    name,
                    channels: usize::from(channels),
                })
            })
            .collect()
    }

    pub fn output_devices(backend: AudioBackend) -> Result<Vec<DeviceInfo>, String> {
        let host = Self::host(backend)?;
        host.output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .enumerate()
            .map(|(index, device)| {
                let name = device.to_string();
                let channels = device
                    .default_output_config()
                    .map_err(|e| format!("Could not read output configuration for {name}: {e}"))?
                    .channels();
                Ok(DeviceInfo {
                    index,
                    name,
                    channels: usize::from(channels),
                })
            })
            .collect()
    }

    pub fn default_device_indices(backend: AudioBackend) -> (Option<usize>, Option<usize>) {
        let Ok(host) = Self::host(backend) else {
            return (None, None);
        };
        let input_name = host.default_input_device().map(|device| device.to_string());
        let output_name = host
            .default_output_device()
            .map(|device| device.to_string());
        let input_index = input_name.and_then(|name| {
            host.input_devices()
                .ok()?
                .enumerate()
                .find_map(|(index, device)| (device.to_string() == name).then_some(index))
        });
        let output_index = output_name.and_then(|name| {
            host.output_devices()
                .ok()?
                .enumerate()
                .find_map(|(index, device)| (device.to_string() == name).then_some(index))
        });
        (input_index, output_index)
    }

    pub fn sample_rate(backend: AudioBackend, output_index: usize) -> Result<f64, String> {
        let host = Self::host(backend)?;
        let output_device = host
            .output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .nth(output_index)
            .ok_or_else(|| format!("Output device {output_index} is unavailable"))?;
        let output_supported = output_device
            .default_output_config()
            .map_err(|e| format!("Could not read output configuration: {e}"))?;
        Ok(output_supported.sample_rate() as f64)
    }

    pub fn start(
        plugin: Arc<Mutex<Plugin>>,
        backend: AudioBackend,
        input_index: usize,
        output_index: usize,
        input_left: usize,
        input_right: usize,
        output_left: usize,
        output_right: usize,
    ) -> Result<Self, String> {
        let host = Self::host(backend)?;
        let input_device = host
            .input_devices()
            .map_err(|e| format!("Could not enumerate input devices: {e}"))?
            .nth(input_index)
            .ok_or_else(|| format!("Input device {input_index} is unavailable"))?;
        let output_device = host
            .output_devices()
            .map_err(|e| format!("Could not enumerate output devices: {e}"))?
            .nth(output_index)
            .ok_or_else(|| format!("Output device {output_index} is unavailable"))?;
        if backend == AudioBackend::Asio {
            let input_id = input_device
                .id()
                .map_err(|e| format!("Could not identify ASIO input driver: {e}"))?;
            let output_id = output_device
                .id()
                .map_err(|e| format!("Could not identify ASIO output driver: {e}"))?;
            if input_id != output_id {
                return Err("ASIO input and output must use the same driver".to_string());
            }
        }
        let input_supported = input_device
            .default_input_config()
            .map_err(|e| format!("Could not read input configuration: {e}"))?;
        let output_supported = output_device
            .default_output_config()
            .map_err(|e| format!("Could not read output configuration: {e}"))?;

        if !is_supported_sample_format(input_supported.sample_format())
            || !is_supported_sample_format(output_supported.sample_format())
        {
            return Err(format!(
                "Selected devices must use f32 or i32 audio (input: {:?}, output: {:?})",
                input_supported.sample_format(),
                output_supported.sample_format()
            ));
        }
        let input_channels = usize::from(input_supported.channels());
        let output_channels = usize::from(output_supported.channels());
        if input_channels == 0 || output_channels == 0 {
            return Err(format!(
                "Selected devices must provide at least one channel (input: {} channels, output: {} channels)",
                input_supported.channels(),
                output_supported.channels()
            ));
        }
        if input_left >= input_channels || input_right >= input_channels {
            return Err(format!(
                "Selected input channel is unavailable (device has {input_channels} channels)"
            ));
        }
        if output_left >= output_channels || output_right >= output_channels {
            return Err(format!(
                "Selected output channel is unavailable (device has {output_channels} channels)"
            ));
        }

        if input_supported.sample_rate() != output_supported.sample_rate() {
            return Err(format!(
                "Selected devices use different sample rates (input {} Hz, output {} Hz)",
                input_supported.sample_rate(),
                output_supported.sample_rate()
            ));
        }

        let sample_rate = output_supported.sample_rate();
        let input_stream_config = input_supported.config();
        let output_stream_config = output_supported.config();

        let (mut input_producer, mut input_consumer) = RingBuffer::new(TAP_CAPACITY);
        let (mut output_producer, output_tap) = RingBuffer::new(TAP_CAPACITY);
        let levels = Arc::new(LevelAtomics::new());
        let error = Arc::new(Mutex::new(None));

        let input_levels = Arc::clone(&levels);
        let input_error = Arc::clone(&error);
        let input_stream = match input_supported.sample_format() {
            SampleFormat::F32 => input_device.build_input_stream(
                input_stream_config,
                move |data: &[f32], _| {
                    update_levels(data, &input_levels.input_peak, &input_levels.input_rms);
                    push_input_samples(data.iter().copied(), &mut input_producer);
                },
                move |stream_error| {
                    if let Ok(mut error) = input_error.lock() {
                        *error = Some(format!("Input stream error: {stream_error}"));
                    }
                },
                None,
            ),
            SampleFormat::I32 => {
                let input_levels = Arc::clone(&levels);
                let input_error = Arc::clone(&error);
                let mut converted = Vec::new();
                input_device.build_input_stream(
                    input_stream_config,
                    move |data: &[i32], _| {
                        converted.resize(data.len(), 0.0);
                        for (target, &sample) in converted.iter_mut().zip(data) {
                            *target = i32_to_f32(sample);
                        }
                        update_levels(
                            &converted,
                            &input_levels.input_peak,
                            &input_levels.input_rms,
                        );
                        push_input_samples(converted.iter().copied(), &mut input_producer);
                    },
                    move |stream_error| {
                        if let Ok(mut error) = input_error.lock() {
                            *error = Some(format!("Input stream error: {stream_error}"));
                        }
                    },
                    None,
                )
            }
            format => unreachable!("unsupported input format: {format:?}"),
        }
        .map_err(|e| format!("Could not create input stream: {e}"))?;

        let output_plugin = Arc::clone(&plugin);
        let output_levels = Arc::clone(&levels);
        let output_error = Arc::clone(&error);
        let mut buffers = AudioBuffers::new(CHANNELS, CHANNELS, BLOCK_SIZE, sample_rate as f64);
        let output_stream = match output_supported.sample_format() {
            SampleFormat::F32 => output_device.build_output_stream(
                output_stream_config,
                move |data: &mut [f32], _| {
                    process_output(
                        data,
                        &mut input_consumer,
                        &mut output_producer,
                        &output_plugin,
                        &mut buffers,
                        &output_levels,
                        input_channels,
                        output_channels,
                        input_left,
                        input_right,
                        output_left,
                        output_right,
                    );
                },
                move |stream_error| {
                    if let Ok(mut error) = output_error.lock() {
                        *error = Some(format!("Output stream error: {stream_error}"));
                    }
                },
                None,
            ),
            SampleFormat::I32 => {
                let output_plugin = Arc::clone(&plugin);
                let output_levels = Arc::clone(&levels);
                let output_error = Arc::clone(&error);
                let mut converted = Vec::new();
                output_device.build_output_stream(
                    output_stream_config,
                    move |data: &mut [i32], _| {
                        converted.resize(data.len(), 0.0);
                        process_output(
                            &mut converted,
                            &mut input_consumer,
                            &mut output_producer,
                            &output_plugin,
                            &mut buffers,
                            &output_levels,
                            input_channels,
                            output_channels,
                            input_left,
                            input_right,
                            output_left,
                            output_right,
                        );
                        for (target, &sample) in data.iter_mut().zip(&converted) {
                            *target = f32_to_i32(sample);
                        }
                    },
                    move |stream_error| {
                        if let Ok(mut error) = output_error.lock() {
                            *error = Some(format!("Output stream error: {stream_error}"));
                        }
                    },
                    None,
                )
            }
            format => unreachable!("unsupported output format: {format:?}"),
        }
        .map_err(|e| format!("Could not create output stream: {e}"))?;

        plugin
            .lock()
            .map_err(|_| "plugin lock poisoned".to_string())?
            .start_processing()
            .map_err(|e| e.to_string())?;

        if let Err(error) = input_stream.play() {
            stop_plugin(&plugin);
            return Err(format!("Could not start input stream: {error}"));
        }
        if let Err(error) = output_stream.play() {
            stop_plugin(&plugin);
            return Err(format!("Could not start output stream: {error}"));
        }

        Ok(Self {
            _input_stream: input_stream,
            _output_stream: output_stream,
            plugin,
            output_tap,
            levels,
            error,
        })
    }

    pub fn shutdown(self) {
        let Self {
            _input_stream: input_stream,
            _output_stream: output_stream,
            plugin,
            ..
        } = self;
        let _ = input_stream.pause();
        let _ = output_stream.pause();
        drop(input_stream);
        drop(output_stream);
        if let Ok(mut plugin) = plugin.lock() {
            let _ = plugin.stop_processing();
        }
    }

    pub fn levels(&self) -> SignalLevels {
        self.levels.read()
    }

    pub fn drain_output(&mut self, samples: &mut Vec<f32>) {
        while let Ok(sample) = self.output_tap.pop() {
            samples.push(sample);
        }
    }

    pub fn take_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut error| error.take())
    }
}

fn stop_plugin(plugin: &Arc<Mutex<Plugin>>) {
    if let Ok(mut plugin) = plugin.lock() {
        let _ = plugin.stop_processing();
    }
}

fn is_supported_sample_format(format: SampleFormat) -> bool {
    matches!(format, SampleFormat::F32 | SampleFormat::I32)
}

fn i32_to_f32(sample: i32) -> f32 {
    sample as f32 / 2_147_483_648.0
}

fn f32_to_i32(sample: f32) -> i32 {
    if !sample.is_finite() {
        return 0;
    }
    let sample = sample.clamp(-1.0, 1.0);
    if sample <= -1.0 {
        i32::MIN
    } else if sample >= 1.0 {
        i32::MAX
    } else {
        (sample * 2_147_483_648.0).round() as i32
    }
}

fn push_input_samples(samples: impl Iterator<Item = f32>, producer: &mut Producer<f32>) {
    for sample in samples {
        if producer.push(sample).is_err() {
            break;
        }
    }
}

fn process_output(
    data: &mut [f32],
    input_consumer: &mut Consumer<f32>,
    output_producer: &mut Producer<f32>,
    plugin: &Arc<Mutex<Plugin>>,
    buffers: &mut AudioBuffers,
    levels: &LevelAtomics,
    input_channels: usize,
    output_channels: usize,
    input_left: usize,
    input_right: usize,
    output_left: usize,
    output_right: usize,
) {
    data.fill(0.0);
    let frames = data.len() / output_channels;
    for channel in buffers.inputs.iter_mut().chain(buffers.outputs.iter_mut()) {
        if channel.len() != frames {
            channel.resize(frames, 0.0);
        }
    }
    buffers.block_size = frames;

    for frame in 0..frames {
        let mut left = 0.0;
        let mut right = 0.0;
        for channel in 0..input_channels {
            let sample = input_consumer.pop().unwrap_or(0.0);
            route_input_sample(
                channel,
                sample,
                input_left,
                input_right,
                &mut left,
                &mut right,
            );
        }
        buffers.inputs[0][frame] = left;
        buffers.inputs[1][frame] = right;
    }

    let processed = plugin
        .lock()
        .map(|mut plugin| plugin.process_audio(buffers).is_ok())
        .unwrap_or(false);
    if !processed {
        return;
    }

    for frame in 0..frames {
        let left = buffers.outputs[0][frame];
        let right = buffers.outputs[1][frame];
        route_output_sample(
            &mut data[frame * output_channels..(frame + 1) * output_channels],
            output_left,
            output_right,
            left,
            right,
        );
        let _ = output_producer.push(left);
        let _ = output_producer.push(right);
    }
    update_channel_levels(
        &buffers.outputs,
        frames,
        &levels.output_peak,
        &levels.output_rms,
    );
}

fn route_input_sample(
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

fn route_output_sample(
    frame: &mut [f32],
    output_left: usize,
    output_right: usize,
    left: f32,
    right: f32,
) {
    frame[output_left] = left;
    frame[output_right] = right;
}

fn update_levels(samples: &[f32], peak: &AtomicU32, rms: &AtomicU32) {
    let max = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0, f32::max);
    let mean_square = if samples.is_empty() {
        0.0
    } else {
        samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32
    };
    peak.store(max.to_bits(), Ordering::Relaxed);
    rms.store(mean_square.sqrt().to_bits(), Ordering::Relaxed);
}

fn update_channel_levels(channels: &[Vec<f32>], frames: usize, peak: &AtomicU32, rms: &AtomicU32) {
    let mut max: f32 = 0.0;
    let mut sum = 0.0;
    let mut count = 0;
    for channel in channels {
        for &sample in channel.iter().take(frames) {
            max = max.max(sample.abs());
            sum += sample * sample;
            count += 1;
        }
    }
    peak.store(max.to_bits(), Ordering::Relaxed);
    rms.store(
        if count == 0 {
            0.0
        } else {
            (sum / count as f32).sqrt()
        }
        .to_bits(),
        Ordering::Relaxed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_peak_and_rms() {
        let peak = AtomicU32::new(0);
        let rms = AtomicU32::new(0);
        update_levels(&[-1.0, 0.5, -0.5, 0.0], &peak, &rms);

        assert_eq!(f32::from_bits(peak.load(Ordering::Relaxed)), 1.0);
        assert!((f32::from_bits(rms.load(Ordering::Relaxed)) - 0.6123724).abs() < 0.0001);
    }

    #[test]
    fn empty_levels_are_silent() {
        let peak = AtomicU32::new(1.0f32.to_bits());
        let rms = AtomicU32::new(1.0f32.to_bits());
        update_levels(&[], &peak, &rms);

        assert_eq!(f32::from_bits(peak.load(Ordering::Relaxed)), 0.0);
        assert_eq!(f32::from_bits(rms.load(Ordering::Relaxed)), 0.0);
    }

    #[test]
    fn converts_i32_samples_to_normalized_f32() {
        assert_eq!(i32_to_f32(0), 0.0);
        assert_eq!(i32_to_f32(i32::MIN), -1.0);
        assert!((i32_to_f32(i32::MAX) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn converts_f32_samples_to_clamped_i32() {
        assert_eq!(f32_to_i32(-2.0), i32::MIN);
        assert_eq!(f32_to_i32(-1.0), i32::MIN);
        assert_eq!(f32_to_i32(0.0), 0);
        assert_eq!(f32_to_i32(1.0), i32::MAX);
        assert_eq!(f32_to_i32(2.0), i32::MAX);
        assert_eq!(f32_to_i32(f32::NAN), 0);
    }

    #[test]
    fn routes_selected_input_channels() {
        let mut left = 0.0;
        let mut right = 0.0;
        for (channel, sample) in [0.1, 0.2, 0.3, 0.4].into_iter().enumerate() {
            route_input_sample(channel, sample, 2, 0, &mut left, &mut right);
        }

        assert_eq!(left, 0.3);
        assert_eq!(right, 0.1);
    }

    #[test]
    fn allows_one_input_channel_for_both_plugin_inputs() {
        let mut left = 0.0;
        let mut right = 0.0;
        route_input_sample(1, 0.75, 1, 1, &mut left, &mut right);

        assert_eq!(left, 0.75);
        assert_eq!(right, 0.75);
    }

    #[test]
    fn routes_plugin_outputs_to_selected_hardware_channels() {
        let mut frame = [0.0; 4];
        route_output_sample(&mut frame, 2, 0, 0.25, -0.5);

        assert_eq!(frame, [-0.5, 0.0, 0.25, 0.0]);
    }
}
