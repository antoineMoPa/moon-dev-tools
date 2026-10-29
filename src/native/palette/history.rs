//! History - the box of what was run from the palette, drawn above it: the project's entries
//! as the server keeps them (see [`crate::palette_history`]), filtered by what is typed, the
//! keys that move into and through it, and running an entry again.
//!
//! The box is always there under ⌘⇧P, faint. ↑ on the first row of the list, or Ctrl+R from
//! any row, puts the keyboard in it - see [`HistoryCursor`].

use egui::{Key, Modifiers};
use egui_command_palette::{Aside, HistoryBox, Kept, Row};

use crate::{
    moontasks::TaskView,
    native::{app::App, panes::OpenPaneRequest, theme::Palette},
    palette_history::{HistoryEntry, HistoryRecord, HistoryThing},
};

#[cfg(not(target_arch = "wasm32"))]
use crate::native::board::marks::Activity;

use super::{Command, CommandAction, PaletteMode, commands_for, sections::marks_of, terms_of};

/// What the box says at its bottom, and in place of its rows when none match.
const HISTORY_LABEL: &str = "history";
const NOTHING_MATCHES: &str = "nothing in the history matches";

/// What the query line says before what is typed while Ctrl+R searches the history.
pub(super) const SEARCH_LABEL: &str = "history search:";

/// How a quick task's agent is doing, as its entry says it, and the colour it says it in: the
/// card's dot's - see [`crate::native::board::marks`].
#[cfg(not(target_arch = "wasm32"))]
const AGENT_STATES: [(Activity, &str, fn(&Palette) -> egui::Color32); 4] = [
    (Activity::Running, "working", |palette| palette.added),
    (Activity::Quiet, "waiting for you", |palette| {
        palette.partial
    }),
    (Activity::Attention, "waiting for you", |palette| {
        palette.warn
    }),
    (Activity::Ended, "ended", |palette| palette.muted),
];

/// The palette's history as the window holds it: the project's entries, and where the keyboard
/// is in them.
#[derive(Default)]
pub(crate) struct HistoryState {
    /// Oldest first, as last read from the server - when the palette was opened - with what
    /// was run since added. `None` until read.
    entries: Option<Vec<HistoryEntry>>,
    /// The board's cards, read with the entries: which quick tasks are still open, and what
    /// their agents are doing.
    cards: Option<Vec<TaskView>>,
    pub(super) cursor: HistoryCursor,
}

impl HistoryState {
    /// Put the keyboard back in the palette's list, as a new opening has it.
    pub(crate) fn rest(&mut self) {
        self.cursor = HistoryCursor::default();
    }

    /// The entries and cards a test opens the palette on, as if read from the server.
    #[cfg(test)]
    pub(crate) fn seed(&mut self, entries: Vec<HistoryEntry>, cards: Vec<TaskView>) {
        self.entries = Some(entries);
        self.cards = Some(cards);
    }

    /// Whether the keyboard is in the box.
    #[cfg(test)]
    pub(crate) fn in_use(&self) -> bool {
        self.cursor.active()
    }

    /// Add what was just run to the entries the window holds, as the server adds it to its
    /// file. Not read yet, the next read has it.
    fn add(&mut self, record: HistoryRecord) {
        if let Some(entries) = &mut self.entries {
            crate::palette_history::record(entries, record);
        }
    }
}

/// Where the keyboard is in the history box. At rest the box is faint and the keyboard is in
/// the palette's list; ↑ on that list's first row, or Ctrl+R, brings it into the box.
#[derive(Default, Debug)]
pub(crate) struct HistoryCursor {
    active: bool,
    /// Ctrl+R's search, which the query line says it is in.
    searching: bool,
    /// The entry the keyboard is on, by its place among those that match, newest first.
    highlighted: usize,
    /// The query the highlight was put on: typing puts it back on the newest match.
    query: String,
}

impl HistoryCursor {
    pub(super) fn active(&self) -> bool {
        self.active
    }

    pub(super) fn searching(&self) -> bool {
        self.searching
    }

