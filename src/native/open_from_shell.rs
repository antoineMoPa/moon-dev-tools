//! `moon open <file>` arriving in the window: the tab it opens, and what the window writes
//! down about itself so a shell can find it in the first place.
//!
//! A file of the project this window is on opens in the window's own review. A file of any
//! other project opens too: the shell hands it here when no window is open on its project -
//! see [`crate::instances::windows_for`] - and the window opens a session on that project,
//! the same way a submodule review is opened, so the file has a repo to be read and written
//! in. One session per project, kept in [`SessionsForAskedFiles`], so a second file of the
//! same project lands in a tab beside the first rather than opening the project again.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{
    api::OpenSessionRequest,
    instances::window::{OpenFileAsked, ShellAsks},
    native::{app::App, panes::OpenAt},
};

/// The sessions this window opened so it could take files of other projects, by the project
/// each was opened on. Shared with the tasks that open them, which is where entries come from.
pub(crate) type SessionsForAskedFiles = Arc<Mutex<HashMap<PathBuf, ProjectSession>>>;

/// What became of the session a file of another project needs.
#[derive(Clone)]
pub(crate) enum ProjectSession {
    /// The session the project is open on now, which the file's tab is opened against.
    Open(String),
    /// The project could not be opened, and the window has said so. The files waiting on it
    /// are dropped rather than asked for again on every frame.
    Unopenable,
}

impl App {
    /// Start answering the shells that ask this window to open a file. Only the real window
    /// calls it: a ui test that listened would take asks meant for the window the developer
    /// running it has open.
    pub(crate) fn listen_for_shell_asks(&mut self, ctx: &egui::Context) {
        let reads_this_machine = self.backend().reads_this_machine();
        match ShellAsks::listen(self.frame().command(), reads_this_machine, ctx.clone()) {
            Ok(asks) => self.shell_asks = Some(asks),
            // Not fatal: the window works, `moon open` just cannot reach this one.
            Err(error) => eprintln!("[moonreview] `moon open` cannot reach this window: {error}"),
        }
    }

    /// Keep the window's record in step with the project it is on and with when it was last
    /// in front, and open what shells have asked for since the last frame.
    ///
    /// The project is the review's own repo path rather than the path the window was started
    /// with: a file is measured against it, and only the review's has been resolved - see
    /// [`crate::git::project_root`].
    pub(super) fn follow_shell_asks(&mut self, ctx: &egui::Context) {
        let repo_root = self.repo_root();
        // The window that was in front most recently is where a file whose project no window
        // is open on goes, so coming forward is written down as it happens.
        let focused = ctx.input(|input| input.focused);
        let came_forward = focused && !std::mem::replace(&mut self.window_is_in_front, focused);
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
            if came_forward && let Err(error) = asks.came_to_the_front() {
                eprintln!("[moonreview] could not write this window down: {error}");
            }
            self.asked_files.extend(arrived);
        }

        // One a frame: a tab is opened through the same deferred slot every other pane change
        // goes through, and what is left waits for the next frame rather than being dropped.
        if self.pending_action.is_some() {
            return;
        }
        // A file is opened by its path inside a project, so an ask that arrives before this
        // window's own project has finished opening waits for it rather than being refused.
        let Some(repo_root) = repo_root else {
            return;
        };
        let Some(asked) = self.asked_files.front() else {
            return;
        };

        if asked.path.starts_with(&repo_root) {
            let asked = self.asked_files.pop_front().expect("a file is waiting");
            let session_id = self.model.root_session_id.clone();
            self.open_asked_file(ctx, &session_id, &repo_root, asked);
            return;
        }
        self.open_asked_file_of_another_project(ctx);
    }

    /// The file at the front of the queue is one of another project: open it in the session
    /// this window keeps for that project, opening the session first when there is not one.
    fn open_asked_file_of_another_project(&mut self, ctx: &egui::Context) {
        let path = self
            .asked_files
            .front()
            .expect("a file is waiting")
            .path
            .clone();
        let folder = path.parent().expect("a file sits in a folder");
        let project = match crate::git::project_root(folder) {
            Ok(project) => project,
            Err(error) => {
                self.asked_files.pop_front();
                self.model
                    .error(format!("could not open {}: {error}", path.display()));
                return;
            }
        };

        let opened = self
            .sessions_for_asked_files
            .lock()
            .expect("the sessions lock")
            .get(&project)
            .cloned();
        match opened {
            Some(ProjectSession::Open(session_id)) => {
                let asked = self.asked_files.pop_front().expect("a file is waiting");
                self.open_asked_file(ctx, &session_id, &project, asked);
            }
            // The window has already said why, so the file goes quietly.
            Some(ProjectSession::Unopenable) => {
                self.asked_files.pop_front();
            }
            None => self.open_project_for_asked_files(project),
        }
    }

    /// Open a session on a project this window is not on, so the files a shell asked for can
    /// be read and written in it. Keyed by the project, so the files waiting on it ask for it
    /// once between them however many frames go by before it answers.
    fn open_project_for_asked_files(&mut self, project: PathBuf) {
        let sessions = Arc::clone(&self.sessions_for_asked_files);
        let repo_path = project.display().to_string();
        self.tasks.spawn_keyed(
            Some(format!("open-project-for-asked-files:{repo_path}")),
            move |backend| {
                backend.open_session(OpenSessionRequest {
                    repo_path,
                    diff_target: None,
                    active_commit: None,
                })
            },
            move |model, result| {
                let mut sessions = sessions.lock().expect("the sessions lock");
                match result {
                    Ok(opened) => {
                        sessions.insert(project, ProjectSession::Open(opened.session_id));
                    }
                    Err(error) => {
                        model.error(format!(
                            "could not open {} to put the file in: {error}",
                            project.display()
                        ));
                        sessions.insert(project, ProjectSession::Unopenable);
                    }
                }
            },
        );
    }

    /// Open one file a shell asked for, in the session on the project holding it, and bring
    /// the window to the front - the ask was typed somewhere else, so this window is not the
    /// one being looked at.
    fn open_asked_file(
        &mut self,
        ctx: &egui::Context,
        session_id: &str,
        project: &Path,
        asked: OpenFileAsked,
    ) {
        // A file is named by its path inside the project its session is on. It can still be a
        // file of the project this window was on when the ask was made, which is this window's
        // answer to a window that has moved on: it says so and opens nothing.
        let Ok(file_path) = asked.path.strip_prefix(project) else {
            self.model.error(format!(
                "{} is outside {}, so this window cannot open it",
                asked.path.display(),
                project.display()
            ));
            return;
        };
        let file_path = file_path.display().to_string();

        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        // A line is a place in the text, so a tab opened at one opens on the text: the
        // rendered page a markdown file otherwise opens on has no line 40 to show.
        let at = asked.line.map(|line| OpenAt {
            line,
            // Nothing was searched for, so nothing is marked - the line is the whole of what
            // was asked for.
            query: String::new(),
        });
        self.open_file_pane_asked_for(session_id, &file_path, at);
    }
}
