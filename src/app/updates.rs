use semver::Version;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use velopack::sources::GithubSource;
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, VelopackApp, VelopackAsset};

const GITHUB_RELEASES_URL: &str = "https://github.com/ghanizadev/stomprig";

pub fn initialize_velopack() {
    VelopackApp::build().set_auto_apply_on_startup(false).run();
}

pub fn create_update_manager() -> Result<AppUpdater, String> {
    let source = GithubSource::new(GITHUB_RELEASES_URL, None, false);
    let manager = UpdateManager::new(source, None, None).map_err(|error| error.to_string())?;
    Ok(AppUpdater {
        backend: Arc::new(VelopackUpdateBackend { manager }),
    })
}

pub fn apply_update(updater: &AppUpdater, update: &PendingUpdate) -> Result<(), String> {
    updater.backend.apply_update(&update.asset)
}

#[derive(Clone)]
pub struct AppUpdater {
    backend: Arc<dyn UpdateBackend>,
}

pub struct PendingUpdate {
    asset: VelopackAsset,
}

trait UpdateBackend: Send + Sync {
    fn check_for_updates(&self) -> Result<UpdateCheck, String>;
    fn download_updates(&self, update: &UpdateInfo) -> Result<(), String>;
    fn current_version(&self) -> Version;
    fn pending_update(&self) -> Option<VelopackAsset>;
    fn apply_update(&self, update: &VelopackAsset) -> Result<(), String>;
}

struct VelopackUpdateBackend {
    manager: UpdateManager,
}

impl UpdateBackend for VelopackUpdateBackend {
    fn check_for_updates(&self) -> Result<UpdateCheck, String> {
        self.manager
            .check_for_updates()
            .map_err(|error| error.to_string())
    }

    fn download_updates(&self, update: &UpdateInfo) -> Result<(), String> {
        self.manager
            .download_updates(update, None)
            .map_err(|error| error.to_string())
    }

    fn current_version(&self) -> Version {
        self.manager.get_current_version()
    }

    fn pending_update(&self) -> Option<VelopackAsset> {
        self.manager.get_update_pending_restart()
    }

    fn apply_update(&self, update: &VelopackAsset) -> Result<(), String> {
        self.manager
            .apply_updates_and_restart(update)
            .map_err(|error| error.to_string())
    }
}

