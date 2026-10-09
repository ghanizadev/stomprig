use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use vst::editor::Editor;
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, RECT},
    Graphics::Gdi::UpdateWindow,
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        AdjustWindowRectEx, CW_USEDEFAULT, CreateWindowExW, DestroyWindow, DispatchMessageW,
        IsWindow, MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SW_SHOW, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, TranslateMessage,
        WM_CLOSE, WM_QUIT, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    },
};

use super::types::RackPlugin;

const EDITOR_THREAD_POLL_INTERVAL: Duration = Duration::from_millis(16);
const FALLBACK_CLIENT_WIDTH: i32 = 640;
const FALLBACK_CLIENT_HEIGHT: i32 = 480;

enum EditorCommand {
    Open,
    Close,
    Shutdown,
}

struct EditorThreadState {
    is_open: AtomicBool,
    open_pending: AtomicBool,
    closed_by_user: AtomicBool,
    shutdown_requested: AtomicBool,
    opening_since: Mutex<Option<Instant>>,
    slow_open_warning_logged: AtomicBool,
    error: Mutex<Option<String>>,
}

struct EditorThreadControl {
    commands: SyncSender<EditorCommand>,
    state: Arc<EditorThreadState>,
}

impl Drop for EditorThreadControl {
    fn drop(&mut self) {
        self.state.shutdown_requested.store(true, Ordering::Release);
        let _ = self.commands.try_send(EditorCommand::Shutdown);
    }
}

/// Owns the VST2 editor and its HWND on a dedicated thread. A plug-in may block forever inside
/// `effEditOpen`; keeping that call away from eframe's thread preserves the Rack UI in that case.
pub(crate) struct WindowsVst2EditorHost {
    title: String,
    editor: Option<Box<dyn Editor + Send>>,
    plugin_owner: Arc<Mutex<RackPlugin>>,
    control: Option<Arc<EditorThreadControl>>,
    startup_error: Option<String>,
}

impl WindowsVst2EditorHost {
    pub(crate) fn new(
        editor: Box<dyn Editor + Send>,
        title: String,
        plugin_owner: Arc<Mutex<RackPlugin>>,
    ) -> Self {
        Self {
            title,
            editor: Some(editor),
            plugin_owner,
            control: None,
            startup_error: None,
        }
    }

    pub(crate) fn open(&mut self) -> Result<(), String> {
        self.start_worker()?;
        let control = self
            .control
            .as_ref()
            .expect("VST2 editor worker should have started");
        if control.state.open_pending.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        control.state.closed_by_user.store(false, Ordering::Release);
        if let Ok(mut opening_since) = control.state.opening_since.lock() {
            *opening_since = Some(Instant::now());
        }
        control
            .state
            .slow_open_warning_logged
            .store(false, Ordering::Release);
        match control.commands.try_send(EditorCommand::Open) {
            Ok(()) => {
                log::info!(
                    "queued VST2 editor open for '{}' on dedicated thread",
                    self.title
                );
                Ok(())
            }
            Err(error) => {
                control.state.open_pending.store(false, Ordering::Release);
                if let Ok(mut opening_since) = control.state.opening_since.lock() {
                    *opening_since = None;
                }
                Err(editor_command_error("open", error))
            }
        }
    }

    pub(crate) fn close(&mut self) {
        let Some(control) = self.control.as_ref() else {
            return;
        };
        control.state.open_pending.store(false, Ordering::Release);
        control.state.closed_by_user.store(false, Ordering::Release);
        if let Ok(mut opening_since) = control.state.opening_since.lock() {
            *opening_since = None;
        }
        if let Err(error) = control.commands.try_send(EditorCommand::Close) {
            log::debug!(
                "could not queue VST2 editor close for '{}': {}",
                self.title,
                editor_command_error("close", error)
            );
        }
    }

    pub(crate) fn service_platform_events(&mut self) -> Result<(), String> {
        let Some(control) = self.control.as_ref() else {
            return Ok(());
        };
        if control.state.open_pending.load(Ordering::Acquire)
            && let Ok(opening_since) = control.state.opening_since.lock()
            && let Some(opening_since) = *opening_since
        {
            let elapsed = opening_since.elapsed();
            if elapsed >= Duration::from_secs(2)
                && !control
                    .state
                    .slow_open_warning_logged
                    .swap(true, Ordering::AcqRel)
            {
                log::warn!(
                    "VST2 effEditOpen for '{}' has not returned after {elapsed:?}; it is isolated on the editor worker thread",
                    self.title
                );
            }
        }
        match control.state.error.lock() {
            Ok(mut error) => error.take().map_or(Ok(()), Err),
            Err(_) => Err("VST2 editor thread state lock poisoned".to_owned()),
        }
    }

