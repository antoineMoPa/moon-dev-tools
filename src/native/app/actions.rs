//! What the window does between frames: polls the backend, opens reviews and windows, and
//! runs the commands a keystroke or a menu item asks for.

use std::{
    sync::Arc,
    time::Instant,
};

use egui_frames::PaneId;

use crate::{
    api::OpenSessionRequest,
    native::{
        bindings::Action,
        board, find,
        model::Stage,
        palette::CommandAction,
        panes::{OpenPaneRequest, Pane, PaneKind},
        programs::Opens,
    },
};

use super::{
    App, BACKGROUND_POLL_INTERVAL, BOARD_POLL_INTERVAL, POLL_INTERVAL, QUIT_CONFIRM_WINDOW,
    TabAction,
};

impl App {

    pub(super) fn open_review(&mut self, open: OpenSessionRequest) {
        let frame = self.frame;
        // Only remembered once the review is actually open: a path that turns out not to be a
        // repo has no business on the launch screen's list.
        let repo_path = open.repo_path.clone();
        self.tasks.spawn(
            move |backend| backend.open_session(open),
            move |model, result| match result {
                Ok(opened) => {
                    model.root_session_id = opened.session_id.clone();
                    // A stored arrangement contributes its splits; what goes in it is new.
                    model.layout = crate::native::workspace::arrangement_for(
                        model.restored_layout.take(),
                        &opened.session_id,
                        frame,
                    );
                    model.review(&opened.session_id);
                    model.stage = Stage::Ready;
                    model.opened_project = Some(repo_path.clone());
                    model.project_path = Some(repo_path.clone());
                    model.adopt_shells_pending = true;
                    model.board.refresh_requested = true;
                    // `moonshell` opens on a shell, which has to be started before there is
                    // anything to draw.
                    model.open_shell_pending = frame == crate::cli::Frame::Shell;
                }
                Err(error) => {
                    let message = format!("{error}");
                    model.stage = Stage::Prompt {
                        repo_path: String::new(),
                        error: Some(message.clone()),
                    };
                    model.error(format!("could not open the review: {message}"));
                }
            },
        );
    }

    /// Refetch every open review, on the poll clock or because something changed it.
    ///
    /// Each refetch re-reads the diff from git, which is not free in a large repo, so a
    /// window nobody is looking at checks back far less often.
    pub(super) fn poll_reviews(&mut self, focused: bool) {
        let interval = if focused {
            POLL_INTERVAL
        } else {
            BACKGROUND_POLL_INTERVAL
        };
        let due = self.last_poll.elapsed() >= interval;
        let session_ids: Vec<String> = self
            .model
            .reviews
            .values()
            .filter(|review| due || review.refresh_requested)
            .map(|review| review.session_id.clone())
            .collect();
        if session_ids.is_empty() {
            return;
        }
        if due {
            self.last_poll = Instant::now();
        }

        for session_id in session_ids {
            self.model.review(&session_id).refresh_requested = false;
            let key = format!("state:{session_id}");
            let for_apply = session_id.clone();
            self.tasks.spawn_keyed(
                Some(key),
                move |backend| backend.session_state(&session_id),
                move |model, result| {
                    let review = model.review(&for_apply);
                    review.loading = false;
                    match result {
                        Ok(payload) => {
                            review.error = None;
                            // An active hunk that the diff no longer contains would leave the
                            // keyboard acting on something the user cannot see.
                            if let Some(active) = review.active_hunk_id.clone()
                                && !payload.hunks.iter().any(|hunk| hunk.id == active)
                            {
                                review.active_hunk_id = None;
                            }
                            review.history_has_more = payload.history_has_more;
                            if review.history_loaded.is_empty() {
                                review.history_loaded = payload.history_commits.clone();
                            }
                            // A comment being typed must survive the file changing under it.
                            // Hunk ids are content hashes, so an agent editing the file while
                            // a composer is open renames every hunk and would strand its
                            // draft: re-anchor each one to the hunk that still matches.
                            for draft in &mut review.drafts {
                                if payload.hunks.iter().any(|hunk| hunk.id == draft.hunk_id) {
                                    continue;
                                }
                                use crate::native::review::diff::{
                                    build_diff_lines, insertion_line,
                                };
                                let matched = payload
                                    .hunks
                                    .iter()
                                    .find(|hunk| {
                                        hunk.file_path == draft.file_path
                                            && insertion_line(
                                                &build_diff_lines(&hunk.patch_preview),
                                                &draft.selection,
                                                &[],
                                            )
                                            .is_some()
                                    })
                                    .or_else(|| {
                                        payload
                                            .hunks
                                            .iter()
                                            .find(|hunk| hunk.file_path == draft.file_path)
                                    });
                                if let Some(hunk) = matched {
                                    draft.hunk_id = hunk.id.clone();
                                    draft.header = hunk.header.clone();
                                }
                            }
                            review.payload = Some(Arc::new(payload));
                        }
                        Err(error) => review.error = Some(format!("{error}")),
                    }
                },
            );
        }
    }

