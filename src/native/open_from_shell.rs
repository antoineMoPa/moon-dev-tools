//! Open from shell - handles `moon open <file>`, `moon open <folder>` and `moon shell <folder>`
//! when they arrive in the window: opens the tab for a file and for a shell, brings the file
//! picker up on a folder, and writes down what a shell needs to find this window.
//!
//! A file of the project this window is on opens in the window's own review. A file of any
//! other project opens too: the shell hands it here when no window is open on its project -
//! see [`crate::instances::windows_for`] - and the window has the repo's side place it, in a
//! session on the project it sits in, the same way a submodule review is opened, so the file
//! has a folder to be read and written in - see [`crate::api::folders::FilePlaced`]. A file
//! in no repo at all, `/etc/hostname`, is placed in a session on its own folder. The project
//! is found where the disk is rather than here, so the files the window's own picker hands
//! on - see [`App::open_picked_file`] - are placed the same whichever machine that is.
//!
//! A folder for a shell goes the way a submodule's shell does: the session on its project is
//! opened as the shell is started - see [`App::open_shell_in_folder`] - so it needs no session
//! kept.
//!
//! A folder to open is browsed rather than opened: the window's own picker comes up on it,
//! the one File › Open brings up - see [`crate::native::file_picker`] - and the file picked
//! there opens the way that one's does. The picker lists any folder of the disk, so a folder
//! of another project needs no session until a file of it is picked.
//!
//! A line of the wire - `moon wire post @handle` - arrives the same way and opens nothing: it
//! is typed into a shell, which is [`crate::native::wire`]'s.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{
    api::folders::FilePlaced,
    instances::window::{OpenFileAsked, OpenShellAsked, PickFileAsked, ShellAsks},
    native::{
        app::App,
        file_picker::PickPurpose,
        panes::{OpenAt, Pane},
    },
};

/// Where the repo's side put the files of other projects this window asked it to place, by
/// the path each was asked for under. Shared with the tasks that ask, which is where entries
/// come from; an entry is taken out as its file is opened.
pub(crate) type PlacesOfAskedFiles = Arc<Mutex<HashMap<PathBuf, AskedFilePlace>>>;

/// What came of placing a file of another project.
pub(crate) enum AskedFilePlace {
    /// The session its tab is opened against, and its path in that session's project.
    Placed(FilePlaced),
    /// It could not be placed, and the window has said so. The file is dropped rather than
    /// asked about again on every frame.
    Nowhere,
}

/// A tab a `moon edit --wait` asked for, which the shell that asked is waiting on to close.
pub(crate) struct WaitedTab {
    /// The file as the shell named it, which is how it asks after it.
    path: PathBuf,
    session_id: String,
    /// The file inside the project of `session_id`, which is how its tab names it.
    file_path: String,
    /// Whether the tab has been open yet. It opens a frame or two after it is asked for, and
    /// until then its absence is the tab not being there yet rather than it having closed.
    seen_open: bool,
}

impl App {
    /// Start answering the shells that ask this window to open a file. Only the real window
    /// calls it: a ui test that listened would take asks meant for the window the developer
    /// running it has open.
    pub(crate) fn listen_for_shell_asks(&mut self, ctx: &egui::Context) {
        let reads_this_machine = self.backend().reads_this_machine();
        match ShellAsks::listen(self.frame().command(), reads_this_machine, ctx.clone()) {
            Ok(asks) => {
                asks.agents_answered_by(Arc::new(crate::native::agent_asks::WindowAgents {
                    backend: Arc::clone(self.backend()),
                }));
                // A window that is its machine's session starts what `moon launch` asks for,
                // on the screen it is on. Any other window starts no programs and says so.
                // Compiled wherever `started` is: on Linux, and with the tests.
                #[cfg(any(target_os = "linux", test))]
                if self.manages_the_session {
                    asks.applications_started_by(Arc::new(
                        crate::display::started::OnThisScreen,
                    ));
                }
                self.shell_asks = Some(asks);
            }
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
        let mut wired = Vec::new();
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
            self.asked_shells.extend(asks.drain_shells());
            // The picker is one box over the window, so the folder asked for last is the one
            // it comes up on.
            if let Some(asked) = asks.drain_file_picks().pop() {
                self.asked_file_pick = Some(asked);
            }
            wired = asks.drain_wired();
        }
        // A line of the wire is for a shell this window's moon holds, whatever project the
        // window is on by now, so it waits on nothing.
        self.type_wired_lines(wired);
        self.release_closed_tabs();

        // A file is opened by its path inside a project, and a shell is started through the
        // window's own session, so an ask that arrives before this window's own project has
        // finished opening waits for it rather than being refused.
        let Some(repo_root) = repo_root else {
            return;
        };
        // A shell is started on a task of its own rather than through the deferred pane slot,
        // so every folder asked for since the last frame starts now.
        for asked in std::mem::take(&mut self.asked_shells) {
            self.open_asked_shell(ctx, asked);
        }
        // The picker is put up directly as well: it is a box over the window, not a pane.
        if let Some(asked) = self.asked_file_pick.take() {
            self.pick_file_in_asked_folder(ctx, asked);
        }

        // One a frame: a tab is opened through the same deferred slot every other pane change
        // goes through, and what is left waits for the next frame rather than being dropped.
        if self.pending_action.is_some() {
            return;
        }
        let Some(asked) = self.asked_files.front() else {
            return;
        };

        if let Ok(file_path) = asked.path.strip_prefix(&repo_root) {
            let file_path = file_path.display().to_string();
            let asked = self.asked_files.pop_front().expect("a file is waiting");
            let session_id = self.model.root_session_id.clone();
            // A shell only reaches a window that reads this machine - see
            // [`crate::instances::window`] - so this is the disk the path was typed against.
            let exists = asked.path.exists();
            self.open_asked_file(ctx, &session_id, file_path, exists, asked);
            return;
        }
        self.open_asked_file_of_another_project(ctx);
    }

