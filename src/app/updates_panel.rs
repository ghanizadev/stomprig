use super::updates::{UpdateController, UpdateStatus};

pub(crate) fn show(ctx: &egui::Context, updates: &mut UpdateController) {
    if !updates.dialog_open() {
        return;
    }

    let status = updates.status().clone();
    let mut open = updates.dialog_open();
    let mut action = UpdatePanelAction::None;
    egui::Window::new("StompRig Updates")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| match &status {
            UpdateStatus::NotInstalled => {
                ui.label("This installation is not managed by Velopack.");
                ui.hyperlink_to(
                    "Open the StompRig releases page",
                    "https://github.com/ghanizadev/stomprig/releases/latest",
                );
            }
            UpdateStatus::Idle => {
                ui.label("Check GitHub for the latest stable StompRig release.");
                action_button(
                    ui,
                    "Check for updates",
                    &mut action,
                    UpdatePanelAction::Check,
                );
            }
            UpdateStatus::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking GitHub releases...");
                });
            }
            UpdateStatus::UpToDate => {
                ui.label("StompRig is up to date.");
                action_button(ui, "Check again", &mut action, UpdatePanelAction::Check);
            }
            UpdateStatus::Available(version) => {
                ui.label(format!("Stable release {version} is available."));
                action_button(
                    ui,
                    "Download update",
                    &mut action,
                    UpdatePanelAction::Download,
                );
            }
            UpdateStatus::Downloading(version) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Downloading stable release {version}..."));
                });
            }
            UpdateStatus::ReadyToRestart(version) => {
                ui.label(format!(
                    "Version {version} is downloaded and ready to install."
                ));
                ui.label("Restarting will stop audio and close plugin editors.");
                action_button(
                    ui,
                    "Restart to install",
                    &mut action,
                    UpdatePanelAction::Restart,
                );
            }
            UpdateStatus::Failed(error) => {
                ui.label(format!("Update check failed: {error}"));
                action_button(ui, "Try again", &mut action, UpdatePanelAction::Check);
            }
        });
    updates.set_dialog_open(open);

    match action {
        UpdatePanelAction::None => {}
        UpdatePanelAction::Check => updates.check_again(ctx),
        UpdatePanelAction::Download => updates.download_available(ctx),
        UpdatePanelAction::Restart => updates.request_restart(ctx),
    }
}

#[derive(Clone, Copy)]
enum UpdatePanelAction {
    None,
    Check,
    Download,
    Restart,
}

fn action_button(
    ui: &mut egui::Ui,
    label: &str,
    action: &mut UpdatePanelAction,
    next_action: UpdatePanelAction,
) {
    if ui.button(label).clicked() {
        *action = next_action;
    }
}