    /// Refetch the moontasks board while a pane is showing it.
    ///
    /// The board is a folder anything may write to - an agent moving its own card, a second
    /// window, a text editor - so the only way to know what is on it is to read it again.
    pub(super) fn poll_board(&mut self) {
        if self.model.root_session_id.is_empty() || !board::is_open(self) {
            return;
        }
        let due = self.last_board_poll.elapsed() >= BOARD_POLL_INTERVAL;
        if !due && !self.model.board.refresh_requested {
            return;
        }
        if due {
            self.last_board_poll = Instant::now();
        }
        self.model.board.refresh_requested = false;

        let session_id = self.model.root_session_id.clone();
        // The columns come back with the cards rather than on a read of their own, so a card
        // is never drawn for a frame against a board that has not heard of its column yet.
        self.tasks.spawn_keyed(
            Some("tasks".to_string()),
            move |backend| {
                let columns = backend.list_columns(&session_id)?;
                let tasks = backend.list_tasks(&session_id)?;
                Ok((columns, tasks))
            },
            |model, result| {
                model.board.loaded = true;
                match result {
                    Ok((columns, tasks)) => {
                        model.board.error = None;
                        board::columns::accept_columns(model, columns);
                        board::cards::accept_board(model, tasks);
                    }
                    Err(error) => model.board.error = Some(format!("{error}")),
                }
            },
        );
    }

    /// Open a tab on a shell the board just started.
    pub(super) fn open_shell_the_board_started(&mut self) {
        let Some(opened) = self.model.board.opened_shell.take() else {
            return;
        };
        self.open_pane(OpenPaneRequest::AttachTerminal {
            terminal_id: opened.terminal_id,
            command: opened.command,
            task_id: Some(opened.task_id),
        });
    }

    /// Open the file the board just readied - a task's notes, or a file linked to a card - in
    /// a pane down the right.
    ///
    /// The board's tasks are read from the root session's repo, so that is the session the
    /// file pane reads the file through.
    pub(super) fn open_file_the_board_readied(&mut self) {
        let Some(file_path) = self.model.board.opened_file.take() else {
            return;
        };
        let session_id = self.model.root_session_id.clone();
        self.open_notes_pane(session_id, file_path);
    }

    /// Put a file on a task's card, and open it once it is there.
    ///
    /// The link comes first: the pane is the same one the card opens the file into, so a
    /// link the server refused would be a file open beside a card that does not name it.
    fn link_task_file(&mut self, task_id: String, file_path: String) {
        let session_id = self.model.root_session_id.clone();
        let opens = file_path.clone();
        self.tasks.spawn(
            move |backend| backend.link_task_file(&session_id, &task_id, &file_path),
            move |model, result| {
                model.board.refresh_requested = true;
                match result {
                    Ok(()) => model.board.opened_file = Some(opens),
                    Err(error) => model.error(format!("could not link the file: {error}")),
                }
            },
        );
    }

    pub(super) fn poll_submodules(&mut self) {
        if self.model.root_session_id.is_empty() {
            return;
        }
        let session_id = self.model.root_session_id.clone();
        self.tasks.spawn_keyed(
            Some("submodules".to_string()),
            move |backend| backend.session_submodules(&session_id),
            |model, result| {
                if let Ok(submodules) = result {
                    model.submodules = submodules;
                }
            },
        );
    }

    /// Run whatever the palette or the menu bar asked for.
    pub(crate) fn run_action(&mut self, ctx: &egui::Context, action: CommandAction) {
        match action {
            CommandAction::OpenPane(request) => self.open_pane(request),
            CommandAction::ToggleTheme => self.set_theme(self.model.theme.toggled()),
            CommandAction::InstallLaunchers => self.install_launchers(),
            CommandAction::NewWindow(frame) => self.open_new_window(frame),
            CommandAction::RestartWindow => self.restart_window(ctx),
            CommandAction::OpenFile => self.pick_file_to_edit(ctx),
            CommandAction::FindFile => self.model.palette.show_files(),
            CommandAction::LinkTaskFile { task_id, file_path } => {
                self.link_task_file(task_id, file_path);
            }
            CommandAction::SearchContent => self.model.palette.show_contents(),
            CommandAction::Split(side) => self.split_frame(side),
        }
    }