    pub(super) fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// Follow what is typed: a different query matches different entries, and the newest of
    /// them is the one the keyboard is on.
    fn follow_query(&mut self, query: &str) {
        if self.query != query {
            self.query = query.to_string();
            self.highlighted = 0;
        }
    }

    /// ↑: one entry older in the box, or into the box from the first row of the list. Whether
    /// the key was the box's.
    fn up(&mut self, on_first_row: bool, matches: usize) -> bool {
        if self.active {
            self.highlighted = (self.highlighted + 1).min(matches.saturating_sub(1));
            return true;
        }
        if !on_first_row {
            return false;
        }
        self.active = true;
        self.highlighted = 0;
        true
    }

    /// ↓: one entry newer, or out of the box from the newest. Whether the key was the box's.
    fn down(&mut self) -> bool {
        if !self.active {
            return false;
        }
        match self.highlighted {
            0 => self.rest(),
            _ => self.highlighted -= 1,
        }
        true
    }

    /// Ctrl+R: into the box's search on the newest match, or once in it the next older match.
    fn search_older(&mut self, matches: usize) {
        if self.searching {
            self.highlighted = (self.highlighted + 1).min(matches.saturating_sub(1));
            return;
        }
        self.active = true;
        self.searching = true;
        self.highlighted = 0;
    }

    /// Back to faint, the keyboard in the palette's list - Esc, ↓ from the newest, or an entry
    /// run. What is typed stays.
    fn rest(&mut self) {
        self.active = false;
        self.searching = false;
        self.highlighted = 0;
    }
}

/// One entry as the box lists it: what it is now, its row, and what running it does.
pub(super) struct ShownEntry {
    /// What the entry records when it is run again - a command's with its title as the list
    /// has it now.
    thing: HistoryThing,
    row: Row,
    run: HistoryRun,
}

/// What running an entry does.
enum HistoryRun {
    /// What the palette's own rows run.
    Action(CommandAction),
    /// A line of shell, put back on the query line and run under the palette - see
    /// [`super::shell`].
    #[cfg(not(target_arch = "wasm32"))]
    Shell(String),
}

/// Whether an entry's thing is there to run.
enum Reach {
    Runs(Row, HistoryRun),
    /// Not to be run from this window now, though it may be later: a file of a repo that has
    /// no review open, a quick task before the board is read.
    Hidden,
    /// No longer there: hidden, and dropped with the next write.
    Gone,
}

impl App {
    /// Read the project's history, and the board's cards that say how its quick tasks are
    /// doing, for the palette being opened. What was read last stays up until they come.
    pub(crate) fn load_palette_history(&mut self) {
        let session_id = self.model.root_session_id.clone();
        if session_id.is_empty() {
            return;
        }
        self.tasks.spawn_keyed(
            Some("palette-history".to_string()),
            move |backend| {
                let entries = backend.palette_history(&session_id)?;
                let cards = backend.list_tasks(&session_id)?;
                Ok((entries, cards))
            },
            |model, result| match result {
                Ok((entries, cards)) => {
                    model.palette.history.entries = Some(entries);
                    model.palette.history.cards = Some(cards);
                }
                Err(error) => {
                    model.error(format!("could not read the palette's history: {error:#}"))
                }
            },
        );
    }

    /// Keep a pick from the menu bar in the history as the palette's command that does the
    /// same, when it offers one: the menu bar's own items are not what the palette lists.
    pub(crate) fn record_menu_pick(&mut self, action: &CommandAction) {
        let key = action_key(action);
        let Some(command) = commands_for(self)
            .into_iter()
            .find(|command| action_key(&command.action) == key)
        else {
            return;
        };
        if let Some(thing) = thing_of(self, PaletteMode::Commands, &command) {
            self.record_in_history(thing);
        }
    }

    /// Keep `thing` in the project's history as just run, and drop with it what is no longer
    /// there.
    pub(super) fn record_in_history(&mut self, thing: HistoryThing) {
        let record = HistoryRecord {
            entry: HistoryEntry {
                thing,
                ran_at_unix: crate::native::messages::now_unix(),
            },
            gone: gone_things(self),
        };
        self.model.palette.history.add(record.clone());
        let session_id = self.model.root_session_id.clone();
        self.tasks.spawn(
            move |backend| backend.record_in_palette_history(&session_id, &record),
            |model, result| model.report(result, "could not keep it in the palette's history"),
        );
    }
}

