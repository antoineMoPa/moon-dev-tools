//! The command palette: everything the workspace can open, searchable.
//!
//! Everything ⌘⇧P offers, in one list.

mod commands;
mod drawing;
mod rows;

pub(crate) use commands::commands_for;
pub(crate) use drawing::draw;

use rows::{code_action_rows, content_rows, file_rows, place_rows, rename_rows};

use egui_frames::DropSide;

use crate::{
    api::SearchScope,
    native::{app::App, bindings, panes::OpenPaneRequest},
    project::ProjectCommand,
};

/// What the palette's query is picking.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteMode {
    /// Something the window can do, out of the list below.
    Commands,
    /// A file of the repo, by name, found with `ag` wherever the repo lives.
    Files,
    /// A line of the repo, by the text on it, found the same way.
    Contents,
    /// A new name for the name at a file tab's caret - see [`crate::native::renaming`].
    Rename,
    /// One of the places a language server named, filtered by what is typed - see
    /// [`crate::native::places`].
    Places,
    /// One of the code actions a language server offered, filtered by what is typed - see
    /// [`crate::native::code_actions`].
    CodeActions,
}

/// One question put to a search: what was typed, and which files it was looked for in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct SearchRequest {
    pub(crate) query: String,
    pub(crate) scope: SearchScope,
}

/// What one of the palette's two searches has found so far, for the request it was started
/// on. The matches come in while the search runs and `done` says when it is over; a palette
/// that has moved on since - a key typed, the scope box toggled - starts another search,
/// whose ticket is what tells its reports from the old one's.
pub(crate) struct Search<T> {
    pub(crate) searched: Option<SearchRequest>,
    /// The ticket the search was started on - see `PaletteState::latest_search`. A report
    /// from a search with another ticket is not about this list.
    pub(crate) ticket: u64,
    pub(crate) matches: Vec<T>,
    /// Set when the repo had more matches than the search hands back, so the palette can say
    /// that narrowing the query would show different rows rather than only fewer.
    pub(crate) truncated: bool,
    /// Whether the search is over: until it is, the rows are what has been found so far.
    pub(crate) done: bool,
    pub(crate) error: Option<String>,
}

impl<T> Default for Search<T> {
    fn default() -> Self {
        Self {
            searched: None,
            ticket: 0,
            matches: Vec::new(),
            truncated: false,
            done: false,
            error: None,
        }
    }
}

pub(crate) struct Command {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) action: CommandAction,
    /// The keyboard chord that does the same thing, for the ones that have one. Read out of
    /// the binding table so the palette cannot drift from what the keyboard actually does.
    pub(crate) shortcut: Option<&'static [bindings::Press]>,
}

/// What running a command does. Most open a pane; the rest are the window's own actions,
/// which on macOS also sit in the menu bar.
#[derive(Clone)]
pub(crate) enum CommandAction {
    OpenPane(OpenPaneRequest),
    ToggleTheme,
    /// Paint this window's ground, so it is told from the other windows open beside it.
    MarkWorkspace(crate::native::workspace_color::WorkspaceColor),
    // This one and the next two start programs on this machine, and `OpenFile` is its file
    // picker: none of them is anything a browser's window can do.
    #[cfg(not(target_arch = "wasm32"))]
    InstallLaunchers,
    /// Another window of one of the three programs, on its launch screen.
    #[cfg(not(target_arch = "wasm32"))]
    NewWindow(crate::cli::Frame),
    /// Start this program again on the repo this window is on, and close this window.
    #[cfg(not(target_arch = "wasm32"))]
    RestartWindow,
    /// Ask the OS which file of the repo to open for editing, and open it in a tab.
    #[cfg(not(target_arch = "wasm32"))]
    OpenFile,
    /// Turn the palette into the file finder, where what is typed is a file name.
    FindFile,
    /// Put a file of the repo on a task's card, then open it: what the file finder does with
    /// a pick when a card's `[start]` menu opened it.
    LinkTaskFile {
        task_id: String,
        file_path: String,
    },
    /// Turn the palette into the content search, where what is typed is looked for in the
    /// text of every file of the repo.
    SearchContent,
    /// Split the frame the keyboard is in against this side, with a shell in the new half.
    Split(DropSide),
    /// Run one of the project's own commands in a shell of its own.
    RunProject(ProjectCommand),
    /// Put this window's project down and go back to its launch screen, to open another.
    SwitchProject,
    /// Start an open extension over from its script - see [`crate::extensions`].
    #[cfg(not(target_arch = "wasm32"))]
    RestartExtension(String),
    /// Ask what the name at the caret of the file tab in front is, and open the palette on it
    /// to type a new one - see [`crate::native::renaming`].
    RenameSymbol,
    /// Rename the name the palette was opened on to this.
    RenameTo(String),
    /// Go to, or list, the places of one kind of the name at the caret of the file tab in
    /// front - see [`crate::native::places`].
    FindPlaces(egui_moon_code_ide::LspPlaces),
    /// Format the file tab in front with its language server - see
    /// [`crate::native::formatting`].
    FormatFile,
    /// List what the language server offers to do at the caret of the file tab in front - see
    /// [`crate::native::code_actions`].
    CodeActions,
    /// Put the blame of the file tab in front up beside its lines, or take it down - see
    /// [`crate::native::blame`].
    ToggleBlame,
    /// Carry out one of the code actions the palette is listing, by its place in the list.
    ApplyCodeAction(usize),
    /// Open the project's work log at a new dated entry - see [`crate::native::work_log`].
    OpenWorkLog,
    /// This window's repo in a browser - see `crate::native::open_in_web`. Nothing a
    /// browser's window has to offer: it is the browser.
    #[cfg(not(target_arch = "wasm32"))]
    OpenInWeb,
    /// A new pass key to this window's server, on the clipboard - see
    /// `crate::native::pass_key`. A browser's window is let in already, and has no server of
    /// its own to make one for.
    #[cfg(not(target_arch = "wasm32"))]
    GeneratePassKey,
}