    /// Open another window - of this program or of one of its siblings - on its launch
    /// screen, where it asks which repo to open.
    ///
    /// A window is a process here: each one carries its own review server and its own shells,
    /// so there is nothing to open a second window out of but a second run of the executable.
    /// It is left to run on its own; closing this one does not take it with it.
    fn open_new_window(&mut self, frame: crate::cli::Frame) {
        let Some(executable) = crate::native::programs::executable_for(frame) else {
            self.model.error(format!(
                "{} is not installed beside this one",
                frame.program()
            ));
            return;
        };

        if let Err(error) = self.start_window(frame, &executable, Opens::LaunchScreen) {
            self.model.error(format!(
                "could not open a {} window: {error}",
                frame.display_name()
            ));
        }
    }

    /// Start this program again on the repo this window is on, and close this window once the
    /// new one is on its way.
    ///
    /// A window runs the executable it was started with, so a rebuilt one only reaches the
    /// screen through a second process. The new instance is started first: a window that
    /// closed on a failed spawn would leave the user with nothing.
    fn restart_window(&mut self, ctx: &egui::Context) {
        let frame = self.frame;
        let Some(executable) = crate::native::programs::executable_for(frame) else {
            self.model.error(format!(
                "{} is no longer installed beside this window",
                frame.program()
            ));
            return;
        };

        // Without a project the window is on its launch screen, and that is where it comes
        // back to.
        let project_path = self.model.project_path.clone();
        let opens = match &project_path {
            Some(path) => Opens::Repo(path),
            None => Opens::LaunchScreen,
        };
        match self.start_window(frame, &executable, opens) {
            Ok(()) => self.close_window(ctx),
            Err(error) => self
                .model
                .error(format!("could not restart this window: {error}")),
        }
    }