/// Add a record made elsewhere - a quick task's, made once its card is - to the entries the
/// window holds.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn add_recorded(model: &mut crate::native::model::Model, record: HistoryRecord) {
    model.palette.history.add(record);
}

/// The things of the entries no longer there, to drop with the next write.
pub(crate) fn gone_things(app: &App) -> Vec<HistoryThing> {
    let Some(entries) = &app.model.palette.history.entries else {
        return Vec::new();
    };
    let commands = commands_for(app);
    let palette = app.palette_of();
    entries
        .iter()
        .filter(|entry| {
            matches!(
                reach_of(app, &commands, &palette, &entry.thing),
                Reach::Gone
            )
        })
        .map(|entry| entry.thing.clone())
        .collect()
}

/// What the palette's history records for a row of the list run in `mode`: the command, the
/// file or the line it opens. Nothing for the rows tied to a caret - a rename, a code action,
/// a place - nor for a quick task, which is recorded once its card is made.
pub(super) fn thing_of(app: &App, mode: PaletteMode, command: &Command) -> Option<HistoryThing> {
    match mode {
        PaletteMode::Commands | PaletteMode::Files | PaletteMode::Contents => {}
        PaletteMode::Rename | PaletteMode::Places | PaletteMode::CodeActions => return None,
    }
    match &command.action {
        CommandAction::OpenPane(OpenPaneRequest::File {
            session_id,
            file_path,
            at,
        }) => {
            // The searches ask the server by session, so a file can be found in a review
            // whose repo has not been read into the window yet - and kept nowhere, with no
            // path to keep it by.
            let repo_path = app
                .model
                .review_ref(session_id)
                .and_then(|review| review.payload.as_ref())?
                .repo_path
                .clone();
            Some(match at {
                None => HistoryThing::File {
                    repo_path,
                    file_path: file_path.clone(),
                },
                Some(at) => HistoryThing::Line {
                    repo_path,
                    file_path: file_path.clone(),
                    line_number: at.line,
                    text: command.title.clone(),
                },
            })
        }
        CommandAction::LinkTaskFile { .. }
        | CommandAction::RenameSymbol
        | CommandAction::RenameTo(_)
        | CommandAction::FindPlaces(_)
        | CommandAction::FormatFile
        | CommandAction::CodeActions
        | CommandAction::ApplyCodeAction(_)
        | CommandAction::ToggleBlame => None,
        #[cfg(not(target_arch = "wasm32"))]
        CommandAction::StartQuickTask { .. } => None,
        action => Some(HistoryThing::Command {
            action: action_key(action),
            title: command.title.clone(),
            description: command.description.clone(),
        }),
    }
}

/// How the history knows a command - see [`HistoryThing::Command`].
fn action_key(action: &CommandAction) -> String {
    format!("{action:?}")
}

/// The entries the box lists for what is typed, newest first. After `!` only the lines of
/// shell, matched against what follows it.
pub(super) fn shown_entries(app: &App) -> Vec<ShownEntry> {
    let Some(entries) = &app.model.palette.history.entries else {
        return Vec::new();
    };
    let query = app.model.palette.query();
    #[cfg(not(target_arch = "wasm32"))]
    let shell_line = super::shell::typed_line(app);
    #[cfg(target_arch = "wasm32")]
    let shell_line: Option<&str> = None;
    let terms = terms_of(shell_line.unwrap_or(query));
    let commands = commands_for(app);
    let palette = app.palette_of();
    let now = crate::native::messages::now_unix();
    entries
        .iter()
        .rev()
        .filter(|entry| shell_line.is_none() || matches!(entry.thing, HistoryThing::Shell { .. }))
        .filter_map(|entry| {
            let Reach::Runs(mut row, run) = reach_of(app, &commands, &palette, &entry.thing) else {
                return None;
            };
            let searchable = format!("{} {}", row.title, row.description).to_lowercase();
            if !terms.iter().all(|term| searchable.contains(term.as_str())) {
                return None;
            }
            let needles: Vec<&str> = terms.iter().map(String::as_str).collect();
            row.marks = marks_of(&row.title, &needles)
                .into_iter()
                .filter(|mark| {
                    row.inked
                        .iter()
                        .all(|(inked, _)| mark.end <= inked.start || inked.end <= mark.start)
                })
                .collect();
            row.asides.push(Aside {
                text: age_text(now.saturating_sub(entry.ran_at_unix)),
                ink: None,
                boxed: false,
            });
            Some(ShownEntry {
                thing: refreshed(&entry.thing, &row),
                row,
                run,
            })
        })
        .collect()
}