/// Every typed term has to appear somewhere in the title or description, which makes
/// "term cl" find the Claude terminal.
pub(crate) fn filter(commands: Vec<Command>, query: &str) -> Vec<Command> {
    let terms: Vec<String> = query
        .trim()
        .to_lowercase()
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect();
    if terms.is_empty() {
        return commands;
    }

    commands
        .into_iter()
        .filter(|command| {
            let searchable = format!("{} {}", command.title, command.description).to_lowercase();
            terms.iter().all(|term| searchable.contains(term))
        })
        .collect()
}

/// What the palette is offering under the query: the commands that match it, or the files.
fn rows_for(app: &App) -> Vec<Command> {
    match app.model.palette.mode {
        PaletteMode::Commands => filter(commands_for(app), &app.model.palette.query),
        PaletteMode::Files => file_rows(app),
        PaletteMode::Contents => content_rows(app),
        PaletteMode::Rename => rename_rows(app),
        PaletteMode::Places => filter(place_rows(app), &app.model.palette.query),
        PaletteMode::CodeActions => filter(code_action_rows(app), &app.model.palette.query),
    }
}

#[cfg(test)]
mod tests {
    use super::commands::work_log_commands;
    use super::drawing::{ROW_HEIGHT, rows_height_of};
    use super::*;

    fn command(title: &str, description: &str) -> Command {
        Command {
            title: title.to_string(),
            description: description.to_string(),
            action: CommandAction::OpenPane(OpenPaneRequest::Agents),
            shortcut: None,
        }
    }

    /// `wl` typed into the palette is the work log, first: Enter opens it.
    #[test]
    fn wl_is_the_work_log_and_the_first_row_for_it() {
        let commands = vec![
            command("review", "Bring the repo review forward"),
            command("terminal", "Open a new shell"),
        ]
        .into_iter()
        .chain(work_log_commands())
        .collect();

        let matches = filter(commands, "wl");

        assert_eq!(matches[0].title, "wl");
        assert!(matches!(matches[0].action, CommandAction::OpenWorkLog));
    }

    /// Three rows want three rows' height and the two gaps between them, and one row wants
    /// no gap at all.
    #[test]
    fn the_rows_height_counts_the_gaps_between_the_rows() {
        assert_eq!(rows_height_of(3, 4.0), 3.0 * ROW_HEIGHT + 8.0);
        assert_eq!(rows_height_of(1, 4.0), ROW_HEIGHT);
        assert_eq!(rows_height_of(0, 4.0), 0.0);
    }

    #[test]
    fn an_empty_query_keeps_every_command() {
        let commands = vec![
            command("review", "Open the moon-dev-tools review"),
            command("terminal", "Open a new shell"),
        ];

        assert_eq!(filter(commands, "  ").len(), 2);
    }

    #[test]
    fn every_term_has_to_match_somewhere() {
        let commands = vec![
            command("terminal", "Open a new shell"),
            command("claude", "Open Claude in a terminal"),
        ];

        let matches = filter(commands, "term cl");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].title, "claude");
    }

    #[test]
    fn matching_is_case_insensitive_and_searches_descriptions() {
        let commands = vec![command("comment agents", "Open the comment agent monitor")];

        assert_eq!(filter(commands, "MONITOR").len(), 1);
    }
}
