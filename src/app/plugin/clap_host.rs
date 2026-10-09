use clack_extensions::gui::{GuiSize, HostGui, HostGuiImpl};
use clack_extensions::latency::{HostLatency, HostLatencyImpl};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
};
use clack_extensions::thread_check::{HostThreadCheck, HostThreadCheckImpl};
use clack_host::prelude::{
    HostError, HostExtensions, HostHandlers, MainThreadHandler, SharedHandler,
};
use clack_host::utils::ClapId;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread::{self, ThreadId};

pub(crate) struct RackClapHost;

#[derive(Default)]
pub(crate) struct ClapHostShared {
    main_thread: Option<ThreadId>,
    restart_requested: AtomicBool,
    process_requested: AtomicBool,
    callback_requested: AtomicBool,
    flush_requested: std::sync::Arc<AtomicBool>,
    parameter_rescan_requested: AtomicBool,
    latency_changed: AtomicBool,
    gui_action: AtomicU8,
    gui_closed: AtomicBool,
    gui_destroyed: AtomicBool,
}

pub(crate) struct ClapHostMainThread<'a> {
    pub(crate) shared: &'a ClapHostShared,
}

thread_local! {
    static IN_CLAP_AUDIO_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

pub(crate) struct AudioCallbackScope(bool);

impl AudioCallbackScope {
    pub(crate) fn enter() -> Self {
        let previous = IN_CLAP_AUDIO_CALLBACK.with(|flag| flag.replace(true));
        Self(previous)
    }
}

impl Drop for AudioCallbackScope {
    fn drop(&mut self) {
        IN_CLAP_AUDIO_CALLBACK.with(|flag| flag.set(self.0));
    }
}

impl ClapHostShared {
    pub(crate) fn new() -> Self {
        Self {
            main_thread: Some(thread::current().id()),
            ..Self::default()
        }
    }

    pub(crate) fn take_restart_requested(&self) -> bool {
        self.restart_requested.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_callback_requested(&self) -> bool {
        self.callback_requested.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_gui_action(&self) -> u8 {
        self.gui_action.swap(0, Ordering::AcqRel)
    }

    pub(crate) fn take_gui_destroyed(&self) -> bool {
        self.gui_destroyed.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_gui_closed(&self) -> bool {
        self.gui_closed.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_latency_changed(&self) -> bool {
        self.latency_changed.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_parameter_rescan_requested(&self) -> bool {
        self.parameter_rescan_requested
            .swap(false, Ordering::AcqRel)
    }

    pub(crate) fn flush_request_flag(&self) -> std::sync::Arc<AtomicBool> {
        std::sync::Arc::clone(&self.flush_requested)
    }
}

impl<'a> SharedHandler<'a> for ClapHostShared {
    fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::Release);
    }

    fn request_process(&self) {
        self.process_requested.store(true, Ordering::Release);
    }

    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::Release);
    }
}

impl<'a> MainThreadHandler<'a> for ClapHostMainThread<'a> {}
impl HostHandlers for RackClapHost {
    type Shared<'a> = ClapHostShared;
    type MainThread<'a> = ClapHostMainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _: &ClapHostShared) {
        builder
            .register::<HostParams>()
            .register::<HostGui>()
            .register::<HostLatency>()
            .register::<HostLog>()
            .register::<HostThreadCheck>();
    }
}

impl HostParamsImplShared for ClapHostShared {
    fn request_flush(&self) {
        self.flush_requested.store(true, Ordering::Release);
    }
}

impl HostParamsImplMainThread for ClapHostMainThread<'_> {
    fn rescan(&self, flags: ParamRescanFlags) {
        if !flags.is_empty() {
            self.shared
                .parameter_rescan_requested
                .store(true, Ordering::Release);
        }
        if flags.requires_restart() {
            self.shared.restart_requested.store(true, Ordering::Release);
        }
    }

    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

impl HostLatencyImpl for ClapHostMainThread<'_> {
    fn changed(&self) {
        self.shared.latency_changed.store(true, Ordering::Release);
    }
}

impl HostLogImpl for ClapHostShared {
    fn log(&self, severity: LogSeverity, message: &str) {
        match severity {
            LogSeverity::Debug => log::debug!("CLAP plugin: {message}"),
            LogSeverity::Info => log::info!("CLAP plugin: {message}"),
            LogSeverity::Warning => log::warn!("CLAP plugin: {message}"),
            LogSeverity::Error
            | LogSeverity::Fatal
            | LogSeverity::PluginMisbehaving
            | LogSeverity::HostMisbehaving => {
                log::error!("CLAP plugin: {message}")
            }
        }
    }
}

impl HostThreadCheckImpl for ClapHostShared {
    fn is_main_thread(&self) -> bool {
        self.main_thread
            .as_ref()
            .is_some_and(|main| main == &thread::current().id())
    }

    fn is_audio_thread(&self) -> bool {
        IN_CLAP_AUDIO_CALLBACK.with(Cell::get)
    }
}

impl HostGuiImpl for ClapHostShared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, _new_size: GuiSize) -> Result<(), HostError> {
        Err(HostError::Message(
            "Rack does not embed CLAP editor windows",
        ))
    }

    fn request_show(&self) -> Result<(), HostError> {
        self.gui_action.store(1, Ordering::Release);
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        self.gui_action.store(2, Ordering::Release);
        Ok(())
    }

    fn closed(&self, was_destroyed: bool) {
        self.gui_destroyed.store(was_destroyed, Ordering::Release);
        self.gui_closed.store(true, Ordering::Release);
        self.gui_action.store(2, Ordering::Release);
    }
}