/// Whether there is a history to draw above the palette at all, whatever is typed - which is
/// what keeps the palette where it is as the box's rows come and go.
pub(super) fn has_history(app: &App) -> bool {
    app.model.palette.mode == PaletteMode::Commands
        && app
            .model
            .palette
            .history
            .entries
            .as_ref()
            .is_some_and(|entries| !entries.is_empty())
}

/// A command's entry with the title and description its row has now.
fn refreshed(thing: &HistoryThing, row: &Row) -> HistoryThing {
    match thing {
        HistoryThing::Command { action, .. } => HistoryThing::Command {
            action: action.clone(),
            title: row.title.clone(),
            description: row.description.clone(),
        },
        other => other.clone(),
    }
}

/// Whether `thing` is there to run, and its row and what running it does if it is.
fn reach_of(app: &App, commands: &[Command], palette: &Palette, thing: &HistoryThing) -> Reach {
    match thing {
        HistoryThing::Command { action, .. } => {
            match commands
                .iter()
                .find(|command| action_key(&command.action) == *action)
            {
                Some(command) => Reach::Runs(
                    super::drawing::row_of(command),
                    HistoryRun::Action(command.action.clone()),
                ),
                None => Reach::Gone,
            }
        }
        HistoryThing::File {
            repo_path,
            file_path,
        } => match session_of_repo(app, repo_path) {
            Some(session_id) => Reach::Runs(
                tagged(
                    Row {
                        title: super::rows::file_name_of(file_path).to_string(),
                        description: file_path.clone(),
                        ..Row::default()
                    },
                    "file",
                ),
                HistoryRun::Action(CommandAction::OpenPane(OpenPaneRequest::File {
                    session_id,
                    file_path: file_path.clone(),
                    at: None,
                })),
            ),
            None => Reach::Hidden,
        },
        HistoryThing::Line {
            repo_path,
            file_path,
            line_number,
            text,
        } => match session_of_repo(app, repo_path) {
            Some(session_id) => Reach::Runs(
                tagged(
                    Row {
                        title: text.clone(),
                        prefix: line_number.to_string(),
                        description: file_path.clone(),
                        ..Row::default()
                    },
                    "line",
                ),
                HistoryRun::Action(CommandAction::OpenPane(OpenPaneRequest::File {
                    session_id,
                    file_path: file_path.clone(),
                    at: Some(crate::native::panes::OpenAt {
                        line: *line_number,
                        query: String::new(),
                    }),
                })),
            ),
            None => Reach::Hidden,
        },
        #[cfg(not(target_arch = "wasm32"))]
        HistoryThing::Shell { line } => {
            if !app.model.project.palette_shell() {
                return Reach::Hidden;
            }
            Reach::Runs(
                tagged(super::shell::shell_row(line, palette), "shell"),
                HistoryRun::Shell(line.clone()),
            )
        }
        #[cfg(not(target_arch = "wasm32"))]
        HistoryThing::QuickTask {
            task_id,
            title,
            agent,
        } => quick_task_reach(
            app.model.palette.history.cards.as_deref(),
            task_id,
            title,
            *agent,
            palette,
        ),
        // A browser's window starts no process and tells no shell a line.
        #[cfg(target_arch = "wasm32")]
        HistoryThing::Shell { .. } | HistoryThing::QuickTask { .. } => {
            let _ = palette;
            Reach::Hidden
        }
    }
}

