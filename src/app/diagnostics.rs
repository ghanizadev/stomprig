//! Crash diagnostics: file logging, panic hook, native crash handler with minidump,
//! and lock-free breadcrumbs that are safe to update from the audio thread.

use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger,
};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

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

/// Directory used for `rack.log`, `crash.log` and `crash.dmp`.
pub fn log_dir() -> PathBuf {
    LOG_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("logs"))
}

/// Call once at the start of `main`. Level can be overridden with `RACK_LOG`.
pub fn init() {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("logs")))
        .unwrap_or_else(|| PathBuf::from("logs"));
    let _ = fs::create_dir_all(&dir);
    let _ = LOG_DIR.set(dir.clone());

    let level = std::env::var("RACK_LOG")
        .ok()
        .and_then(|value| value.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Debug);
    let config = ConfigBuilder::new()
        .set_thread_level(LevelFilter::Error)
        .set_thread_mode(simplelog::ThreadLogMode::Both)
        .set_location_level(LevelFilter::Error)
        .set_time_format_rfc3339()
        .build();

    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
        level,
        config.clone(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )];
    if let Ok(file) = File::create(dir.join("rack.log")) {
        loggers.push(WriteLogger::new(level, config, SyncFile(file)));
    }
    let _ = CombinedLogger::init(loggers);

    install_panic_hook(dir.clone());
    #[cfg(windows)]
    native::install(dir.clone());
    log::info!(
        "rack {} started, pid {}, logs in {}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        dir.display()
    );
}

/// Flushes after every write so nothing is lost when the process dies.
struct SyncFile(File);

impl Write for SyncFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.0.write(buf)?;
        let _ = self.0.sync_data();
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
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
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("crash.log"))
        {
            let _ = file.write_all(message.as_bytes());
            let _ = file.sync_all();
        }
    }));
}

#[cfg(windows)]
mod native {
    use super::breadcrumb_summary;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
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
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("crash.log"))
        {
            let _ = file.write_all(text.as_bytes());
            let _ = file.sync_all();
        }
        if let Ok(file) = File::create(dir.join("crash.dmp")) {
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
        EXCEPTION_EXECUTE_HANDLER
    }
}
