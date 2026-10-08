use super::super::plugin::service_run_loop;
use super::PluginEditor;

impl PluginEditor {
    pub(super) fn service_editor_windows(&mut self) {
        for index in 0..self.rack.slots.len() {
            let slot = &mut self.rack.slots[index];
            if let Err(error) = slot.editor_window.service_platform_events() {
                self.state = super::EditorState::Error;
                self.error = Some(format!(
                    "Could not service {} editor: {error}",
                    slot.snapshot.name
                ));
            }
            if slot.editor_window.closed_by_user() {
                slot.editor_window.close();
            }
            service_run_loop(&slot.instance);
        }
    }
}
