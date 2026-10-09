use std::ptr;

use windows_sys::Win32::System::Ole::{OleInitialize, OleUninitialize};

/// Initializes OLE's single-threaded apartment for a Windows thread that owns plug-in UI.
pub(crate) struct WindowsStaApartment;

impl WindowsStaApartment {
    pub(crate) fn initialize() -> Result<Self, String> {
        let result = unsafe { OleInitialize(ptr::null_mut()) };
        if result < 0 {
            return Err(format!(
                "OleInitialize failed with HRESULT 0x{:08X}",
                result as u32
            ));
        }
        log::debug!(
            "initialized Windows OLE STA on thread {:?}",
            std::thread::current().id()
        );
        Ok(Self)
    }
}

impl Drop for WindowsStaApartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}