    pub(crate) fn closed_by_user(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.state.closed_by_user.load(Ordering::Acquire))
    }

    fn start_worker(&mut self) -> Result<(), String> {
        if self.control.is_some() {
            return Ok(());
        }
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        let Some(editor) = self.editor.take() else {
            return Err("VST2 editor is unavailable".to_owned());
        };

        let (command_sender, command_receiver) = mpsc::sync_channel(8);
        let state = Arc::new(EditorThreadState {
            is_open: AtomicBool::new(false),
            open_pending: AtomicBool::new(false),
            closed_by_user: AtomicBool::new(false),
            shutdown_requested: AtomicBool::new(false),
            opening_since: Mutex::new(None),
            slow_open_warning_logged: AtomicBool::new(false),
            error: Mutex::new(None),
        });
        let worker_state = Arc::clone(&state);
        let panic_state = Arc::clone(&state);
        let title = self.title.clone();
        let plugin_owner = Arc::clone(&self.plugin_owner);
        let spawn_result = thread::Builder::new()
            .name("rack-vst2-editor".to_owned())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    editor_thread_loop(editor, title, plugin_owner, command_receiver, worker_state);
                }));
                if result.is_err() {
                    log::error!("VST2 editor worker panicked");
                    panic_state.open_pending.store(false, Ordering::Release);
                    panic_state.is_open.store(false, Ordering::Release);
                    if let Ok(mut opening_since) = panic_state.opening_since.lock() {
                        *opening_since = None;
                    }
                    store_editor_error(&panic_state, "VST2 editor worker panicked".to_owned());
                }
            });
        if let Err(error) = spawn_result {
            let message = format!("could not start VST2 editor thread: {error}");
            self.startup_error = Some(message.clone());
            return Err(message);
        }

        self.control = Some(Arc::new(EditorThreadControl {
            commands: command_sender,
            state,
        }));
        Ok(())
    }
}

fn editor_command_error(action: &str, error: TrySendError<EditorCommand>) -> String {
    match error {
        TrySendError::Full(_) => format!("VST2 editor thread is busy; could not queue {action}"),
        TrySendError::Disconnected(_) => {
            format!("VST2 editor thread stopped before {action}")
        }
    }
}

