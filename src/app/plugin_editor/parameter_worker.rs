use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Instant;

use super::super::plugin::PluginParameterValue;
use super::super::plugin::RackPlugin;

enum PluginParameterCommand {
    Read {
        generation: u64,
        parameters: Vec<(u32, bool)>,
    },
    Set(u32, f64),
    SetMany(Vec<(u32, f64)>),
}

pub(super) enum PluginParameterResult {
    Values {
        generation: u64,
        result: Result<Vec<PluginParameterValue>, String>,
    },
    Set {
        id: u32,
        result: Result<(), String>,
    },
    SetMany(Result<(), String>),
    WorkerError(String),
}

pub(super) struct PluginParameterWorker {
    commands: mpsc::Sender<PluginParameterCommand>,
    results: mpsc::Receiver<PluginParameterResult>,
    read_pending: bool,
    generation: u64,
}

impl PluginParameterWorker {
    pub(super) fn new(plugin: Arc<Mutex<RackPlugin>>) -> Result<Self, String> {
        let (command_sender, command_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        thread::Builder::new()
            .name("rack-plugin-parameters".to_owned())
            .spawn(move || {
                while let Ok(command) = command_receiver.recv() {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match command {
                            PluginParameterCommand::Read {
                                generation,
                                parameters,
                            } => {
                                let started = Instant::now();
                                let result = with_plugin(&plugin, |plugin| {
                                    plugin.parameter_values(&parameters)
                                });
                                let elapsed = started.elapsed();
                                if elapsed >= std::time::Duration::from_millis(50) {
                                    log::warn!(
                                        "reading {} parameter values took {elapsed:?} on the parameter worker",
                                        parameters.len()
                                    );
                                }
                                PluginParameterResult::Values { generation, result }
                            }
                            PluginParameterCommand::Set(id, value) => {
                                let result = with_plugin(&plugin, |plugin| {
                                    plugin.set_parameter(id, value)
                                });
                                PluginParameterResult::Set { id, result }
                            }
                            PluginParameterCommand::SetMany(parameters) => {
                                let result = with_plugin(&plugin, |plugin| {
                                    let mut errors = Vec::new();
                                    for (id, value) in parameters {
                                        if let Err(error) = plugin.set_parameter(id, value) {
                                            errors.push(format!("parameter {id}: {error}"));
                                        }
                                    }
                                    if errors.is_empty() {
                                        Ok(())
                                    } else {
                                        Err(errors.join("; "))
                                    }
                                });
                                PluginParameterResult::SetMany(result)
                            }
                        }))
                        .unwrap_or_else(|panic| {
                        let message = panic
                            .downcast_ref::<&str>()
                            .copied()
                            .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                            .unwrap_or("unknown plugin parameter worker panic");
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

        Ok(Self {
            commands: command_sender,
            results: result_receiver,
            read_pending: false,
            generation: 0,
        })
    }

    pub(super) fn request_read(&mut self, parameters: Vec<(u32, bool)>) {
        if self.read_pending {
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
        self.commands
            .send(PluginParameterCommand::Set(id, value))
            .map_err(|_| "plugin parameter worker stopped".to_owned())?;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    pub(super) fn set_many(&mut self, parameters: Vec<(u32, f64)>) -> Result<(), String> {
        self.commands
            .send(PluginParameterCommand::SetMany(parameters))
            .map_err(|_| "plugin parameter worker stopped".to_owned())?;
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
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => None,
        }
    }
}

fn with_plugin<T>(
    plugin: &Arc<Mutex<RackPlugin>>,
    operation: impl FnOnce(&mut RackPlugin) -> Result<T, String>,
) -> Result<T, String> {
    let mut plugin = plugin
        .lock()
        .map_err(|_| "plugin lock poisoned".to_owned())?;
    operation(&mut plugin)
}