impl AppUpdater {
    fn pending_update(&self) -> Option<PendingUpdate> {
        self.backend
            .pending_update()
            .map(|asset| PendingUpdate { asset })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateStatus {
    NotInstalled,
    Idle,
    Checking,
    UpToDate,
    Available(String),
    Downloading(String),
    ReadyToRestart(String),
    Failed(String),
}

enum UpdateWorkerMessage {
    CheckFinished(Result<Option<UpdateInfo>, String>),
    DownloadFinished(Result<UpdateInfo, String>),
}

pub(crate) struct UpdateController {
    updater: Option<AppUpdater>,
    status: UpdateStatus,
    dialog_open: bool,
    startup_check_started: bool,
    prompt_when_update_found: bool,
    available_update: Option<UpdateInfo>,
    staged_update: Option<PendingUpdate>,
    restart_requested: bool,
    pending_update_sender: Sender<PendingUpdate>,
    worker_sender: Sender<UpdateWorkerMessage>,
    worker_receiver: Receiver<UpdateWorkerMessage>,
}

impl UpdateController {
    pub(crate) fn new(
        updater: Option<AppUpdater>,
        pending_update_sender: Sender<PendingUpdate>,
    ) -> Self {
        let (worker_sender, worker_receiver) = mpsc::channel();
        let mut controller = Self {
            status: if updater.is_some() {
                UpdateStatus::Idle
            } else {
                UpdateStatus::NotInstalled
            },
            updater,
            dialog_open: false,
            startup_check_started: false,
            prompt_when_update_found: false,
            available_update: None,
            staged_update: None,
            restart_requested: false,
            pending_update_sender,
            worker_sender,
            worker_receiver,
        };
        controller.restore_staged_update();
        controller
    }

    pub(crate) fn check_on_startup(&mut self, ctx: &egui::Context) {
        if self.startup_check_started {
            return;
        }
        self.startup_check_started = true;
        if self.updater.is_some() {
            self.start_check(ctx, true);
        }
    }

    pub(crate) fn open_and_check(&mut self, ctx: &egui::Context) {
        self.dialog_open = true;
        if matches!(
            &self.status,
            UpdateStatus::Idle | UpdateStatus::UpToDate | UpdateStatus::Failed(_)
        ) {
            self.start_check(ctx, false);
        }
    }

    pub(crate) fn check_again(&mut self, ctx: &egui::Context) {
        self.start_check(ctx, false);
    }

    pub(crate) fn download_available(&mut self, ctx: &egui::Context) {
        let (Some(updater), Some(update)) = (self.updater.as_ref(), self.available_update.take())
        else {
            return;
        };
        let updater = updater.clone();
        let version = update.TargetFullRelease.Version.clone();
        let sender = self.worker_sender.clone();
        let repaint_context = ctx.clone();
        self.status = UpdateStatus::Downloading(version);
        thread::spawn(move || {
            let result = updater.backend.download_updates(&update).map(|()| update);
            let _ = sender.send(UpdateWorkerMessage::DownloadFinished(result));
            repaint_context.request_repaint();
        });
    }

    pub(crate) fn request_restart(&mut self, ctx: &egui::Context) {
        if self.staged_update.is_some() {
            self.restart_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(crate) fn status(&self) -> &UpdateStatus {
        &self.status
    }

    pub(crate) fn dialog_open(&self) -> bool {
        self.dialog_open
    }

    pub(crate) fn set_dialog_open(&mut self, open: bool) {
        self.dialog_open = open;
    }

    pub(crate) fn on_exit(&mut self) {
        if !self.restart_requested {
            return;
        }
        let Some(update) = self.staged_update.take() else {
            return;
        };
        if self.pending_update_sender.send(update).is_err() {
            log::error!("could not hand the downloaded update to the application entry point");
        }
    }

    pub(crate) fn poll(&mut self, ctx: &egui::Context) {
        loop {
            match self.worker_receiver.try_recv() {
                Ok(UpdateWorkerMessage::CheckFinished(Ok(Some(update)))) => {
                    let version = update.TargetFullRelease.Version.clone();
                    self.available_update = Some(update);
                    self.status = UpdateStatus::Available(version);
                    if self.prompt_when_update_found {
                        self.dialog_open = true;
                    }
                    self.prompt_when_update_found = false;
                }
                Ok(UpdateWorkerMessage::CheckFinished(Ok(None))) => {
                    self.status = UpdateStatus::UpToDate;
                    self.prompt_when_update_found = false;
                }
                Ok(UpdateWorkerMessage::CheckFinished(Err(error))) => {
                    log::warn!("could not check for StompRig updates: {error}");
                    self.status = UpdateStatus::Failed(error);
                    self.prompt_when_update_found = false;
                }
                Ok(UpdateWorkerMessage::DownloadFinished(Ok(update))) => {
                    let version = update.TargetFullRelease.Version.clone();
                    self.staged_update = Some(PendingUpdate {
                        asset: update.TargetFullRelease.clone(),
                    });
                    self.status = UpdateStatus::ReadyToRestart(version);
                    self.dialog_open = true;
                }
                Ok(UpdateWorkerMessage::DownloadFinished(Err(error))) => {
                    log::warn!("could not download the StompRig update: {error}");
                    self.status = UpdateStatus::Failed(error);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if self.dialog_open {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn restore_staged_update(&mut self) {
        let Some(update) = self.updater.as_ref().and_then(AppUpdater::pending_update) else {
            return;
        };
        let version = update.asset.Version.clone();
        self.staged_update = Some(update);
        self.status = UpdateStatus::ReadyToRestart(version);
        self.dialog_open = true;
    }

    fn start_check(&mut self, ctx: &egui::Context, prompt_when_update_found: bool) {
        let Some(updater) = self.updater.as_ref().cloned() else {
            self.status = UpdateStatus::NotInstalled;
            return;
        };
        if matches!(
            &self.status,
            UpdateStatus::Checking | UpdateStatus::Downloading(_)
        ) {
            return;
        }
        self.status = UpdateStatus::Checking;
        self.prompt_when_update_found = prompt_when_update_found;
        let sender = self.worker_sender.clone();
        let repaint_context = ctx.clone();
        thread::spawn(move || {
            let result = check_for_stable_update(&updater);
            let _ = sender.send(UpdateWorkerMessage::CheckFinished(result));
            repaint_context.request_repaint();
        });
    }
}

fn check_for_stable_update(updater: &AppUpdater) -> Result<Option<UpdateInfo>, String> {
    match updater.backend.check_for_updates()? {
        UpdateCheck::RemoteIsEmpty | UpdateCheck::NoUpdateAvailable => Ok(None),
        UpdateCheck::UpdateAvailable(update) => {
            let current_version = updater.backend.current_version();
            if stable_version_is_newer(&current_version, &update.TargetFullRelease.Version) {
                Ok(Some(*update))
            } else {
                Ok(None)
            }
        }
    }
}

fn stable_version_is_newer(current_version: &Version, release_version: &str) -> bool {
    // Release tags and the packaged version use plain SemVer without a prefix.
    let Ok(candidate_version) = Version::parse(release_version) else {
        return false;
    };
    candidate_version.pre.is_empty() && candidate_version.cmp_precedence(current_version).is_gt()
}

#[cfg(test)]
mod tests {
    use super::stable_version_is_newer;
    use semver::Version;

    #[test]
    fn accepts_newer_stable_semver_versions() {
        assert!(stable_version_is_newer(
            &Version::parse("1.4.9").unwrap(),
            "1.5.0"
        ));
        assert!(stable_version_is_newer(
            &Version::parse("1.4.9").unwrap(),
            "1.5.1+build.2"
        ));
        assert!(!stable_version_is_newer(
            &Version::parse("1.5.0").unwrap(),
            "1.5.0+build.2"
        ));
    }

    #[test]
    fn rejects_equal_older_prerelease_and_prefixed_versions() {
        let current = Version::parse("1.5.0").unwrap();
        for candidate in [
            "1.5.0",
            "1.4.99",
            "1.6.0-alpha.1",
            "1.6.0-beta.1",
            "1.6.0-rc.1",
            "v1.6.0",
            "latest",
        ] {
            assert!(
                !stable_version_is_newer(&current, candidate),
                "unexpected stable update from tag {candidate}"
            );
        }
    }
}
