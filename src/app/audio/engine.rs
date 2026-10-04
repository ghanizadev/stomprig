use super::conversion::{f32_to_i32, i32_to_f32, is_supported_sample_format};
use super::devices::AudioDevices;
use super::metering::LevelAtomics;
use super::processing::{
    BLOCK_SIZE, CHANNELS, ProcessContext, process_output, update_input_levels,
};
use super::streams::report_error;
use super::types::{AudioConfig, SignalLevels};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, Stream};
use rtrb::{Consumer, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use vst3_host::{AudioBuffers, Plugin};

const TAP_CAPACITY: usize = 16_384;

struct CallbackGate {
    stopping: AtomicBool,
    active: AtomicUsize,
}

impl CallbackGate {
    fn new() -> Self {
        Self {
            stopping: AtomicBool::new(false),
            active: AtomicUsize::new(0),
        }
    }

    fn enter(self: &Arc<Self>) -> Option<CallbackGuard> {
        if self.stopping.load(Ordering::Acquire) {
            return None;
        }

        self.active.fetch_add(1, Ordering::AcqRel);
        if self.stopping.load(Ordering::Acquire) {
            self.active.fetch_sub(1, Ordering::Release);
            return None;
        }

        Some(CallbackGuard {
            gate: Arc::clone(self),
        })
    }

    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    fn wait_until_idle(&self) {
        while self.active.load(Ordering::Acquire) != 0 {
            thread::yield_now();
        }
    }
}

struct CallbackGuard {
    gate: Arc<CallbackGate>,
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        self.gate.active.fetch_sub(1, Ordering::Release);
    }
}

pub struct AudioEngine {
    input_stream: Stream,
    output_stream: Stream,
    plugins: Arc<Vec<Arc<Mutex<Plugin>>>>,
    output_tap: Consumer<f32>,
    levels: Arc<LevelAtomics>,
    error: Arc<Mutex<Option<String>>>,
    callback_gate: Arc<CallbackGate>,
}

#[derive(Clone, Copy)]
pub enum ShutdownMode {
    UnloadPlugins,
    KeepPluginsLoaded,
}

