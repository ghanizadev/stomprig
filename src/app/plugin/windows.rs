use std::sync::{Arc, Mutex};
use vst::editor::Editor;
#[cfg(not(windows))]
use vst3_host::PluginWindow;

#[cfg(windows)]
use super::{windows_vst2_host::WindowsVst2EditorHost, windows_vst3_host::WindowsVst3EditorHost};

pub(crate) enum EditorWindow {
    #[cfg(not(windows))]
    Vst3(PluginWindow),
    #[cfg(windows)]
    Vst3(WindowsVst3EditorHost),
    Vst2(Vst2EditorWindow),
    None,
}

#[cfg(windows)]
type Vst2EditorWindow = WindowsVst2EditorHost;

#[cfg(not(windows))]
pub(crate) struct Vst2EditorWindow {
    editor: Option<Box<dyn Editor + Send>>,
    title: String,
    _plugin_owner: Arc<Mutex<super::types::RackPlugin>>,
}

impl EditorWindow {
    pub(crate) fn new(
        plugin: &super::RackPlugin,
        plugin_owner: Arc<Mutex<super::RackPlugin>>,
        vst2_editor: Option<Box<dyn Editor + Send>>,
    ) -> Self {
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
        } else if let Some(editor) = vst2_editor {
            Self::Vst2(Vst2EditorWindow::new(
                editor,
                plugin.info().name,
                plugin_owner,
            ))
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

#[cfg(not(windows))]
impl Vst2EditorWindow {
    fn new(
        editor: Box<dyn Editor + Send>,
        title: String,
        plugin_owner: Arc<Mutex<super::types::RackPlugin>>,
    ) -> Self {
        Self {
            editor: Some(editor),
            title,
            _plugin_owner: plugin_owner,
        }
    }

    fn open(&mut self) -> Result<(), String> {
        Err(format!(
            "VST2 native editors are only supported on Windows ({})",
            self.title
        ))
    }

    fn close(&mut self) {
        if let Some(editor) = self.editor.as_mut()
            && editor.is_open()
        {
            editor.close();
        }
    }

    fn service_platform_events(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn closed_by_user(&self) -> bool {
        false
    }
}

#[cfg(not(windows))]
impl Drop for Vst2EditorWindow {
    fn drop(&mut self) {
        self.close();
    }
}
