use super::super::midi::PresetMidiMapping;
use super::super::plugin::{
    ClapEffect, EditorWindow, ParameterChangeConsumer, ParameterRuntimeFeedbackProducer, RackPlugin,
};
use super::super::plugin_runtime::PluginSnapshot;
use super::parameter_worker::PluginParameterWorker;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub(super) struct PluginSlot {
    pub(super) path: String,
    pub(super) instance: Option<Arc<Mutex<RackPlugin>>>,
    pub(super) clap: Option<ClapEffect>,
    pub(super) editor_window: EditorWindow,
    pub(super) snapshot: PluginSnapshot,
    #[cfg(target_os = "macos")]
    pub(super) last_recovery_count: u64,
    pub(super) parameter_worker: Option<PluginParameterWorker>,
    pub(super) parameter_changes: Option<ParameterChangeConsumer>,
    pub(super) parameter_feedback: Option<ParameterRuntimeFeedbackProducer>,
    pub(super) bypass_state: Arc<AtomicBool>,
    pub(super) midi_mappings: Vec<PresetMidiMapping>,
    pub(super) collapsed: bool,
}

impl PluginSlot {
    pub(super) fn close_editor(&mut self) {
        self.editor_window.close();
        if let Some(plugin) = &mut self.clap {
            plugin.close_editor();
        }
    }
}

#[derive(Default)]
pub(super) struct PluginRack {
    pub(super) slots: Vec<PluginSlot>,
}