impl AudioEngine {
    pub fn start(
        plugins: Arc<Vec<Arc<Mutex<Plugin>>>>,
        config: AudioConfig,
    ) -> Result<Self, String> {
        if plugins.is_empty() {
            return Err("The plugin stack is empty".to_string());
        }
        let (input_device, output_device) = AudioDevices::selected_devices(
            config.backend,
            config.input_index,
            config.output_index,
        )?;
        if config.backend == super::types::AudioBackend::Asio
            && input_device
                .id()
                .map_err(|e| format!("Could not identify ASIO input driver: {e}"))?
                != output_device
                    .id()
                    .map_err(|e| format!("Could not identify ASIO output driver: {e}"))?
        {
            return Err("ASIO input and output must use the same driver".to_string());
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
        log::info!(
            "audio backend {:?}: input {:?} / output {:?}",
            config.backend,
            input_supported,
            output_supported
        );
        log::info!(
            "audio backend {:?}: input {:?} / output {:?}",
            config.backend,
            input_supported,
            output_supported
        );
        let input_channels = usize::from(input_supported.channels());
        let output_channels = usize::from(output_supported.channels());
        let routing = config.routing;
        if input_channels == 0 || output_channels == 0 {
            return Err("Selected devices must provide at least one channel".to_string());
        }
        if routing.input_left >= input_channels || routing.input_right >= input_channels {
            return Err(format!(
                "Selected input channel is unavailable (device has {input_channels} channels)"
            ));
        }
        if routing.output_left >= output_channels || routing.output_right >= output_channels {
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
        let buffer_size = config.buffer_size.frames();
        let supports_buffer = |supported: &cpal::SupportedBufferSize| match supported {
            cpal::SupportedBufferSize::Range { min, max } => (*min..=*max).contains(&buffer_size),
            cpal::SupportedBufferSize::Unknown => true,
        };
        let stream_buffer_size = if supports_buffer(input_supported.buffer_size())
            && supports_buffer(output_supported.buffer_size())
        {
            BufferSize::Fixed(buffer_size)
        } else {
            BufferSize::Default
        };
        let levels = Arc::new(LevelAtomics::new());
        let error = Arc::new(Mutex::new(None));
        let callback_gate = Arc::new(CallbackGate::new());
        let (mut input_producer, mut input_consumer) = RingBuffer::new(TAP_CAPACITY);
        let (mut output_producer, output_tap) = RingBuffer::new(TAP_CAPACITY);
        let input_levels = Arc::clone(&levels);
        let input_error = Arc::clone(&error);
        let input_callback_gate = Arc::clone(&callback_gate);
        let mut input_config = input_supported.config();
        input_config.buffer_size = stream_buffer_size;
        let input_stream = match input_supported.sample_format() {
            SampleFormat::F32 => input_device.build_input_stream(
                input_config,
                move |data: &[f32], _| {
                    let Some(_callback_guard) = input_callback_gate.enter() else {
                        return;
                    };
                    update_input_levels(data, &input_levels);
                    super::processing::push_input_samples(
                        data.iter().copied(),
                        &mut input_producer,
                    );
                },
                move |e| report_error(&input_error, format!("Input stream error: {e}")),
                None,
            ),
            SampleFormat::I32 => {
                let input_levels = Arc::clone(&levels);
                let input_error = Arc::clone(&error);
                let input_callback_gate = Arc::clone(&callback_gate);
                let mut converted = Vec::new();
                input_device.build_input_stream(
                    input_config,
                    move |data: &[i32], _| {
                        let Some(_callback_guard) = input_callback_gate.enter() else {
                            return;
                        };
                        converted.resize(data.len(), 0.0);
                        for (target, &sample) in converted.iter_mut().zip(data) {
                            *target = i32_to_f32(sample);
                        }
                        update_input_levels(&converted, &input_levels);
                        super::processing::push_input_samples(
                            converted.iter().copied(),
                            &mut input_producer,
                        );
                    },
                    move |e| report_error(&input_error, format!("Input stream error: {e}")),
                    None,
                )
            }
            _ => unreachable!(),
        }
        .map_err(|e| format!("Could not create input stream: {e}"))?;
        let output_plugins = Arc::clone(&plugins);
        let output_levels = Arc::clone(&levels);
        let output_error = Arc::clone(&error);
        let output_callback_gate = Arc::clone(&callback_gate);
        let mut buffers = AudioBuffers::new(CHANNELS, CHANNELS, BLOCK_SIZE, sample_rate as f64);
        let mut output_config = output_supported.config();
        output_config.buffer_size = stream_buffer_size;
        let output_stream = match output_supported.sample_format() {
            SampleFormat::F32 => output_device.build_output_stream(
                output_config,
                move |data: &mut [f32], _| {
                    let Some(_callback_guard) = output_callback_gate.enter() else {
                        return;
                    };
                    let mut context = ProcessContext {
                        input_channels,
                        output_channels,
                        input_left: routing.input_left,
                        input_right: routing.input_right,
                        output_left: routing.output_left,
                        output_right: routing.output_right,
                        input_consumer: &mut input_consumer,
                        output_producer: &mut output_producer,
                        plugins: output_plugins.as_slice(),
                        buffers: &mut buffers,
                        levels: &output_levels,
                    };
                    process_output(data, &mut context);
                },
                move |e| report_error(&output_error, format!("Output stream error: {e}")),
                None,
            ),
            SampleFormat::I32 => {
                let output_plugins = Arc::clone(&plugins);
                let output_levels = Arc::clone(&levels);
                let output_error = Arc::clone(&error);
                let output_callback_gate = Arc::clone(&callback_gate);
                let mut converted = Vec::new();
                output_device.build_output_stream(
                    output_config,
                    move |data: &mut [i32], _| {
                        let Some(_callback_guard) = output_callback_gate.enter() else {
                            return;
                        };
                        converted.resize(data.len(), 0.0);
                        let mut context = ProcessContext {
                            input_channels,
                            output_channels,
                            input_left: routing.input_left,
                            input_right: routing.input_right,
                            output_left: routing.output_left,
                            output_right: routing.output_right,
                            input_consumer: &mut input_consumer,
                            output_producer: &mut output_producer,
                            plugins: output_plugins.as_slice(),
                            buffers: &mut buffers,
                            levels: &output_levels,
                        };
                        process_output(&mut converted, &mut context);
                        for (target, &sample) in data.iter_mut().zip(&converted) {
                            *target = f32_to_i32(sample);
                        }
                    },
                    move |e| report_error(&output_error, format!("Output stream error: {e}")),
                    None,
                )
            }
            _ => unreachable!(),
        }
        .map_err(|e| format!("Could not create output stream: {e}"))?;
        log::info!(
            "streams built (buffer {stream_buffer_size:?}, {sample_rate} Hz); starting {} plugin(s)",
            plugins.len()
        );
        start_plugins(&plugins)?;
        log::info!("plugins started; playing streams");
        input_stream.play().map_err(|e| {
            stop_plugins(&plugins);
            format!("Could not start input stream: {e}")
        })?;
        output_stream.play().map_err(|e| {
            stop_plugins(&plugins);
            format!("Could not start output stream: {e}")
        })?;
        Ok(Self {
            input_stream,
            output_stream,
            plugins,
            output_tap,
            levels,
            error,
            callback_gate,
        })
    }
    pub fn shutdown(self, mode: ShutdownMode) {
        log::info!("audio engine shutdown");
        let Self {
            input_stream,
            output_stream,
            plugins,
            callback_gate,
            ..
        } = self;
        callback_gate.stop();
        let _ = input_stream.pause();
        let _ = output_stream.pause();
        log::debug!("audio callbacks paused; waiting for active callbacks");
        callback_gate.wait_until_idle();
        if matches!(mode, ShutdownMode::UnloadPlugins) {
            log::debug!("audio callbacks idle; stopping plugins");
            stop_plugins(&plugins);
            log::debug!("plugins stopped; releasing audio streams");
        } else {
            log::debug!("audio callbacks idle; releasing audio streams");
        }
        drop(input_stream);
        drop(output_stream);
        log::debug!("audio streams released");
        if matches!(mode, ShutdownMode::KeepPluginsLoaded) {
            // Keep the plugin references alive until process termination. Dropping them here
            // would call third-party native destructors during eframe's shutdown sequence.
            std::mem::forget(plugins);
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

fn start_plugins(plugins: &[Arc<Mutex<Plugin>>]) -> Result<(), String> {
    let mut started = 0;
    for plugin in plugins {
        match plugin.lock() {
            Ok(mut plugin) => {
                if let Err(error) = plugin.start_processing() {
                    stop_plugins(&plugins[..started]);
                    return Err(error.to_string());
                }
                started += 1;
            }
            Err(_) => {
                stop_plugins(&plugins[..started]);
                return Err("plugin lock poisoned".to_string());
            }
        }
    }
    Ok(())
}

fn stop_plugins(plugins: &[Arc<Mutex<Plugin>>]) {
    for plugin in plugins {
        if let Ok(mut plugin) = plugin.lock() {
            let _ = plugin.stop_processing();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CallbackGate;
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn stopping_prevents_new_callbacks() {
        let gate = Arc::new(CallbackGate::new());
        let callback = gate.enter().expect("callback should enter before stop");

        gate.stop();
        assert!(gate.enter().is_none());

        drop(callback);
        gate.wait_until_idle();
    }

    #[test]
    fn waits_for_active_callback_to_exit() {
        let gate = Arc::new(CallbackGate::new());
        let callback = gate.enter().expect("callback should enter");
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let waiter_gate = Arc::clone(&gate);

        thread::spawn(move || {
            started_tx.send(()).unwrap();
            waiter_gate.wait_until_idle();
            finished_tx.send(()).unwrap();
        });

        started_rx.recv().unwrap();
        assert!(finished_rx.try_recv().is_err());
        drop(callback);
        finished_rx.recv().unwrap();
    }
}
