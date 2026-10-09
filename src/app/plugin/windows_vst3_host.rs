use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

use vst3_host::{Plugin, PluginWindow};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

const WINDOW_THREAD_POLL_INTERVAL: Duration = Duration::from_millis(8);

enum EditorCommand {
    Open,
    Close,
}

struct EditorThreadState {
    is_open: AtomicBool,
    open_pending: AtomicBool,
    closed_by_user: AtomicBool,
    service_requested: AtomicBool,
    shutdown_requested: AtomicBool,
    error: Mutex<Option<String>>,
}

struct EditorThreadControl {
    commands: SyncSender<EditorCommand>,
    state: Arc<EditorThreadState>,
}

impl Drop for EditorThreadControl {
    fn drop(&mut self) {
        self.state.shutdown_requested.store(true, Ordering::Release);
    }
}

/// UI-thread-facing handle for the native VST3 editor owned by its plugin-loading thread.
///
/// A number of Windows VST3 implementations create thread-local GUI state while their module or
/// controller is initialized. Keeping editor creation and the HWND message pump on that same
/// persistent thread avoids handing a view to a different thread after background loading.
#[derive(Clone)]
pub(crate) struct WindowsVst3EditorHost {
    control: Arc<EditorThreadControl>,
}

impl WindowsVst3EditorHost {
    pub(crate) fn spawn_for_plugin(
        load_plugin: impl FnOnce() -> Result<Plugin, String> + Send + 'static,
    ) -> Result<(Arc<Mutex<Plugin>>, Self), String> {
        let (command_sender, command_receiver) = mpsc::sync_channel(8);
        let (loaded_sender, loaded_receiver) = mpsc::sync_channel(1);
        let state = Arc::new(EditorThreadState {
            is_open: AtomicBool::new(false),
            open_pending: AtomicBool::new(false),
            closed_by_user: AtomicBool::new(false),
            service_requested: AtomicBool::new(false),
            shutdown_requested: AtomicBool::new(false),
            error: Mutex::new(None),
        });
        let control = Arc::new(EditorThreadControl {
            commands: command_sender,
            state: Arc::clone(&state),
        });
        let thread_state = Arc::clone(&state);

        thread::Builder::new()
            .name("rack-vst3-editor".to_owned())
            .spawn(move || {
                let plugin = match load_plugin() {
                    Ok(plugin) => plugin,
                    Err(error) => {
                        let _ = loaded_sender.send(Err(error));
                        return;
                    }
                };
                let plugin = Arc::new(Mutex::new(plugin));
                if loaded_sender.send(Ok(Arc::clone(&plugin))).is_ok() {
                    // Keep the owner thread alive even when this plugin has no native editor;
                    // Rack's custom controls still queue controller updates for this thread.
                    let window = PluginWindow::new(Arc::clone(&plugin));
                    editor_thread_loop(window, plugin, command_receiver, thread_state);
                }
            })
            .map_err(|error| format!("could not start VST3 editor thread: {error}"))?;

        let plugin = loaded_receiver
            .recv()
            .map_err(|_| "VST3 editor thread stopped while loading the plugin".to_owned())??;
        Ok((plugin, Self { control }))
    }

    pub(crate) fn open(&self) -> Result<(), String> {
        if self.control.state.is_open.load(Ordering::Acquire)
            || self.control.state.open_pending.swap(true, Ordering::AcqRel)
        {
            return Ok(());
        }
        self.control
            .state
            .closed_by_user
            .store(false, Ordering::Release);
        match self.control.commands.try_send(EditorCommand::Open) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.control
                    .state
                    .open_pending
                    .store(false, Ordering::Release);
                Err(editor_command_error("open", error))
            }
        }
    }

    pub(crate) fn close(&self) {
        self.control
            .state
            .open_pending
            .store(false, Ordering::Release);
        self.control
            .state
            .closed_by_user
            .store(false, Ordering::Release);
        if let Err(error) = self.control.commands.try_send(EditorCommand::Close) {
            log::debug!(
                "could not queue VST3 editor close request: {}",
                editor_command_error("close", error)
            );
        }
    }

    /// Queue host-side editor servicing from Rack's UI frame. The owner thread also pumps native
    /// HWND messages continuously so repaint and input handling do not depend on eframe timing.
    pub(crate) fn service_platform_events(&self) -> Result<(), String> {
        self.control
            .state
            .service_requested
            .store(true, Ordering::Release);
        match self.control.state.error.lock() {
            Ok(mut error) => error.take().map_or(Ok(()), Err),
            Err(_) => Err("VST3 editor thread state lock poisoned".to_owned()),
        }
    }

    pub(crate) fn closed_by_user(&self) -> bool {
        self.control.state.closed_by_user.load(Ordering::Acquire)
    }
}

fn editor_command_error(action: &str, error: TrySendError<EditorCommand>) -> String {
    match error {
        TrySendError::Full(_) => format!("VST3 editor thread is busy; could not queue {action}"),
        TrySendError::Disconnected(_) => format!("VST3 editor thread stopped before {action}"),
    }
}

fn editor_thread_loop(
    mut window: PluginWindow,
    plugin: Arc<Mutex<Plugin>>,
    commands: mpsc::Receiver<EditorCommand>,
    state: Arc<EditorThreadState>,
) {
    loop {
        if state.shutdown_requested.load(Ordering::Acquire) {
            break;
        }
        match commands.recv_timeout(WINDOW_THREAD_POLL_INTERVAL) {
            Ok(command) => {
                service_editor_command(command, &mut window, &state);
                while let Ok(command) = commands.try_recv() {
                    service_editor_command(command, &mut window, &state);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        pump_windows_messages();
        service_plugin_control_thread(&plugin);
        if window.closed_by_user() {
            window.close();
            state.is_open.store(false, Ordering::Release);
            state.closed_by_user.store(true, Ordering::Release);
        }
        if (state.service_requested.swap(false, Ordering::AcqRel) || window.is_open())
            && let Err(error) = window.service_platform_events()
        {
            store_editor_error(&state, error.to_string());
        }
    }

    window.close();
    state.is_open.store(false, Ordering::Release);
    state.open_pending.store(false, Ordering::Release);
}

fn service_plugin_control_thread(plugin: &Arc<Mutex<Plugin>>) {
    let mut plugin = match plugin.try_lock() {
        Ok(plugin) => plugin,
        Err(std::sync::TryLockError::Poisoned(poison)) => poison.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    plugin.service_control_thread();
}

fn service_editor_command(
    command: EditorCommand,
    window: &mut PluginWindow,
    state: &EditorThreadState,
) {
    match command {
        EditorCommand::Open => {
            match window.open() {
                Ok(()) => {
                    state.is_open.store(true, Ordering::Release);
                    state.closed_by_user.store(false, Ordering::Release);
                }
                Err(error) => {
                    state.is_open.store(false, Ordering::Release);
                    store_editor_error(state, error.to_string());
                }
            }
            state.open_pending.store(false, Ordering::Release);
        }
        EditorCommand::Close => {
            window.close();
            state.is_open.store(false, Ordering::Release);
        }
    }
}

fn store_editor_error(state: &EditorThreadState, message: String) {
    if let Ok(mut error) = state.error.lock() {
        *error = Some(message);
    }
}

fn pump_windows_messages() {
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
