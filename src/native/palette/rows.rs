//! The rows each mode of the palette lists besides its commands - code actions, places, the
//! repo's files and what is in them - and the searches that fill the last two in the background.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::{
    api::SearchProgress,
    native::{app::App, model::Model, panes::OpenPaneRequest, tasks::ModelEdits},
    search::SearchListener,
};

use super::commands::repo_name_of;
use super::{Command, CommandAction, PaletteMode, Search, SearchRequest};

/// One row per code action the language server offered, in the order it put them.
pub(super) fn code_action_rows(app: &App) -> Vec<Command> {
    let Some(actions) = crate::native::code_actions::offered(&app.model) else {
        return Vec::new();
    };
    actions
        .iter()
        .enumerate()
        .map(|(index, action)| Command {
            title: action.title.clone(),
            description: crate::native::code_actions::about(action),
            action: CommandAction::ApplyCodeAction(index),
            shortcut: None,
        })
        .collect()
}

/// One row per place the language server named: what its line reads, and where it is.
pub(super) fn place_rows(app: &App) -> Vec<Command> {
    let Some(found) = &app.model.palette.places else {
        return Vec::new();
    };
    found
        .places
        .iter()
        .map(|place| Command {
            title: place
                .line_text
                .as_deref()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .unwrap_or_else(|| file_name_of(&place.file_path))
                .to_string(),
            description: format!("{}:{}", place.file_path, place.line_number),
            action: crate::native::places::open_at(&found.session_id, place, &found.word),
            shortcut: None,
        })
        .collect()
}

/// The one row the rename asks for: the name the palette was opened on, to what is typed.
/// None until what is typed is a name other than the one it has.
pub(super) fn rename_rows(app: &App) -> Vec<Command> {
    let Some(target) = crate::native::renaming::naming(&app.model) else {
        return Vec::new();
    };
    let new_name = app.model.palette.query.trim();
    if new_name.is_empty() || new_name == target.name {
        return Vec::new();
    }
    vec![Command {
        title: format!("rename {} to {new_name}", target.name),
        description: format!(
            "Everywhere the language server knows it is used, from {}",
            target.file_path
        ),
        action: CommandAction::RenameTo(new_name.to_string()),
        shortcut: None,
    }]
}

/// The name the rename mode is asking a new name for, as its hint and its empty line say it.
fn renamed_name(app: &App) -> &str {
    crate::native::renaming::naming(&app.model).map_or("the name", |target| target.name.as_str())
}

/// One row per file the search found: the name to read it by, and the path it is at.
///
/// Running one opens the file - and first puts it on a card, when the finder was opened from
/// that card's `[start]` menu.
pub(super) fn file_rows(app: &App) -> Vec<Command> {
    app.model
        .palette
        .files
        .matches
        .iter()
        .map(|file_path| Command {
            title: file_name_of(file_path).to_string(),
            description: file_path.clone(),
            action: match &app.model.palette.files_link_to_task {
                Some(task_id) => CommandAction::LinkTaskFile {
                    task_id: task_id.clone(),
                    file_path: file_path.clone(),
                },
                None => CommandAction::OpenPane(OpenPaneRequest::File {
                    session_id: app.model.palette.search_session_id.clone(),
                    file_path: file_path.clone(),
                    at: None,
                }),
            },
            shortcut: None,
        })
        .collect()
}

/// One row per matching line: the line itself to read the match by, and where in the repo it
/// is. Running it opens the file at that line, with the text that was searched for marked.
pub(super) fn content_rows(app: &App) -> Vec<Command> {
    // What the rows on screen were found for, which is not what is typed while a search
    // started by the last keystroke is still out.
    let searched = app
        .model
        .palette
        .contents
        .searched
        .as_ref()
        .map(|asked| asked.query.clone())
        .unwrap_or_default();
    app.model
        .palette
        .contents
        .matches
        .iter()
        .map(|found| Command {
            title: found.line.clone(),
            description: format!("{}:{}", found.file_path, found.line_number),
            action: CommandAction::OpenPane(OpenPaneRequest::File {
                session_id: app.model.palette.search_session_id.clone(),
                file_path: found.file_path.clone(),
                at: Some(crate::native::panes::OpenAt {
                    line: found.line_number,
                    query: searched.clone(),
                }),
            }),
            shortcut: None,
        })
        .collect()
}

/// What the searches have to say under their rows, if anything - see [`footnote_of`].
pub(super) fn footnote(app: &App, shown: usize) -> Option<String> {
    match app.model.palette.mode {
        PaletteMode::Commands
        | PaletteMode::Rename
        | PaletteMode::Places
        | PaletteMode::CodeActions => None,
        PaletteMode::Files => footnote_of(&app.model.palette.files, shown),
        PaletteMode::Contents => footnote_of(&app.model.palette.contents, shown),
    }
}

fn file_name_of(file_path: &str) -> &str {
    file_path.rsplit('/').next().unwrap_or(file_path)
}

pub(super) fn hint_of(app: &App) -> String {
    match app.model.palette.mode {
        PaletteMode::Commands => "Execute a command…".to_string(),
        PaletteMode::Files if app.model.palette.files_link_to_task.is_some() => {
            "Link a file to the task by name…".to_string()
        }
        PaletteMode::Files => format!(
            "Open a file{} by name, or by a path with * or ?…",
            searched_repo_note(app)
        ),
        PaletteMode::Contents => format!("Find text in the files{}…", searched_repo_note(app)),
        PaletteMode::Rename => format!("A new name for {}…", renamed_name(app)),
        PaletteMode::Places => match &app.model.palette.places {
            Some(found) => format!(
                "{} {} ({}) - type to filter…",
                crate::native::places::kind_of(found.which).listed,
                found.word,
                found.places.len()
            ),
            None => "Filter the places…".to_string(),
        },
        PaletteMode::CodeActions => "What to do at the caret…".to_string(),
    }
}