fn editor_thread_loop(
    mut editor: Box<dyn Editor + Send>,
    title: String,
    _plugin_owner: Arc<Mutex<RackPlugin>>,
    commands: mpsc::Receiver<EditorCommand>,
    state: Arc<EditorThreadState>,
) {
    let _ole_apartment = match super::windows_sta::WindowsStaApartment::initialize() {
        Ok(apartment) => apartment,
        Err(error) => {
            store_editor_error(&state, error);
            state.open_pending.store(false, Ordering::Release);
            return;
        }
    };
    let mut window = Vst2NativeWindow::new(title);
    log::info!(
        "VST2 editor worker started with OLE STA on thread {:?}",
        thread::current().id()
    );

    while !state.shutdown_requested.load(Ordering::Acquire) {
        match commands.recv_timeout(EDITOR_THREAD_POLL_INTERVAL) {
            Ok(command) => {
                service_editor_command(command, &mut window, &mut editor, &state);
                while let Ok(command) = commands.try_recv() {
                    service_editor_command(command, &mut window, &mut editor, &state);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if window.pump_messages() {
            window.close(&mut editor);
            state.is_open.store(false, Ordering::Release);
            state.open_pending.store(false, Ordering::Release);
            state.closed_by_user.store(true, Ordering::Release);
        }

        if window.is_open() {
            let idle_started = Instant::now();
            editor.idle();
            let elapsed = idle_started.elapsed();
            if elapsed >= Duration::from_millis(50) {
                log::warn!("VST2 effEditIdle took {elapsed:?} on the editor worker thread");
            }
        }
    }

    window.close(&mut editor);
    state.is_open.store(false, Ordering::Release);
    state.open_pending.store(false, Ordering::Release);
    log::info!("VST2 editor worker stopped");
}

fn service_editor_command(
    command: EditorCommand,
    window: &mut Vst2NativeWindow,
    editor: &mut Box<dyn Editor + Send>,
    state: &EditorThreadState,
) {
    match command {
        EditorCommand::Open => {
            match window.open(editor) {
                Ok(()) => {
                    state.is_open.store(true, Ordering::Release);
                    state.closed_by_user.store(false, Ordering::Release);
                }
                Err(error) => {
                    state.is_open.store(false, Ordering::Release);
                    store_editor_error(state, error);
                }
            }
            state.open_pending.store(false, Ordering::Release);
            if let Ok(mut opening_since) = state.opening_since.lock() {
                *opening_since = None;
            }
        }
        EditorCommand::Close => {
            window.close(editor);
            state.is_open.store(false, Ordering::Release);
        }
        EditorCommand::Shutdown => {
            state.shutdown_requested.store(true, Ordering::Release);
        }
    }
}

fn store_editor_error(state: &EditorThreadState, message: String) {
    if let Ok(mut error) = state.error.lock() {
        *error = Some(message);
    }
}

struct Vst2NativeWindow {
    title: String,
    hwnd: Option<isize>,
}

impl Vst2NativeWindow {
    fn new(title: String) -> Self {
        Self { title, hwnd: None }
    }

    fn open(&mut self, editor: &mut Box<dyn Editor + Send>) -> Result<(), String> {
        if let Some(hwnd) = self.hwnd
            && unsafe { IsWindow(hwnd as _) != 0 }
        {
            unsafe {
                ShowWindow(hwnd as _, SW_SHOW);
                SetForegroundWindow(hwnd as _);
            }
            return Ok(());
        }
        self.hwnd = None;

        let open_started = Instant::now();
        log::info!(
            "starting VST2 editor open for '{}' on worker thread {:?}; fallback client size is {FALLBACK_CLIENT_WIDTH}x{FALLBACK_CLIENT_HEIGHT}",
            self.title,
            thread::current().id()
        );
        let class_name = wide("rack-vst2-editor");
        let window_title = wide(&format!("{} - VST2", self.title));
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        let class = WNDCLASSW {
            lpfnWndProc: Some(vst2_window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS {
            return Err("Could not register the VST2 editor window class".to_owned());
        }

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: FALLBACK_CLIENT_WIDTH,
            bottom: FALLBACK_CLIENT_HEIGHT,
        };
        unsafe {
            AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            log::error!(
                "CreateWindowExW failed for VST2 editor '{}' after {:?}",
                self.title,
                open_started.elapsed()
            );
            return Err("Failed to create VST2 editor window".to_owned());
        }
        self.hwnd = Some(hwnd as isize);
        log::info!(
            "created VST2 host HWND {:#x} for '{}' after {:?}",
            hwnd as usize,
            self.title,
            open_started.elapsed()
        );

        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            UpdateWindow(hwnd);
        }
        log::info!(
            "calling VST2 effEditOpen for '{}' with host HWND {:#x} on worker thread",
            self.title,
            hwnd as usize
        );
        let edit_open_started = Instant::now();
        if !editor.open(hwnd) {
            log::error!(
                "VST2 effEditOpen returned false for '{}' after {:?}",
                self.title,
                edit_open_started.elapsed()
            );
            unsafe { DestroyWindow(hwnd) };
            self.hwnd = None;
            return Err("VST2 plugin refused to open its editor".to_owned());
        }
        log::info!(
            "VST2 effEditOpen returned true for '{}' after {:?}",
            self.title,
            edit_open_started.elapsed()
        );

        // Some VST2 plugins only report a valid editor rectangle after effEditOpen.
        let size_started = Instant::now();
        let (width, height) = editor.size();
        log::info!(
            "VST2 effEditGetRect returned for '{}' with {width}x{height} after {:?}",
            self.title,
            size_started.elapsed()
        );
        if width > 0 && height > 0 {
            let mut editor_rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            unsafe {
                AdjustWindowRectEx(&mut editor_rect, WS_OVERLAPPEDWINDOW, 0, 0);
            }
            let resized = unsafe {
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    0,
                    0,
                    editor_rect.right - editor_rect.left,
                    editor_rect.bottom - editor_rect.top,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            if resized == 0 {
                log::warn!(
                    "SetWindowPos failed for VST2 editor '{}' at requested client size {width}x{height}; keeping fallback window size",
                    self.title
                );
            }
        } else {
            log::warn!(
                "VST2 plugin '{}' returned invalid editor size {width}x{height}; keeping fallback window size",
                self.title
            );
        }

        log::info!(
            "VST2 editor open completed for '{}' in {:?}",
            self.title,
            open_started.elapsed()
        );
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.hwnd
            .is_some_and(|hwnd| unsafe { IsWindow(hwnd as _) != 0 })
    }

    /// Returns true when a user close or destruction of the host HWND was observed.
    fn pump_messages(&mut self) -> bool {
        let Some(hwnd) = self.hwnd else {
            return false;
        };
        let mut close_requested = false;
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT
                    || (message.message == WM_CLOSE && message.hwnd == hwnd as _)
                {
                    close_requested = true;
                    continue;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        close_requested || !self.is_open()
    }

    fn close(&mut self, editor: &mut Box<dyn Editor + Send>) {
        let close_started = Instant::now();
        if editor.is_open() {
            log::info!(
                "calling VST2 effEditClose for '{}' on editor thread",
                self.title
            );
            editor.close();
            log::info!(
                "VST2 effEditClose returned for '{}' after {:?}",
                self.title,
                close_started.elapsed()
            );
        }
        if let Some(hwnd) = self.hwnd.take()
            && unsafe { IsWindow(hwnd as _) != 0 }
        {
            unsafe { DestroyWindow(hwnd as _) };
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn vst2_window_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, message, wparam, lparam)
    }
}
