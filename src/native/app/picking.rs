//! Picking - asks for the folder a window opens on and for a file to edit, with the window's
//! picker, and opens what was picked.
//!
//! Each ask is in two halves, because the picker answers on a later frame - see
//! [`crate::native::file_picker`]: the half that asks, and the half that is handed the path.

use crate::{
    api::OpenSessionRequest,
    native::{
        file_picker::{self, PickPurpose},
        model::Stage,
    },
};

use super::App;

impl App {
    /// Ask which folder the window opens on, starting where the last project was found so
    /// the next one is usually a sibling. The pick goes to [`App::open_picked_folder`] when
    /// it is made.
    pub(super) fn pick_repo_folder(&mut self) {
        let beside_the_last_project = self
            .model
            .settings
            .as_ref()
            .and_then(|settings| settings.recent_projects.first())
            // A project at the filesystem root has no parent to open beside it.
            .map(|recent| {
                let recent = std::path::Path::new(recent);
                recent.parent().unwrap_or(recent).to_path_buf()
            });

        let folder = match &beside_the_last_project {
            Some(folder) => folder.display().to_string(),
            None => file_picker::HOME.to_string(),
        };
        self.ask_for_a_pick(PickPurpose::RepoFolder, &folder);
    }

    /// Open the window on a folder: the one picked, typed, or clicked among the recent ones on
    /// the launch screen.
    pub(crate) fn open_picked_folder(&mut self, repo_path: String) {
        self.model.stage = Stage::Opening;
        self.open_review(OpenSessionRequest {
            repo_path,
            diff_target: None,
            active_commit: None,
        });
    }

    /// Ask which file to read and edit in a tab of its own - the same tab the review opens a
    /// file into - with the picker opened on the repo.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn pick_file_to_edit(&mut self) {
        let Some(repo_root) = self.repo_root() else {
            self.model.error("no repo is open in this window yet");
            return;
        };
        self.ask_for_a_pick(PickPurpose::FileToEdit, &repo_root.display().to_string());
    }

    /// Open a file picked anywhere on the disk the repo is on, by its absolute, resolved path.
    ///
    /// A file of the repo opens in the window's own review. Any other is handed on the way a
    /// file `moon open` named is, and opens through a session on the project it does sit in -
    /// see [`crate::native::open_from_shell`].
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_picked_file(&mut self, picked: std::path::PathBuf) {
        let Some(repo_root) = self.repo_root() else {
            self.model.error("no repo is open in this window yet");
            return;
        };
        match picked.strip_prefix(&repo_root) {
            Ok(file_path) => {
                let session_id = self.model.root_session_id.clone();
                // Brought forward when it is already in a tab: it was just asked for.
                self.open_file_pane_at(&session_id, &file_path.display().to_string(), None);
            }
            Err(_) => self
                .asked_files
                .push_back(crate::instances::window::OpenFileAsked {
                    path: picked,
                    line: None,
                    wait: false,
                }),
        }
    }
}