    /// The file at the front of the queue is one of another project: open it where the repo's
    /// side placed it, asking for that first when it has not been asked yet.
    fn open_asked_file_of_another_project(&mut self, ctx: &egui::Context) {
        let path = self
            .asked_files
            .front()
            .expect("a file is waiting")
            .path
            .clone();
        let place = self
            .places_of_asked_files
            .lock()
            .expect("the places lock")
            .remove(&path);
        match place {
            Some(AskedFilePlace::Placed(placed)) => {
                let asked = self.asked_files.pop_front().expect("a file is waiting");
                // Said in the tab's header, where the path inside the project alone would
                // not say which `hostname` this is.
                self.model
                    .projects_of_placed_files
                    .insert(placed.session_id.clone(), placed.project);
                self.open_asked_file(
                    ctx,
                    &placed.session_id,
                    placed.file_path,
                    placed.exists,
                    asked,
                );
            }
            // The window has already said why, so the file goes quietly.
            Some(AskedFilePlace::Nowhere) => {
                let asked = self.asked_files.pop_front().expect("a file is waiting");
                self.release(&asked);
            }
            None => self.place_asked_file(path),
        }
    }

    /// Ask the repo's side for a session on the project a file sits in, so the file can be
    /// read and written there. Keyed by the file, so it is asked for once however many frames
    /// go by before the answer.
    fn place_asked_file(&mut self, path: PathBuf) {
        let places = Arc::clone(&self.places_of_asked_files);
        let asked = path.display().to_string();
        self.tasks.spawn_keyed(
            Some(format!("place-asked-file:{asked}")),
            move |backend| backend.place_file(&asked),
            move |model, result| {
                let place = match result {
                    Ok(placed) => AskedFilePlace::Placed(placed),
                    Err(error) => {
                        model.error(format!("could not open {}: {error}", path.display()));
                        AskedFilePlace::Nowhere
                    }
                };
                places.lock().expect("the places lock").insert(path, place);
            },
        );
    }

    /// Open one asked-for file in the session on the project holding it, by its path inside
    /// that project, and bring the window to the front - a shell's ask was typed somewhere
    /// else, so this window is not the one being looked at.
    fn open_asked_file(
        &mut self,
        ctx: &egui::Context,
        session_id: &str,
        file_path: String,
        exists: bool,
        asked: OpenFileAsked,
    ) {
        if asked.wait {
            self.waited_tabs.push(WaitedTab {
                path: asked.path.clone(),
                session_id: session_id.to_string(),
                file_path: file_path.clone(),
                seen_open: false,
            });
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        // A path nothing is at yet is a file about to be written, the way `vim notes.md` is:
        // the tab opens empty and its first save creates the file, so a tab closed unsaved
        // leaves nothing behind.
        if !exists {
            self.pending_action = Some(crate::native::palette::CommandAction::OpenPane(
                crate::native::panes::OpenPaneRequest::NewFile {
                    session_id: session_id.to_string(),
                    file_path,
                },
            ));
            return;
        }
        // A line is a place in the text, so a tab opened at one opens on the text: the
        // rendered page a markdown file otherwise opens on has no line 40 to show.
        let at = asked.line.map(|line| OpenAt {
            line,
            // Nothing was searched for, so nothing is marked - the line is the whole of what
            // was asked for.
            query: String::new(),
        });
        self.open_file_pane_at(session_id, &file_path, at);
    }

    /// Start the shell a `moon shell <folder>` asked for, and bring the window to the front -
    /// the ask was typed somewhere else, so this window is not the one being looked at.
    fn open_asked_shell(&mut self, ctx: &egui::Context, asked: OpenShellAsked) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.open_shell_in_folder(asked.folder);
    }

    /// Bring the file picker up on the folder a `moon open <folder>` named, and bring the
    /// window to the front - the ask was typed somewhere else, so this window is not the one
    /// being looked at. The pick is File › Open's: it goes to [`App::open_picked_file`].
    fn pick_file_in_asked_folder(&mut self, ctx: &egui::Context, asked: PickFileAsked) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.ask_for_a_pick(PickPurpose::FileToEdit, &asked.folder);
    }

    /// Stop the shell waiting on a file this window will never open a tab on.
    fn release(&self, asked: &OpenFileAsked) {
        if asked.wait
            && let Some(asks) = &self.shell_asks
        {
            asks.release(&asked.path);
        }
    }

    /// Let go of the shells whose tab has been open and is not any more: the file is done
    /// with, and whatever ran `moon edit --wait` on it - git, for a commit message - can go
    /// on and read it.
    fn release_closed_tabs(&mut self) {
        if self.waited_tabs.is_empty() {
            return;
        }
        let layout = &self.model.layout;
        let shell_asks = &self.shell_asks;
        self.waited_tabs.retain_mut(|waited| {
            let open = layout
                .find_pane(|pane| {
                    matches!(pane, Pane::File { session_id, file_path, revision: None, .. }
                        if *session_id == waited.session_id && *file_path == waited.file_path)
                })
                .is_some();
            if open {
                waited.seen_open = true;
                return true;
            }
            if !waited.seen_open {
                return true;
            }
            if let Some(asks) = shell_asks {
                asks.release(&waited.path);
            }
            false
        });
    }
}
