#[cfg(windows)]
use std::ffi::c_void;

use vst::editor::Editor;
use vst3_host::PluginWindow;

pub(crate) enum EditorWindow {
    Vst3(PluginWindow),
    Vst2(Vst2EditorWindow),
    None,
}

impl EditorWindow {
    pub(crate) fn new(plugin: &super::RackPlugin, vst2_editor: Option<Box<dyn Editor>>) -> Self {
        if let Some(window) = plugin.vst3_window() {
            Self::Vst3(window)
        } else if let Some(editor) = vst2_editor {
            Self::Vst2(Vst2EditorWindow::new(editor, plugin.info().name))
        } else {
            Self::None
        }
    }

    pub(crate) fn open(&mut self) -> Result<(), String> {
        match self {
            Self::Vst3(window) => window.open().map_err(|error| error.to_string()),
            Self::Vst2(window) => window.open(),
            Self::None => Err("Plugin does not have a GUI editor".to_owned()),
        }
    }

    pub(crate) fn close(&mut self) {
        match self {
            Self::Vst3(window) => window.close(),
            Self::Vst2(window) => window.close(),
            Self::None => {}
        }
    }

    pub(crate) fn service_platform_events(&mut self) -> Result<(), String> {
        match self {
            Self::Vst3(window) => window
                .service_platform_events()
                .map_err(|error| error.to_string()),
            Self::Vst2(window) => window.service_platform_events(),
            Self::None => Ok(()),
        }
    }

    pub(crate) fn closed_by_user(&self) -> bool {
        match self {
            Self::Vst3(window) => window.closed_by_user(),
            Self::Vst2(window) => window.closed_by_user(),
            Self::None => false,
        }
    }
}

#[cfg(windows)]
pub(crate) struct Vst2EditorWindow {
    editor: Option<Box<dyn Editor>>,
    title: String,
    hwnd: Option<isize>,
}

#[cfg(not(windows))]
pub(crate) struct Vst2EditorWindow {
    editor: Option<Box<dyn Editor>>,
    title: String,
}

impl std::fmt::Debug for Vst2EditorWindow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Vst2EditorWindow")
            .field("title", &self.title)
            .finish()
    }
}

impl Vst2EditorWindow {
    fn new(editor: Box<dyn Editor>, title: String) -> Self {
        #[cfg(windows)]
        {
            Self {
                editor: Some(editor),
                title,
                hwnd: None,
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                editor: Some(editor),
                title,
            }
        }
    }

    fn open(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::ptr;
            use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                AdjustWindowRectEx, CreateWindowExW, RegisterClassW, SW_SHOW, ShowWindow,
                WNDCLASSW, WS_OVERLAPPEDWINDOW,
            };

            if self.hwnd.is_some() {
                self.close();
            }
            let editor = self
                .editor
                .as_mut()
                .ok_or_else(|| "VST2 editor is unavailable".to_owned())?;
            let (width, height) = editor.size();
            let width = width.max(1);
            let height = height.max(1);
            let class_name = wide("rack-vst2-editor");
            let title = wide(&format!("{} - VST2", self.title));
            let instance = unsafe { GetModuleHandleW(ptr::null()) };
            let class = WNDCLASSW {
                lpfnWndProc: Some(vst2_window_proc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..unsafe { std::mem::zeroed() }
            };
            unsafe {
                RegisterClassW(&class);
            }
            let mut rect = windows_sys::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            unsafe {
                AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0);
            }
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    title.as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    0x80000000u32 as i32,
                    0x80000000u32 as i32,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    instance,
                    ptr::null(),
                )
            };
            if hwnd.is_null() {
                return Err("Failed to create VST2 editor window".to_owned());
            }
            if !editor.open(hwnd as *mut c_void) {
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd) };
                return Err("VST2 plugin refused to open its editor".to_owned());
            }
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
            }
            self.hwnd = Some(hwnd as isize);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Err("VST2 native editors are only supported on Windows".to_owned())
        }
    }

    fn close(&mut self) {
        if let Some(editor) = self.editor.as_mut()
            && editor.is_open()
        {
            editor.close();
        }
        #[cfg(windows)]
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd as _);
            }
        }
    }

    fn service_platform_events(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        if self.hwnd.is_some() {
            if let Some(editor) = self.editor.as_mut() {
                editor.idle();
            }
        }
        Ok(())
    }

    fn closed_by_user(&self) -> bool {
        #[cfg(windows)]
        if let Some(hwnd) = self.hwnd {
            return unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd as _) == 0
            };
        }
        false
    }
}

impl Drop for Vst2EditorWindow {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
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