/// Which repo the searches read, when it is not the one the window was launched on: a
/// search opened from a file of a submodule reads that submodule, and the hint says so,
/// since nothing else on screen would. Empty for the root repo, which needs no saying.
fn searched_repo_note(app: &App) -> String {
    let searched = &app.model.palette.search_session_id;
    if *searched == app.model.root_session_id {
        return String::new();
    }
    format!(" of {}", repo_name_of(app, searched))
}

/// What the palette says when it has no rows to show.
pub(super) fn empty_message(app: &App) -> String {
    match app.model.palette.mode {
        PaletteMode::Commands => "nothing matches".to_string(),
        PaletteMode::Files => searching_message(&app.model.palette.files)
            .unwrap_or_else(|| "no file of the repo has that name".to_string()),
        PaletteMode::Contents => {
            if app.model.palette.query.is_empty() {
                return "type what to look for in the files".to_string();
            }
            searching_message(&app.model.palette.contents)
                .unwrap_or_else(|| "no file of the repo holds that text".to_string())
        }
        PaletteMode::Rename => format!("type a new name for {}", renamed_name(app)),
        PaletteMode::Places | PaletteMode::CodeActions => "nothing matches".to_string(),
    }
}

/// What a search has to say for itself while it has nothing to show, if anything: a search
/// that is not over is still looking, and saying "no matches" would be a lie.
fn searching_message<T>(search: &Search<T>) -> Option<String> {
    match &search.error {
        Some(error) => Some(error.clone()),
        None if !search.done => Some("searching…".to_string()),
        None => None,
    }
}

/// The line under a search's rows, if it has one to say: that the rows are still coming in,
/// or that they are only the start of what the repo matched - a cut-short list is not the
/// whole answer, and the rows alone cannot say so.
fn footnote_of<T>(search: &Search<T>, shown: usize) -> Option<String> {
    if search.error.is_some() {
        None
    } else if !search.done {
        Some("searching…".to_string())
    } else if search.truncated {
        Some(format!(
            "the first {shown} matches - narrow the search for the rest"
        ))
    } else {
        None
    }
}

/// What the palette is asking of a search right now: the query in its box, in the scope its
/// checkbox has.
fn asked_of(app: &App) -> SearchRequest {
    SearchRequest {
        query: app.model.palette.query.clone(),
        scope: app.model.palette.search_scope,
    }
}

/// Keep the file list on the query that is typed.
pub(super) fn refresh_file_matches(app: &mut App) {
    refresh_search(
        app,
        |backend, session_id, asked, listener| {
            backend.find_files(session_id, &asked.query, asked.scope, listener)
        },
        |model| &mut model.palette.files,
    );
}

/// The same for the lines the content search found.
pub(super) fn refresh_content_matches(app: &mut App) {
    refresh_search(
        app,
        |backend, session_id, asked, listener| {
            backend.search_contents(session_id, &asked.query, asked.scope, listener)
        },
        |model| &mut model.palette.contents,
    );
}

/// Keep one of the searches on what is asked: the query that is typed, in the scope that is
/// ticked.
///
/// The repo can be on another machine, so this is a backend call on a worker thread like
/// reading a file is - one that reports as it goes, so the rows fill in while `ag` is still
/// walking the tree. A new request starts a new search at once, on a ticket of its own; the
/// one before it sees it is no longer the latest and stops.
fn refresh_search<T: Send + 'static>(
    app: &mut App,
    find: fn(
        &dyn crate::backend::Backend,
        &str,
        &SearchRequest,
        &mut dyn SearchListener<T>,
    ) -> anyhow::Result<()>,
    search_of: fn(&mut Model) -> &mut Search<T>,
) {
    let asked = asked_of(app);
    if search_of(&mut app.model).searched.as_ref() == Some(&asked) {
        return;
    }
    let ticket = app.model.palette.next_search_ticket();
    let no_repo = app.model.palette.search_session_id.is_empty();
    let search = search_of(&mut app.model);
    *search = Search {
        searched: Some(asked.clone()),
        ticket,
        ..Search::default()
    };
    if no_repo {
        search.done = true;
        search.error = Some("no repo is open in this window yet".to_string());
        return;
    }

    let session_id = app.model.palette.search_session_id.clone();
    let latest = Arc::clone(&app.model.palette.latest_search);
    app.tasks.spawn_editing(
        None,
        move |backend, edits| {
            let mut listener = ListReports {
                edits: edits.clone(),
                ticket,
                latest,
                search_of,
            };
            find(backend, &session_id, &asked, &mut listener)
        },
        move |model, result| {
            let search = search_of(model);
            if search.ticket != ticket {
                return;
            }
            if let Err(error) = result {
                search.matches.clear();
                search.truncated = false;
                search.error = Some(format!("{error}"));
            }
            search.done = true;
        },
    );
}

/// A running search's listener: each report it hears becomes the list on screen, as long as
/// the search is still the one the list is for.
struct ListReports<T> {
    edits: ModelEdits,
    ticket: u64,
    latest: Arc<AtomicU64>,
    search_of: fn(&mut Model) -> &mut Search<T>,
}

impl<T: Send + 'static> SearchListener<T> for ListReports<T> {
    fn wanted(&mut self) -> bool {
        self.latest.load(Ordering::SeqCst) == self.ticket
    }

    fn found(&mut self, progress: SearchProgress<T>) {
        let ticket = self.ticket;
        let search_of = self.search_of;
        self.edits.push(move |model| {
            let search = search_of(model);
            if search.ticket != ticket {
                return;
            }
            search.matches = progress.matches;
            search.truncated = progress.truncated;
            search.done = progress.done;
        });
    }
}
