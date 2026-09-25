//! The window's side of `settings.json`: read from the server once, and changed one
//! [`SettingsChange`] at a time through it - see [`crate::settings`] for why the file is the
//! server's rather than the window's.

use crate::{native::workspace_color::WorkspaceColor, settings::SettingsChange};

use super::App;

impl App {
    /// Ask the server for its settings. Until they come the launch screen has no recent
    /// projects to offer and nothing is written back, since there is nothing yet to compare
    /// a change against.
    pub(super) fn load_settings(&mut self) {
        self.tasks.spawn(
            |backend| backend.settings(),
            |model, result| match result {
                Ok(settings) => {
                    // The agent the person last picked, put back once the review says this
                    // machine still has it - see `App::apply_restored_agent`.
                    model.restored_agent = Some(settings.selected_agent);
                    model.settings = Some(settings);
                }
                Err(error) => model.error(format!("could not read the settings: {error}")),
            },
        );
    }

    /// Make one change to the settings: to the window's copy at once, and to the server's file
    /// behind it. A change to what the copy already says is not sent.
    fn change_settings(&mut self, change: SettingsChange) {
        let Some(settings) = &mut self.model.settings else {
            return;
        };
        if !settings.apply(change.clone()) {
            return;
        }
        self.tasks.spawn(
            move |backend| backend.change_settings(change.clone()),
            |model, result| {
                if let Err(error) = result {
                    model.error(format!("could not save the settings: {error}"));
                }
            },
        );
    }

    /// Mark this window's project with a color, and keep `settings.json` in step.
    ///
    /// A window that is on no project yet - the launch screen - has nothing to mark: the
    /// color is remembered against the project's path, so there is nowhere to put it.
    pub(crate) fn set_workspace_color(&mut self, color: WorkspaceColor) {
        self.model.workspace_color = color;
        // The ground is baked into the style, so the whole window has to be restyled.
        self.needs_style = true;

        let Some(project_path) = self.model.project_path.clone() else {
            return;
        };
        self.change_settings(SettingsChange::MarkWorkspace {
            project_path,
            color,
        });
    }

    /// Paint the window in the color its project was last marked with. The project and the
    /// settings each arrive a moment after the window does - both are a round trip - so this
    /// runs each frame and does nothing until both are there.
    pub(super) fn follow_project_color(&mut self) {
        let (Some(project_path), Some(settings)) =
            (self.model.project_path.as_deref(), &self.model.settings)
        else {
            return;
        };
        let color = settings.workspace_color(project_path);
        if color == self.model.workspace_color {
            return;
        }
        self.model.workspace_color = color;
        self.needs_style = true;
    }

    /// Keep `settings.json` in step with the selector at the top of the review.
    ///
    /// Written when the choice changes rather than on a clock: the file is one line, and a
    /// selector nobody has touched should leave it exactly as the user last left it - or as
    /// they last edited it by hand.
    pub(crate) fn remember_selected_agent(&mut self) {
        // Before the restored agent has been put back, the session still reads as `None`, and
        // writing that would throw away the very choice being restored. Before the settings
        // have come, there is no restored agent yet to wait for.
        if self.model.settings.is_none() || self.model.restored_agent.is_some() {
            return;
        }
        let selected = self.selected_agent();
        self.change_settings(SettingsChange::SelectAgent(selected));
    }

    /// Write a project that has just opened to the head of the recent list, so the next launch
    /// screen offers it. Held until the settings have come, so the change is made to them.
    pub(super) fn remember_opened_project(&mut self) {
        if self.model.settings.is_none() {
            return;
        }
        let Some(path) = self.model.opened_project.take() else {
            return;
        };
        self.change_settings(SettingsChange::RememberProject(path));
    }
}