/// `row` with the small tag that says which kind of entry it is, for the kinds a row does not
/// tell apart by itself.
fn tagged(mut row: Row, tag: &str) -> Row {
    row.asides.push(Aside {
        text: tag.to_string(),
        ink: None,
        boxed: true,
    });
    row
}

/// The open review of the repo at `repo_path`, which a file of it is opened in.
fn session_of_repo(app: &App, repo_path: &str) -> Option<String> {
    app.model
        .reviews
        .iter()
        .find(|(_, review)| {
            review
                .payload
                .as_ref()
                .is_some_and(|payload| payload.repo_path == repo_path)
        })
        .map(|(session_id, _)| session_id.clone())
}

/// A quick task's entry: there while its card is open and not done, its row saying how its
/// agent is doing, and running it bringing that agent's shell to the front - or the task's
/// page, once no agent is running on it.
#[cfg(not(target_arch = "wasm32"))]
fn quick_task_reach(
    cards: Option<&[TaskView]>,
    task_id: &str,
    title: &str,
    agent: crate::api::AgentKind,
    palette: &Palette,
) -> Reach {
    let Some(cards) = cards else {
        return Reach::Hidden;
    };
    let Some(card) = cards.iter().find(|card| {
        card.id == task_id && card.status.as_str() != crate::moontasks::store::ENDS_QUICK_TASKS_IN
    }) else {
        return Reach::Gone;
    };
    let running = card
        .resources
        .iter()
        .filter(|run| {
            run.kind == crate::moontasks::TaskResourceKind::Agent
                && run.running
                && run.terminal_id.is_some()
        })
        .max_by_key(|run| run.started_at_unix);
    let activity = running.map_or(
        Activity::Ended,
        crate::native::board::resources::activity_of,
    );
    let (_, state, ink) = AGENT_STATES
        .iter()
        .find(|(of, _, _)| *of == activity)
        .expect("every activity has a state to say");
    let mut row = super::drawing::quick_task_entry_row(title, agent);
    row.asides.push(Aside {
        text: state.to_string(),
        ink: Some(ink(palette)),
        boxed: false,
    });
    let action = match running {
        Some(run) => CommandAction::OpenPane(OpenPaneRequest::AttachTerminal {
            terminal_id: run
                .terminal_id
                .clone()
                .expect("only a run with a shell was picked"),
            command: Some(run.agent),
            task_id: Some(card.id.clone()),
        }),
        None => CommandAction::OpenPane(OpenPaneRequest::TaskStart {
            task_id: card.id.clone(),
            title: card.title.clone(),
        }),
    };
    Reach::Runs(row, HistoryRun::Action(action))
}

/// How long ago an entry was run, as the box says it.
fn age_text(secs: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const TWO_DAYS: u64 = 2 * DAY;
    match secs {
        0..MINUTE => "now".to_string(),
        MINUTE..HOUR => format!("{} min", secs / MINUTE),
        HOUR..DAY => format!("{} h", secs / HOUR),
        DAY..TWO_DAYS => "yesterday".to_string(),
        _ => format!("{} days", secs / DAY),
    }
}

/// Esc while the keyboard is in the box puts the box back to faint, rather than the palette
/// away. Taken before the palette asks whether it is being dismissed.
pub(super) fn take_escape(app: &mut App, ctx: &egui::Context) {
    if app.model.palette.history.cursor.active()
        && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
    {
        app.model.palette.history.cursor.rest();
    }
}

/// The keys the box takes before the palette's list reads them: ↑ and ↓ while in the box, ↑
/// from the list's first row, Ctrl+R from anywhere, and Enter while in the box - the entry it
/// runs, by its place in `shown`.
pub(super) fn take_keys(
    app: &mut App,
    ctx: &egui::Context,
    shown: &[ShownEntry],
    on_first_row: bool,
) -> Option<usize> {
    let cursor = &mut app.model.palette.history.cursor;
    cursor.follow_query(app.model.palette.input.query());
    if !has_history(app) {
        return None;
    }
    let cursor = &mut app.model.palette.history.cursor;
    let matches = shown.len();
    ctx.input_mut(|input| {
        if input.consume_key(Modifiers::CTRL, Key::R) {
            cursor.search_older(matches);
        }
        if input.key_pressed(Key::ArrowUp) && cursor.up(on_first_row, matches) {
            input.consume_key(Modifiers::NONE, Key::ArrowUp);
        }
        if input.key_pressed(Key::ArrowDown) && cursor.down() {
            input.consume_key(Modifiers::NONE, Key::ArrowDown);
        }
        if cursor.active() && matches > 0 && input.consume_key(Modifiers::NONE, Key::Enter) {
            return Some(cursor.highlighted().min(matches - 1));
        }
        None
    })
}

