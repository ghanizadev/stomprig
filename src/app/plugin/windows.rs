#[cfg(not(windows))]
use vst3_host::PluginWindow;

#[cfg(windows)]
use super::windows_vst3_host::WindowsVst3EditorHost;

pub(crate) enum EditorWindow {
    #[cfg(not(windows))]
    Vst3(PluginWindow),
    #[cfg(windows)]
    Vst3(WindowsVst3EditorHost),
    None,
}

impl EditorWindow {
    pub(crate) fn new(plugin: &super::RackPlugin) -> Self {
        #[cfg(windows)]
        let vst3_window = if plugin.has_editor() {
            plugin.vst3_editor_host()
        } else {
            None
        };
        #[cfg(not(windows))]
        let vst3_window = plugin.vst3_window();

        if let Some(window) = vst3_window {
            Self::Vst3(window)
        } else {
            Self::None
        }
    }

    pub(crate) fn open(&mut self) -> Result<(), String> {
        match self {
            Self::Vst3(window) => window.open().map_err(|error| error.to_string()),
            Self::None => Err("Plugin does not have a GUI editor".to_owned()),
        }
    }

    pub(crate) fn close(&mut self) {
        match self {
            Self::Vst3(window) => window.close(),
            Self::None => {}
        }
    }

    pub(crate) fn service_platform_events(&mut self) -> Result<(), String> {
        match self {
            Self::Vst3(window) => window
                .service_platform_events()
                .map_err(|error| error.to_string()),
            Self::None => Ok(()),
        }
    }

    pub(crate) fn closed_by_user(&self) -> bool {
        match self {
            Self::Vst3(window) => window.closed_by_user(),
            Self::None => false,
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn is_open(&self) -> bool {
        match self {
            Self::Vst3(window) => window.is_open(),
            Self::None => false,
        }
    }
}
