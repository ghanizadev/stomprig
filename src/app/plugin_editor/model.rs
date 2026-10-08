use super::super::midi::PresetMidiMapping;
use super::super::plugin::{EditorWindow, RackPlugin};
use super::super::plugin_runtime::PluginSnapshot;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub(super) struct PluginSlot {
    pub(super) path: String,
    pub(super) instance: Arc<Mutex<RackPlugin>>,
    pub(super) editor_window: EditorWindow,
    pub(super) snapshot: PluginSnapshot,
    pub(super) bypass_state: Arc<AtomicBool>,
    pub(super) midi_mappings: Vec<PresetMidiMapping>,
    pub(super) collapsed: bool,
}

#[derive(Default)]
pub(super) struct PluginRack {
    pub(super) slots: Vec<PluginSlot>,
}