/// Draw the box above the palette. The entry clicked, by its place in `shown`.
pub(super) fn draw(app: &mut App, ctx: &egui::Context, shown: &[ShownEntry]) -> Option<usize> {
    if !has_history(app) {
        return None;
    }
    let rows: Vec<Row> = shown.iter().map(|entry| entry.row.clone()).collect();
    let cursor = &app.model.palette.history.cursor;
    let history = HistoryBox {
        rows: &rows,
        label: HISTORY_LABEL,
        empty: NOTHING_MATCHES,
        active: cursor.active(),
        highlighted: cursor.highlighted().min(rows.len().saturating_sub(1)),
        description_kept: Kept::End,
    };
    app.model.palette.input.show_history(ctx, &history)
}

/// Run an entry again: it is recorded as the newest, the box goes back to faint, and the
/// palette is put away for what it opens - except for a line of shell, which runs under it.
pub(super) fn run(app: &mut App, ctx: &egui::Context, entry: ShownEntry) {
    app.model.palette.history.cursor.rest();
    match entry.run {
        HistoryRun::Action(action) => {
            app.record_in_history(entry.thing);
            app.model.palette.dismiss();
            app.pending_action = Some(action);
        }
        #[cfg(not(target_arch = "wasm32"))]
        HistoryRun::Shell(line) => {
            app.model.palette.input.set_query(&format!("! {line}"));
            super::shell::run(app, ctx, &line);
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = ctx;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ↑ moves the highlight in the list until it is on the first row; from there it goes
    /// into the box, on the newest entry, and on to older ones. ↓ comes back the same way.
    #[test]
    fn up_from_the_first_row_goes_into_the_history() {
        let mut cursor = HistoryCursor::default();
        assert!(!cursor.up(false, 3), "the list keeps ↑ off its first row");
        assert!(!cursor.active());

        assert!(cursor.up(true, 3));
        assert!(cursor.active());
        assert_eq!(cursor.highlighted(), 0);
        cursor.up(true, 3);
        cursor.up(true, 3);
        cursor.up(true, 3);
        assert_eq!(
            cursor.highlighted(),
            2,
            "the oldest match is as far as it goes"
        );

        assert!(cursor.down());
        assert!(cursor.down());
        assert!(cursor.active());
        assert!(cursor.down(), "↓ from the newest is the box's too");
        assert!(!cursor.active());
        assert!(!cursor.down(), "at rest ↓ is the list's");
    }

    /// Ctrl+R starts on the newest match, each Ctrl+R after goes one match older, and typing
    /// goes back to the newest match of what is typed now.
    #[test]
    fn ctrl_r_steps_to_older_matches() {
        let mut cursor = HistoryCursor::default();
        cursor.follow_query("");
        cursor.search_older(3);
        assert!(cursor.active() && cursor.searching());
        assert_eq!(cursor.highlighted(), 0);
        cursor.search_older(3);
        cursor.search_older(3);
        assert_eq!(cursor.highlighted(), 2);

        cursor.follow_query("git");
        assert_eq!(cursor.highlighted(), 0);
        assert!(cursor.searching());

        cursor.rest();
        assert!(!cursor.active() && !cursor.searching());
    }

    #[test]
    fn ages_read_the_way_the_box_says_them() {
        assert_eq!(age_text(5), "now");
        assert_eq!(age_text(3 * 60 + 5), "3 min");
        assert_eq!(age_text(90 * 60), "1 h");
        assert_eq!(age_text(30 * 3600), "yesterday");
        assert_eq!(age_text(50 * 3600), "2 days");
    }
}
