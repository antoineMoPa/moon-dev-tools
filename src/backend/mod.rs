//! Where the window gets its reviews from.
//!
//! The window is the same either way: [`local::LocalBackend`] reviews a repo in this
//! process, and [`remote::RemoteBackend`] reviews a repo on another machine over the HTTP
//! API its `serve` answers. Only this trait knows which one is in play.

pub(crate) mod local;
pub(crate) mod remote;
#[cfg(test)]
mod remote_tests;

use anyhow::Result;

use crate::{
    agent_sessions::AgentSessionView,
    api::{
        AgentKind, AgentLogPayload, BlameOf, BlamePayload, CommentRequest, CommitHistoryPayload,
        ContentMatchesPayload, FileContentPayload, FileMatchesPayload, LspCompletion, LspLocation,
        LspPosition, LspStatus, LspWork, OpenSessionRequest, PatchPayload, SessionOpened,
        SessionPayload, SubmoduleHubPayload,
    },
    commit_suggestion::CommitSuggestion,
    committing::{CommitAction, CommitState},
    moontasks::{
        AttachResourceRequest, BoardColumn, ColumnId, CreateTaskRequest, StartResourceRequest,
        TaskView,
    },
    project::{ProjectCommand, ProjectConfig},
};

/// Every review operation the window performs. Calls block, so the UI runs them
/// on worker threads - a remote backend is a network round-trip.
pub(crate) trait Backend: Send + Sync + 'static {
    /// How this connection reads in the UI, e.g. `local` or `dev-box:42000`.
    fn describe(&self) -> String;

    /// Whether the repos this reads are on the machine the window runs on, which is what
    /// decides if the window can offer a folder picker for them.
    fn reads_this_machine(&self) -> bool;

    /// What another window would have to be given as `--remote` to reach the same repos.
    /// `None` for a backend that reads this machine, which needs no address at all.
    ///
    /// The full address rather than [`Backend::describe`]'s label: that one has its scheme
    /// trimmed off for reading, and a window opened on it would fall back to plain HTTP.
    fn connect_target(&self) -> Option<String>;

    fn open_session(&self, request: OpenSessionRequest) -> Result<SessionOpened>;
    fn session_state(&self, session_id: &str) -> Result<SessionPayload>;
    fn session_submodules(&self, session_id: &str) -> Result<SubmoduleHubPayload>;
    fn commit_history(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<CommitHistoryPayload>;

    fn set_agent(&self, session_id: &str, agent: AgentKind) -> Result<()>;
    fn set_active_commit(&self, session_id: &str, commit: Option<String>) -> Result<()>;

    fn hunk_patch(&self, session_id: &str, hunk_id: &str) -> Result<PatchPayload>;
    fn file_content(&self, session_id: &str, file_path: &str) -> Result<FileContentPayload>;
    /// A file of the repo as one commit has it, for a tab on an old version of it.
    fn file_content_at(
        &self,
        session_id: &str,
        file_path: &str,
        revision: &str,
    ) -> Result<FileContentPayload>;
    /// Who last touched each stretch of the file, in the version `of` names - see
    /// [`crate::git::blame_file`].
    fn blame_file(&self, session_id: &str, file_path: &str, of: &BlameOf) -> Result<BlamePayload>;
    /// The files of the repo whose names match a search, for the palette's file finder.
    fn find_files(&self, session_id: &str, query: &str) -> Result<FileMatchesPayload>;
    /// The lines of the repo that hold what was typed, for the palette's content search.
    fn search_contents(&self, session_id: &str, query: &str) -> Result<ContentMatchesPayload>;
    fn write_file(&self, session_id: &str, file_path: &str, content: &str) -> Result<()>;
    /// Create a file nothing is at yet, which is the first save of a tab opened on a new file.
    /// Refused where something already is.
    fn create_file(&self, session_id: &str, file_path: &str, content: &str) -> Result<()>;

    fn set_comment(&self, session_id: &str, request: CommentRequest) -> Result<()>;
    fn resolve_comment(&self, session_id: &str, hunk_id: &str, comment_index: usize) -> Result<()>;
    fn send_comment_batch(&self, session_id: &str) -> Result<()>;
    fn cancel_dispatch(&self, session_id: &str, hunk_id: &str, comment_index: usize) -> Result<()>;
    fn dispatch_log(&self, session_id: &str, dispatch_key: &str) -> Result<AgentLogPayload>;

    fn stage_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()>;
    fn unstage_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()>;
    fn stage_file(&self, session_id: &str, file_path: &str) -> Result<()>;
    /// Stage the whole working tree at once, which is the commit pane's one staging action.
    fn stage_all(&self, session_id: &str) -> Result<()>;
    fn unstage_file(&self, session_id: &str, file_path: &str) -> Result<()>;
    fn discard_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()>;
    fn discard_hunks(&self, session_id: &str, hunk_ids: &[String]) -> Result<()>;

    /// The moontasks board of the repo this session reviews, and what running it.
    fn list_tasks(&self, session_id: &str) -> Result<Vec<TaskView>>;
    fn create_task(&self, session_id: &str, request: &CreateTaskRequest) -> Result<TaskView>;
    /// Put tasks in a column, at a place among the cards already there, which is what a
    /// drag on the board does, and the only way a card moves. More than one is a card
    /// dragged with others selected, which land as a run in the order they were in.
    fn place_tasks(
        &self,
        session_id: &str,
        task_ids: &[String],
        status: ColumnId,
        position: usize,
    ) -> Result<()>;
    fn delete_task(&self, session_id: &str, task_id: &str) -> Result<()>;
    /// Start a shell or an agent in a task, and answer with the shell it runs in.
    fn start_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        request: StartResourceRequest,
    ) -> Result<String>;
    fn resume_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<String>;
    /// The sessions the installed agents already have for this repo, newest first - what the
    /// attach modal lists.
    fn list_agent_sessions(&self, session_id: &str) -> Result<Vec<AgentSessionView>>;
    /// Put one of those sessions on a task as a new resource, and answer with the shell it
    /// was opened in.
    fn attach_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        request: &AttachResourceRequest,
    ) -> Result<String>;
    fn stop_task_resource(&self, session_id: &str, task_id: &str, resource_id: &str) -> Result<()>;
    /// Take a run off the task for good, rather than leaving it to be resumed.
    fn delete_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<()>;
    fn rename_task(&self, session_id: &str, task_id: &str, title: &str) -> Result<()>;
    /// Make sure the task's notes file exists, and answer with the repo-relative path a file
    /// pane opens it by. Editing then goes through [`Backend::write_file`] like any file.
    fn open_task_notes(&self, session_id: &str, task_id: &str) -> Result<String>;
    /// Put a file of the repo on the task's card, by its path relative to the repo root.
    fn link_task_file(&self, session_id: &str, task_id: &str, file_path: &str) -> Result<()>;

    /// The board's columns, left to right. A board that has never had them changed answers
    /// with the three defaults.
    fn list_columns(&self, session_id: &str) -> Result<Vec<BoardColumn>>;
    /// Add a column, `at` columns from the left. `None` puts it at the right-hand end.
    fn add_column(&self, session_id: &str, label: &str, at: Option<usize>) -> Result<BoardColumn>;
    fn rename_column(&self, session_id: &str, column_id: &ColumnId, label: &str) -> Result<()>;
    /// Which end of a column a card moved in from another column goes to. `None` puts it where
    /// it was dropped, which is what a column says by saying nothing.
    fn set_column_arrivals(
        &self,
        session_id: &str,
        column_id: &ColumnId,
        arrivals: Option<crate::moontasks::ColumnEnd>,
    ) -> Result<()>;
    /// Which order a column keeps its cards in by itself. `None` is the order they are dragged
    /// into.
    fn set_column_sort(
        &self,
        session_id: &str,
        column_id: &ColumnId,
        sort: Option<crate::moontasks::ColumnSort>,
    ) -> Result<()>;
    /// Take an empty column off the board. One still holding cards is refused rather than
    /// taking them with it.
    fn delete_column(&self, session_id: &str, column_id: &ColumnId) -> Result<()>;
    /// Put a column at a place among the others, which is what dragging its heading does. Its
    /// cards go with it, because a card names its column rather than its place on screen.
    fn place_column(&self, session_id: &str, column_id: &ColumnId, position: usize) -> Result<()>;

    /// The two commands the Project menu runs, out of the reviewed repo's own file.
    fn project_commands(&self, session_id: &str) -> Result<ProjectConfig>;
    fn set_project_config(&self, session_id: &str, commands: &ProjectConfig) -> Result<()>;
    /// Start a shell with one of those commands typed into it and sent, and answer with the
    /// shell it runs in. Attached with [`Backend::attach_terminal`], like any other.
    fn run_project_command(&self, session_id: &str, which: ProjectCommand) -> Result<String>;

    /// What the commit pane draws: what is staged, and where a push would send it.
    fn commit_state(&self, session_id: &str) -> Result<CommitState>;
    /// A commit message written from what is staged, by the agent that writes them. Blocks for
    /// as long as that takes, which is seconds rather than milliseconds.
    fn suggest_commit_message(&self, session_id: &str) -> Result<CommitSuggestion>;
    /// Start `git` on one action in a pty, and answer with the shell it runs in. Attached with
    /// [`Backend::attach_terminal`], like any other.
    fn start_commit_run(&self, session_id: &str, action: &CommitAction) -> Result<String>;
    /// The exit code of a run that has ended, `None` while it is still going. Answered once:
    /// the pane asks when the pty closes, and acts on what it gets.
    fn commit_run_outcome(&self, session_id: &str, terminal_id: &str) -> Result<Option<i32>>;

    fn create_terminal(&self, session_id: &str, command: Option<AgentKind>) -> Result<String>;
    /// Start a shell on the repo with one command line typed into it and sent, and answer with
    /// the shell it runs in: what an extension asks for when what it has to show is a program
    /// of its own - `docker logs -f`, a shell inside a container. Attached with
    /// [`Backend::attach_terminal`], like any other.
    fn run_in_shell(&self, session_id: &str, command: &str) -> Result<String>;
    fn list_terminals(&self, session_id: &str) -> Result<Vec<String>>;
    /// The shells with something running in them right now, as opposed to the ones sitting at
    /// a prompt. This is what quitting would interrupt, and so what the window warns about.
    fn terminals_running_a_command(&self, session_id: &str) -> Result<Vec<String>>;
    fn close_terminal(&self, session_id: &str, terminal_id: &str) -> Result<()>;
    /// What a shell is called, if it has been named: an agent's shell is named as it starts,
    /// a plain one only once someone renames it. A shell the server does not have is an error.
    fn terminal_name(&self, session_id: &str, terminal_id: &str) -> Result<Option<String>>;
    /// Call a shell something else, which is what retyping its tab's title does.
    fn rename_terminal(&self, session_id: &str, terminal_id: &str, name: &str) -> Result<()>;
    /// Attach to a shell: everything it has printed, and a handle to type into it. This is
    /// what a terminal pane is built from - see [`egui_tty::TtyStream`].
    fn attach_terminal(&self, session_id: &str, terminal_id: &str) -> Result<egui_tty::TtyStream>;

    /// Whether a language server is behind this file, and whether it has finished starting.
    fn lsp_status(&self, session_id: &str, file_path: &str) -> Result<LspStatus>;
    /// What every language server running for this session is doing right now: indexing,
    /// loading, fetching metadata. Empty when there is nothing to wait for.
    ///
    /// **The caller polls this on a timer, not per frame.** It is a network round trip on a
    /// remote session - see [`crate::native::status_bar`].
    fn lsp_working(&self, session_id: &str) -> Result<Vec<LspWork>>;
    /// Tell the server a file is open and what is in it. Starts the server if this is the
    /// first file of its language.
    fn lsp_did_open(&self, session_id: &str, file_path: &str, text: &str) -> Result<()>;
    /// The whole text again, as it stands. Full-text sync: it skips incremental-version
    /// bookkeeping entirely and the servers used here are fine with it.
    ///
    /// **The caller debounces.** This is a network round trip on a remote session, and a
    /// call per keystroke would flood it: the pane sends this once the typing has paused.
    fn lsp_did_change(&self, session_id: &str, file_path: &str, text: &str) -> Result<()>;
    fn lsp_did_close(&self, session_id: &str, file_path: &str) -> Result<()>;
    /// The places of one kind the server behind a file names for the name at a place: where
    /// it is defined, where its type is, where it is implemented, or everywhere it is used.
    fn lsp_places(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
        which: crate::api::LspPlaces,
    ) -> Result<Vec<LspLocation>>;
    /// The characters the server behind this file said open a completion list on their own:
    /// the `.` of `thing.`, the `:` of a path, the `(` of a call. Empty for a file nothing
    /// serves and for a server that has not finished starting.
    ///
    /// **The caller asks this once per file**, once that file's server is ready - see
    /// [`crate::native::completing`]. The answer is the server's own, said once in its
    /// `initialize` reply and unchanged for as long as it runs, and on a remote session
    /// asking again would be a round trip for something already in hand.
    fn lsp_trigger_characters(&self, session_id: &str, file_path: &str) -> Result<Vec<char>>;
    /// What could finish the word being typed at a place in a file. The pane debounces this
    /// the same way it debounces a change - see [`crate::native::completing`].
    fn lsp_completion(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Vec<LspCompletion>>;
    /// What the name at a place is called, as the server behind the file would rename it:
    /// the text a new name is typed over. `None` when nothing there can be renamed.
    fn lsp_prepare_rename(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Option<String>>;
    /// Everything calling the name at a place `new_name` would change, file by file.
    ///
    /// Nothing is written. Whether a file's edit goes into a tab's buffer or onto disk is the
    /// window's to decide, since only the window knows which files it has open - see
    /// [`crate::native::renaming`].
    fn lsp_rename(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
        new_name: &str,
    ) -> Result<Vec<crate::api::LspFileEdit>>;
    /// The edits that format the whole of one file, indented the way the repo says. Nothing
    /// is written: the window puts them into the tab showing the file.
    fn lsp_format(
        &self,
        session_id: &str,
        file_path: &str,
        options: moon_lsp::LspFormatting,
    ) -> Result<Vec<moon_lsp::LspTextEdit>>;
    /// What the server behind a file says about the name at a place, as markdown.
    fn lsp_hover(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Option<String>>;
    /// What the server behind a file last said is wrong with it.
    ///
    /// **The caller polls this on a timer, not per frame** - see
    /// [`crate::native::diagnostics`]. It is a network round trip on a remote session.
    fn lsp_diagnostics(
        &self,
        session_id: &str,
        file_path: &str,
    ) -> Result<Vec<moon_lsp::LspDiagnostic>>;
    /// Tell the server behind a file it was written to disk.
    fn lsp_did_save(&self, session_id: &str, file_path: &str) -> Result<()>;
    /// What the server behind a file offers to do to the code at a place, each action with
    /// everything it changes. Nothing is written - see [`crate::native::code_actions`].
    fn lsp_code_actions(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Vec<moon_lsp::LspCodeAction>>;
    /// The signature of the call around a place.
    fn lsp_signature_help(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Option<moon_lsp::LspSignature>>;
}
