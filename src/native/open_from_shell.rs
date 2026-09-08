//! `moon open <file>` arriving in the window: the tab it opens, and what the window writes
//! down about itself so a shell can find it in the first place.

use std::path::Path;

use crate::{
    instances::window::{OpenFileAsked, ShellAsks},
    native::{app::App, panes::OpenAt},
};

impl App {
    /// Start answering the shells that ask this window to open a file. Only the real window
    /// calls it: a ui test that listened would take asks meant for the window the developer
    /// running it has open.
    pub(crate) fn listen_for_shell_asks(&mut self, ctx: &egui::Context) {
        match ShellAsks::listen(self.frame().command(), ctx.clone()) {
            Ok(asks) => self.shell_asks = Some(asks),
            // Not fatal: the window works, `moon open` just cannot reach this one.
            Err(error) => eprintln!("[moonreview] `moon open` cannot reach this window: {error}"),
        }
    }

    /// Keep the window's record in step with the project it is on, and open what shells have
    /// asked for since the last frame.
    ///
    /// The project is the review's own repo path rather than the path the window was started
    /// with: a file is measured against it, and only the review's has been resolved - see
    /// [`crate::git::project_root`].
    pub(super) fn follow_shell_asks(&mut self, ctx: &egui::Context) {
        let repo_root = self.repo_root();
        if let Some(asks) = &self.shell_asks {
            let arrived = asks.drain();
            if repo_root != self.project_asks_reach_this_window_on
                && let Some(repo_root) = &repo_root
            {
                if let Err(error) = asks.on_project(&repo_root.display().to_string()) {
                    eprintln!("[moonreview] could not write this window down: {error}");
                }
                self.project_asks_reach_this_window_on = Some(repo_root.clone());
            }
            self.asked_files.extend(arrived);
        }

        // One a frame: a tab is opened through the same deferred slot every other pane change
        // goes through, and what is left waits for the next frame rather than being dropped.
        if self.pending_action.is_some() {
            return;
        }
        // A file is opened by its path inside the project, so an ask that arrives before the
        // project has finished opening waits for it rather than being refused.
        let Some(repo_root) = repo_root else {
            return;
        };
        let Some(asked) = self.asked_files.pop_front() else {
            return;
        };
        self.open_asked_file(ctx, &repo_root, asked);
    }

    /// Open one file a shell asked for, and bring the window to the front - the ask was typed
    /// somewhere else, so this window is not the one being looked at.
    fn open_asked_file(&mut self, ctx: &egui::Context, repo_root: &Path, asked: OpenFileAsked) {
        // The window only takes files of the project it is on - see
        // [`crate::instances::window`] - so this is the path it is named by inside it. It can
        // still be a file of the project the window was on when the ask was made, which is
        // this window's answer to a window that has moved on: it says so and opens nothing.
        let Ok(file_path) = asked.path.strip_prefix(repo_root) else {
            self.model.error(format!(
                "{} is outside {}, so this window cannot open it",
                asked.path.display(),
                repo_root.display()
            ));
            return;
        };
        let file_path = file_path.display().to_string();
        let session_id = self.model.root_session_id.clone();

        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        // A line is a place in the text, so a tab opened at one opens on the text: the
        // rendered page a markdown file otherwise opens on has no line 40 to show.
        let at = asked.line.map(|line| OpenAt {
            line,
            // Nothing was searched for, so nothing is marked - the line is the whole of what
            // was asked for.
            query: String::new(),
        });
        self.open_file_pane_asked_for(&session_id, &file_path, at);
    }
}
