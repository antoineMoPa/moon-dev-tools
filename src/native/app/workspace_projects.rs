//! Workspace projects - remembers which project each space of the window is on, and puts them
//! back when the window starts again.
//!
//! The list is written to `settings.json` whenever a space changes project, and read once when
//! the settings arrive at a window that was started with [`App::restores_workspace`] set. A
//! project whose folder is gone is left out with a line in the log, rather than failing the
//! launch.

use std::{path::Path, sync::Arc};

use crate::{
    api::OpenSessionRequest,
    native::{
        Launch,
        app::{
            App,
            spaces::{SPACE_COUNT, Slot},
        },
        model::Stage,
    },
    settings::{SettingsChange, WorkspaceProjects},
};

impl App {
    /// Reopen the projects saved last time, once the settings have come. The window's own
    /// space is the one in front: it opens the saved project of its place in the list if it was
    /// started on nothing, or keeps the project it was started on. Every other saved space is
    /// opened in its place in the list, behind the front one.
    pub(crate) fn restore_workspace(&mut self, ctx: &egui::Context) {
        if !self.restores_workspace {
            return;
        }
        // The project the window was started on has to be known to find its place in the
        // saved list, and it is not until it has opened.
        if matches!(self.model.stage, Stage::Opening) {
            return;
        }
        let Some(saved) = self
            .model
            .settings
            .as_ref()
            .map(|settings| settings.workspace_projects.clone())
        else {
            return;
        };
        self.restores_workspace = false;
        if saved.spaces.is_empty() {
            return;
        }
        if saved.spaces.len() != self.spaces.count() {
            eprintln!(
                "[moonreview] ignoring the saved projects: {} spaces saved, {} in the window",
                saved.spaces.len(),
                self.spaces.count()
            );
            return;
        }

        let on_this_machine = self.backend().reads_this_machine();
        let usable: Vec<Option<String>> = saved
            .spaces
            .iter()
            .map(|saved_project| {
                let project = saved_project.as_ref()?;
                if !on_this_machine || Path::new(project).is_dir() {
                    return Some(project.clone());
                }
                eprintln!("[moonreview] not reopening {project}: its folder is gone");
                None
            })
            .collect();

        // A window started on nothing puts its front where the saved one was. A window started
        // on a project keeps it in front, in the place the saved list has it - so the spaces
        // come back in their order - or else in the first place the list leaves empty.
        let launched_on = match self.model.stage {
            Stage::Prompt { .. } => None,
            Stage::Opening => unreachable!("an opening window is waited out above"),
            Stage::Ready => Some(
                self.model
                    .project_path
                    .clone()
                    .expect("a window that has opened its project knows its path"),
            ),
        };
        let front = match &launched_on {
            None => saved.front,
            Some(project) => usable
                .iter()
                .position(|saved_project| saved_project.as_ref() == Some(project))
                .or_else(|| usable.iter().position(Option::is_none))
                .unwrap_or(self.spaces.front),
        };
        if front >= SPACE_COUNT {
            return;
        }
        self.spaces.slots = (0..SPACE_COUNT).map(|_| Slot::Untouched).collect();
        self.spaces.slots[front] = Slot::Front;
        self.spaces.front = front;

        if launched_on.is_none() {
            if let Some(project) = usable[front].clone() {
                self.open_picked_folder(project);
            }
        }
        for (index, project) in usable.into_iter().enumerate() {
            if index == front {
                continue;
            }
            let Some(project) = project else {
                continue;
            };
            let parked = self.space_on_project(ctx, project);
            self.spaces.slots[index] = Slot::Parked(Box::new(parked));
        }
    }

    /// A space that opens the project it is for, built the way a space is when it is first
    /// gone to, but on its project rather than its launch screen.
    fn space_on_project(&self, ctx: &egui::Context, repo_path: String) -> App {
        let mut space = App::new(
            ctx.clone(),
            Launch {
                backend: Arc::clone(self.backend()),
                open: Some(OpenSessionRequest {
                    repo_path,
                    diff_target: None,
                    active_commit: None,
                }),
                frame: self.frame,
            },
        );
        // The shells of a project open in another space are that space's tabs - see
        // `App::build_untouched_space`.
        space.model.adopts_shells_on_open = false;
        space
    }

    /// Write the projects the spaces are on, when they have changed since the file last had
    /// them. Waits while the window is still reopening its projects, and while a space is still
    /// opening one: a space on its way to its project has none to write, and that would lose
    /// the saved one.
    pub(crate) fn remember_workspace(&mut self) {
        if self.model.settings.is_none() || self.restores_workspace {
            return;
        }
        if self.spaces.slots.iter().any(|slot| match slot {
            Slot::Front => matches!(self.model.stage, Stage::Opening),
            Slot::Parked(parked) => matches!(parked.model.stage, Stage::Opening),
            Slot::Untouched => false,
        }) {
            return;
        }
        let spaces = self
            .spaces
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Front => self.model.project_path.clone(),
                Slot::Parked(parked) => parked.model.project_path.clone(),
                Slot::Untouched => None,
            })
            .collect();
        self.change_settings(SettingsChange::SaveWorkspaceProjects(WorkspaceProjects {
            spaces,
            front: self.spaces.front,
        }));
    }
}
