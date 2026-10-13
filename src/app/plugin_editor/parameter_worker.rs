use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::super::plugin::{
    ParameterChangeConsumer, ParameterChangeProducer, ParameterRuntimeFeedbackConsumer,
    ParameterRuntimeFeedbackProducer, PluginParameterValue, StompRigPlugin, parameter_change_queue,
    parameter_runtime_feedback_queue,
};

const PARAMETER_WORKER_POLL_INTERVAL: Duration = Duration::from_millis(1);

enum PluginParameterCommand {
    Read {
        generation: u64,
        parameters: Vec<(u32, bool)>,
    },
}

pub(super) enum PluginParameterResult {
    Values {
        generation: u64,
        result: Result<Vec<PluginParameterValue>, String>,
    },
    ParameterChanges(Vec<(u32, f64)>),
    ParameterUpdateError(String),
    WorkerError(String),
}

pub(super) struct PluginParameterWorker {
    commands: mpsc::Sender<PluginParameterCommand>,
    results: mpsc::Receiver<PluginParameterResult>,
    feedback: ParameterRuntimeFeedbackConsumer,
    parameter_changes: ParameterChangeProducer,
    audio_active: Arc<AtomicBool>,
    read_pending: bool,
    generation: u64,
}

impl PluginParameterWorker {
    pub(super) fn new(
        plugin: Arc<Mutex<StompRigPlugin>>,
        audio_active: Arc<AtomicBool>,
    ) -> Result<
        (
            Self,
            ParameterChangeConsumer,
            ParameterRuntimeFeedbackProducer,
        ),
        String,
    > {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let (feedback_producer, feedback_consumer) = parameter_runtime_feedback_queue();
        let (parameter_change_producer, parameter_change_consumer) = parameter_change_queue();
        let worker_audio_active = Arc::clone(&audio_active);

        thread::Builder::new()
            .name("stomprig-plugin-parameters".to_owned())
            .spawn(move || {
                let mut pending_read = None;
                loop {
                    match command_receiver.recv_timeout(PARAMETER_WORKER_POLL_INTERVAL) {
                        Ok(command) => pending_read = Some(command),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }

                    if worker_audio_active.load(Ordering::Acquire) {
                        continue;
                    }
                    let Some(command) = pending_read.take() else {
                        continue;
                    };

                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        process_parameter_command(&plugin, command)
                    }))
                    .unwrap_or_else(|panic| {
                        let message = panic_message(panic.as_ref());
                        log::error!("plugin parameter worker panicked: {message}");
                        PluginParameterResult::WorkerError(format!(
                            "plugin parameter worker panicked: {message}"
                        ))
                    });
                    if result_sender.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("could not start plugin parameter worker: {error}"))?;

        Ok((
            Self {
                commands: command_sender,
                results: result_receiver,
                feedback: feedback_consumer,
                parameter_changes: parameter_change_producer,
                audio_active,
                read_pending: false,
                generation: 0,
            },
            parameter_change_consumer,
            feedback_producer,
        ))
    }

    pub(super) fn request_read(&mut self, parameters: Vec<(u32, bool)>) {
        if self.read_pending || self.audio_active.load(Ordering::Acquire) {
            return;
        }
        if self
            .commands
            .send(PluginParameterCommand::Read {
                generation: self.generation,
                parameters,
            })
            .is_ok()
        {
            self.read_pending = true;
        }
    }

    pub(super) fn set(&mut self, id: u32, value: f64) -> Result<(), String> {
        self.parameter_changes.push(id, value)?;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    pub(super) fn set_many(&mut self, parameters: Vec<(u32, f64)>) -> Result<(), String> {
        if parameters.len() > self.parameter_changes.available_slots() {
            return Err("plugin parameter control queue is full".to_owned());
        }
        for (id, value) in parameters {
            self.parameter_changes.push(id, value)?;
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    pub(super) fn is_current_generation(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub(super) fn try_recv(&mut self) -> Option<PluginParameterResult> {
        match self.results.try_recv() {
            Ok(result) => {
                if matches!(
                    result,
                    PluginParameterResult::Values { .. } | PluginParameterResult::WorkerError(_)
                ) {
                    self.read_pending = false;
                }
                return Some(result);
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {}
        }

        if let Some((id, value)) = self.feedback.pop() {
            return Some(PluginParameterResult::ParameterChanges(vec![(id, value)]));
        }
        let dropped_updates = self.feedback.take_dropped_updates();
        if dropped_updates > 0 {
            return Some(PluginParameterResult::ParameterUpdateError(format!(
                "dropped {dropped_updates} plug-in parameter feedback update(s) because the UI queue was full"
            )));
        }
        None
    }
}

fn process_parameter_command(
    plugin: &Arc<Mutex<StompRigPlugin>>,
    command: PluginParameterCommand,
) -> PluginParameterResult {
    match command {
        PluginParameterCommand::Read {
            generation,
            parameters,
        } => {
            let started = Instant::now();
            let result = with_plugin(plugin, |plugin| plugin.parameter_values(&parameters));
            let elapsed = started.elapsed();
            if elapsed >= Duration::from_millis(50) {
                log::warn!(
                    "reading {} parameter values took {elapsed:?} on the parameter worker",
                    parameters.len()
                );
            }
            PluginParameterResult::Values { generation, result }
        }
    }
}

fn with_plugin<T>(
    plugin: &Arc<Mutex<StompRigPlugin>>,
    operation: impl FnOnce(&mut StompRigPlugin) -> Result<T, String>,
) -> Result<T, String> {
    let mut plugin = plugin
        .lock()
        .map_err(|_| "plugin lock poisoned".to_owned())?;
    operation(&mut plugin)
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown plugin parameter worker panic")
}