    /// Close this window because the window itself was told to go, rather than because someone
    /// pressed ⌘Q.
    ///
    /// Asking for a restart is already the answer to "a shell is still running": the window
    /// arms the confirmation on its way out, so the close it sends itself is not questioned
    /// back and answered with a toast instead of a new window.
    pub(crate) fn close_window(&mut self, ctx: &egui::Context) {
        self.quit_armed_until = Some(Instant::now() + QUIT_CONFIRM_WINDOW);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Start another process of `frame`, against the same machine this window reads.
    fn start_window(
        &self,
        frame: crate::cli::Frame,
        executable: &std::path::Path,
        opens: Opens<'_>,
    ) -> std::io::Result<()> {
        let target = self.backend().connect_target();
        let launcher = crate::native::launchers::installed_launcher(frame);
        crate::native::programs::window_command(
            executable,
            launcher.as_deref(),
            target.as_deref(),
            opens,
        )
        .spawn()
        .map(|_| ())
    }

    /// Write the launchers the OS lists, and say what landed where.
    fn install_launchers(&mut self) {
        match crate::native::launchers::install() {
            Ok(installed) => {
                let names: Vec<&str> = installed
                    .iter()
                    .map(|launcher| launcher.frame.display_name())
                    .collect();
                self.model.info(format!(
                    "{} in {}",
                    names.join(", "),
                    crate::native::launchers::destination_hint()
                ));
            }
            Err(error) => self
                .model
                .error(format!("could not write the launchers: {error}")),
        }
    }

    /// The repo the window was launched on, as a path on whichever machine the backend reads.
    pub(crate) fn repo_root(&self) -> Option<std::path::PathBuf> {
        let session_id = self.model.root_session_id.clone();
        let payload = self.model.review_ref(&session_id)?.payload.as_ref()?;
        Some(std::path::PathBuf::from(&payload.repo_path))
    }

    /// Where a file of the review sits on this machine, if the repo is on this machine at all.
    pub(crate) fn repo_file_path(&self, file_path: &str) -> Option<std::path::PathBuf> {
        Some(self.repo_root()?.join(file_path))
    }

    /// The OS file picker, opened on the repo, for a file to read and edit in a tab of its
    /// own - the same tab the review opens a file into.
    ///
    /// A file is named to the review server by its path inside the repo, so a pick from
    /// anywhere else on disk has no name to be opened under. Rather than open a tab that
    /// cannot load, the pick is refused and says why.
    fn pick_file_to_edit(&mut self, ctx: &egui::Context) {
        let Some(repo_root) = self.repo_root() else {
            self.model.error("no repo is open in this window yet");
            return;
        };

        let picked = rfd::FileDialog::new()
            .set_title("Open a file to edit")
            .set_directory(&repo_root)
            .pick_file();
        // The window loses the keyboard to the dialog, and egui only learns that it is back
        // once something else asks it to draw.
        ctx.request_repaint();
        let Some(picked) = picked else {
            return;
        };

        match path_inside_repo(&repo_root, &picked) {
            Some(file_path) => {
                let session_id = self.model.root_session_id.clone();
                self.open_file_pane(&session_id, &file_path);
            }
            None => self.model.error(format!(
                "{} is outside {}, and only files of the repo can be opened here",
                picked.display(),
                repo_root.display()
            )),
        }
    }

    /// ⌘W closes the tab in front. Only a workspace with nothing left in it passes the chord
    /// on to the window itself.
    pub(super) fn close_active_tab(&mut self, ctx: &egui::Context) {
        match self.active_pane_id() {
            Some(pane_id) => self.pending_close = Some(pane_id),
            None => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// A file with edits that are not on disk takes two presses to close, so a stray ⌘W or a
    /// mis-aimed click cannot throw work away.
    pub(super) fn close_would_lose_edits(&mut self, pane_id: PaneId) -> bool {
        if !self.file_pane_is_dirty(pane_id) {
            return false;
        }
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return false;
        };
        if editor.close_confirmed {
            return false;
        }
        editor.close_confirmed = true;
        let file_path = editor.file_path.clone();
        self.model
            .error(format!("{file_path} has unsaved edits - close again to discard them"));
        true
    }

    /// Read this frame's keyboard through the binding table and act on what it fired.
    pub(super) fn apply_shortcuts(&mut self, ctx: &egui::Context) {
        // A shell gets every plain keystroke - `s` there is the letter s - and so does a text
        // box, the palette's search line included: a box the window owns is still a box the
        // user is typing in. Only the chords marked as reaching anywhere are the window's
        // while either has the keyboard.
        let typing = ctx.egui_wants_keyboard_input()
            || self.active_pane_kind() == Some(PaneKind::Terminal);

        for action in self.keymap.resolve(ctx, typing) {
            self.apply_action(action);
        }
    }

    fn apply_action(&mut self, action: Action) {
        match action {
            Action::OpenPalette => self.model.palette.show(),
            Action::NewShellTab => self.pending_tab_action = Some(TabAction::New),
            Action::CloseTab => self.pending_tab_action = Some(TabAction::Close),
            Action::SelectTab(index) => self.select_tab(index),
            // Deferred into the same slot the menu bar's item uses: on macOS the chord can
            // arrive as both, and two windows is not what one ⌘N asked for.
            Action::NewWindow => self.pending_action = Some(CommandAction::NewWindow(self.frame)),
            Action::SaveFile => {
                if let Some((pane_id, Pane::File { session_id, .. })) = self.active_pane() {
                    let session_id = session_id.clone();
                    self.save_file_pane(pane_id, &session_id);
                }
            }
            Action::ToggleTheme => self.set_theme(self.model.theme.toggled()),
            Action::AdvanceHunk => self.apply_hunk_shortcut(true),
            Action::ReverseHunk => self.apply_hunk_shortcut(false),
            Action::FocusNextFrame => self.focus_next_frame(),
            Action::Find => find::open(self),
            Action::FindFile => self.model.palette.show_files(),
            Action::SearchContent => self.model.palette.show_contents(),
            Action::OpenReview => self.open_root_review(),
        }
    }

    /// cmd+shift+R brings this window's review forward, opening it if the workspace has not
    /// got one - the same thing the palette's "review" command does. A window whose review is
    /// still opening has no session to point a pane at yet, so the chord does nothing there.
    fn open_root_review(&mut self) {
        if self.model.root_session_id.is_empty() {
            return;
        }
        let session_id = self.model.root_session_id.clone();
        self.open_pane(OpenPaneRequest::Review {
            session_id,
            title: "review".to_string(),
        });
    }

    /// `s` and `u` stage and unstage the hunk the review pane has under the caret. A
    /// read-only review has no index to move anything into, so they do nothing there.
    fn apply_hunk_shortcut(&mut self, forward: bool) {
        let Some(session_id) = self.focused_review_session() else {
            return;
        };
        let Some(review) = self.model.review_ref(&session_id) else {
            return;
        };
        let Some(active) = review.active_hunk_id.clone() else {
            return;
        };
        let Some(hunk) = review.hunks().iter().find(|hunk| hunk.id == active).cloned() else {
            return;
        };
        if review.read_only() {
            return;
        }

        if hunk.staged == forward {
            return;
        }
        let hunk_id = hunk.id.clone();
        let for_call = session_id.clone();
        if forward {
            self.tasks
                .act(&session_id, "could not stage the hunk", move |backend| {
                    backend.stage_hunk(&for_call, &hunk_id)
                });
        } else {
            self.tasks
                .act(&session_id, "could not unstage the hunk", move |backend| {
                    backend.unstage_hunk(&for_call, &hunk_id)
                });
        }
    }

    /// The OS folder picker, opened where the last project was found so the next one is
    /// usually a sibling, or on the home directory. What it comes back with is what opens.
    pub(super) fn pick_repo_folder(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut dialog = rfd::FileDialog::new().set_title("Choose a repo");
        if let Some(recent) = self.settings.recent_projects.first() {
            let beside = std::path::Path::new(recent).parent().unwrap_or_else(|| {
                // A project at the filesystem root has no parent to open beside it.
                std::path::Path::new(recent)
            });
            if beside.is_dir() {
                dialog = dialog.set_directory(beside);
            }
        }

        let picked = dialog.pick_folder()?;
        // The window loses the keyboard to the dialog, and egui only learns that it is back
        // once something else asks it to draw.
        ctx.request_repaint();
        Some(picked.display().to_string())
    }

    /// Run the find bar's search over a review, and tell the bar what it found.
    ///
    /// A search walks every hunk, so it only happens when the bar asks - a changed query or a
    /// step to another match - rather than on every frame the review draws.
    pub(crate) fn apply_review_find(&mut self, pane_id: PaneId, session_id: &str) {
        let Some((query, at, pending)) = self
            .model
            .find
            .as_ref()
            .filter(|find| find.pane_id == pane_id)
            .map(|find| (find.query.clone(), find.at, find.pending))
        else {
            // No bar on this pane: nothing of a previous search stays marked.
            let review = self.model.review(session_id);
            review.find_query.clear();
            review.find_match = None;
            return;
        };

        self.model.review(session_id).find_query = query.clone();
        if !pending {
            return;
        }

        let found = crate::native::review::search::find_all(self, session_id, &query);
        let current = found.get(at).cloned();
        let review = self.model.review(session_id);
        // Bringing the hunk into view is what makes a match in a file scrolled far away
        // findable; the mark on the line itself says where in the hunk it is.
        review.scroll_to_hunk = current.as_ref().map(|found| found.hunk_id.clone());
        review.find_match = current;
        if let Some(find) = &mut self.model.find {
            find.found(found.len());
        }
    }
}

/// Both sides are resolved before they are compared: on macOS a repo is often reached through
/// a symlink - `/var` for `/private/var`, and the picker hands back the resolved form - so
/// comparing what was picked against an unresolved root would refuse a file plainly inside it.
fn path_inside_repo(repo_root: &std::path::Path, picked: &std::path::Path) -> Option<String> {
    let repo_root = repo_root.canonicalize().ok()?;
    let picked = picked.canonicalize().ok()?;
    let relative = picked.strip_prefix(&repo_root).ok()?;
    Some(relative.display().to_string())
}

#[cfg(test)]
mod path_tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU32, Ordering},
    };

    use super::path_inside_repo;

    static NEXT_ID: AtomicU32 = AtomicU32::new(0);

    /// A directory of its own, removed when the test ends.
    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moonreview-open-file-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("failed to create temp test directory");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn a_file_in_the_repo_is_named_by_its_path_inside_it() {
        let repo = TestDir::new();
        let nested = repo.path.join("src");
        std::fs::create_dir(&nested).expect("expected a dir");
        let file = nested.join("main.rs");
        std::fs::write(&file, "fn main() {}").expect("expected a file");

        assert_eq!(
            path_inside_repo(&repo.path, &file).as_deref(),
            Some(format!("src{}main.rs", std::path::MAIN_SEPARATOR).as_str())
        );
    }

    #[test]
    fn a_file_outside_the_repo_has_no_name_to_be_opened_under() {
        let repo = TestDir::new();
        let elsewhere = TestDir::new();
        let file = elsewhere.path.join("notes.txt");
        std::fs::write(&file, "not in the repo").expect("expected a file");

        assert_eq!(path_inside_repo(&repo.path, &file), None);
    }

    /// The picker can only hand back a file that is there, but the repo may have moved out
    /// from under the window since it was opened.
    #[test]
    fn a_file_that_is_not_there_is_not_a_file_of_the_repo() {
        let repo = TestDir::new();

        assert_eq!(path_inside_repo(&repo.path, &repo.path.join("gone.rs")), None);
    }
}
