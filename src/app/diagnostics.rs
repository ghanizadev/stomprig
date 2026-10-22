//! Crash diagnostics, file logging, and lock-free audio/runtime incident transport.

mod events;
mod logging;

pub(crate) use events::{
    DiagnosticConsumer, DiagnosticEvent, DiagnosticHistory, DiagnosticKind, DiagnosticProducer,
    DiagnosticSeverity, EVENT_QUEUE_CAPACITY, FixedText, SUBJECT_BYTES, channel,
    format_timestamp_utc,
};
pub use logging::StartupReport;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Last known stage of the audio callback / plugin pipeline.
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum Stage {
    Idle = 0,
    CallbackEnter = 1,
    RoutingInput = 2,
    PluginLock = 3,
    PluginProcess = 4,
    RoutingOutput = 5,
    CallbackExit = 6,
}

impl Stage {
    fn from_u32(value: u32) -> &'static str {
        match value {
            0 => "Idle",
            1 => "CallbackEnter",
            2 => "RoutingInput",
            3 => "PluginLock",
            4 => "PluginProcess",
            5 => "RoutingOutput",
            6 => "CallbackExit",
            _ => "Unknown",
        }
    }
}

static AUDIO_STAGE: AtomicU32 = AtomicU32::new(0);
static AUDIO_PLUGIN_INDEX: AtomicU32 = AtomicU32::new(u32::MAX);
static AUDIO_FRAMES: AtomicU32 = AtomicU32::new(0);
static AUDIO_CALLBACKS: AtomicU64 = AtomicU64::new(0);

/// Record where the audio thread is. Lock-free and allocation-free.
pub fn audio_stage(stage: Stage) {
    AUDIO_STAGE.store(stage as u32, Ordering::Relaxed);
}

pub fn audio_plugin(index: usize) {
    AUDIO_PLUGIN_INDEX.store(index as u32, Ordering::Relaxed);
}

pub fn audio_callback(frames: usize) {
    AUDIO_FRAMES.store(frames as u32, Ordering::Relaxed);
    AUDIO_CALLBACKS.fetch_add(1, Ordering::Relaxed);
}

fn breadcrumb_summary() -> String {
    format!(
        "audio stage={} plugin_index={} frames={} callbacks={}",
        Stage::from_u32(AUDIO_STAGE.load(Ordering::Relaxed)),
        AUDIO_PLUGIN_INDEX.load(Ordering::Relaxed) as i32,
        AUDIO_FRAMES.load(Ordering::Relaxed),
        AUDIO_CALLBACKS.load(Ordering::Relaxed),
    )
}

/// Directory used for `stomprig.log`, `crash.log` and `crash.dmp`.
pub fn log_dir() -> PathBuf {
    logging::log_dir()
}

/// Initialize logging and crash capture before constructing the UI.
pub fn init() -> StartupReport {
    let report = logging::init();
    let dir = report.log_dir.clone();

    install_panic_hook(dir.clone());
    #[cfg(windows)]
    native::install(dir);
    log::info!(
        "StompRig {} started, pid {}, logs in {}",
        super::APPLICATION_VERSION,
        std::process::id(),
        report.log_dir.display()
    );
    report
}

/// Remove the active-run marker after the application exits normally.
pub fn mark_clean_shutdown() {
    logging::mark_clean_shutdown();
}

fn install_panic_hook(dir: PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let message = format!(
            "PANIC on thread '{}': {info}\n{}\n{}\n",
            thread.name().unwrap_or("<unnamed>"),
            std::backtrace::Backtrace::force_capture(),
            breadcrumb_summary(),
        );
        log::error!("{message}");
        if let Err(error) = logging::append_crash_log(&dir, message.as_bytes()) {
            logging::report_crash_write_failure("crash.log", &error);
        }
    }));
}

#[cfg(windows)]
mod native {
    use super::breadcrumb_summary;
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::EXCEPTION_STACK_OVERFLOW;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION, MiniDumpWithIndirectlyReferencedMemory,
        MiniDumpWithThreadInfo, MiniDumpWriteDump, SetUnhandledExceptionFilter,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
    };

    static DIR: OnceLock<PathBuf> = OnceLock::new();
    const EXCEPTION_EXECUTE_HANDLER: i32 = 1;

    pub fn install(dir: PathBuf) {
        let _ = DIR.set(dir);
        // Plugins are native code; an access violation never reaches the panic hook.
        unsafe { SetUnhandledExceptionFilter(Some(filter)) };
    }

    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        let Some(dir) = DIR.get() else {
            return EXCEPTION_EXECUTE_HANDLER;
        };
        let (code, address) = unsafe {
            info.as_ref()
                .and_then(|i| i.ExceptionRecord.as_ref())
                .map(|r| (r.ExceptionCode, r.ExceptionAddress as usize))
                .unwrap_or((0, 0))
        };
        let mut text = format!(
            "NATIVE CRASH: exception 0x{:08X} at 0x{address:016X}, thread {}\n{}\n",
            code as u32,
            unsafe { GetCurrentThreadId() },
            breadcrumb_summary()
        );
        if code == EXCEPTION_STACK_OVERFLOW {
            text.push_str("(stack overflow)\n");
        }
        if let Err(error) = super::logging::append_crash_log(dir, text.as_bytes()) {
            super::logging::report_crash_write_failure("crash.log", &error);
        }
        if let Err(error) = super::logging::rotate_dump(dir) {
            super::logging::report_crash_write_failure("crash.dmp rotation", &error);
        } else {
            match File::create(dir.join("crash.dmp")) {
                Ok(file) => {
                    let exception = MINIDUMP_EXCEPTION_INFORMATION {
                        ThreadId: unsafe { GetCurrentThreadId() },
                        ExceptionPointers: info as *mut EXCEPTION_POINTERS,
                        ClientPointers: 0,
                    };
                    unsafe {
                        MiniDumpWriteDump(
                            GetCurrentProcess(),
                            GetCurrentProcessId(),
                            file.as_raw_handle() as _,
                            MiniDumpWithIndirectlyReferencedMemory | MiniDumpWithThreadInfo,
                            &exception,
                            std::ptr::null(),
                            std::ptr::null(),
                        );
                    }
                }
                Err(error) => super::logging::report_crash_write_failure("crash.dmp", &error),
            }
        }
        EXCEPTION_EXECUTE_HANDLER
    }
}
