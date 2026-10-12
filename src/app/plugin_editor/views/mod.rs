mod parameter_controls;
mod parameter_layout;
mod parameters;
mod playback;
mod rack;
mod tuner;

pub(super) struct PluginUiRequests<'a> {
    pub(super) edits: &'a mut Vec<(usize, u32, f64)>,
    pub(super) remove_requested: &'a mut Option<usize>,
    pub(super) parameter_learn_requested: &'a mut Option<(usize, u32)>,
    pub(super) parameter_learn_cancel_requested: &'a mut bool,
    pub(super) parameter_mapping_remove_requested: &'a mut Vec<(usize, u32)>,
}

pub(super) struct PluginParameterUiData {
    id: u32,
    name: String,
    is_read_only: bool,
    is_learning: bool,
    midi_mapping_label: Option<String>,
}
