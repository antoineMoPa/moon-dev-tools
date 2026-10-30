//! Sections - the command list as it is drawn while something is typed: the commands that
//! match, then the quick task, then the start of what the file finder and the content search
//! find for the same query.
//!
//! The two searches are the ones the Files and Contents modes run, started once typing
//! pauses. Each section shows the first few of its matches, in the places they first came in,
//! with placeholders keeping its room until its search is over - so the list does not grow
//! and shrink, nor move under the highlight, as the matches stream in.

use std::ops::Range;

use egui_command_palette::Row;

use crate::{api::ContentMatch, native::app::App};

#[cfg(not(target_arch = "wasm32"))]
use super::drawing::quick_task_row;
use super::drawing::row_of;
use super::rows::{asked_of, content_row, file_row, start_both_searches};
use super::{Command, PaletteMode, Search, SearchRequest, commands_for, ranked};

/// How long typing has to pause before the searches start on what is typed, in seconds.
const SEARCH_PAUSE: f64 = 0.12;

/// How many letters a query needs before the repo is searched for it: one letter is in
/// nearly every file.
const SEARCH_MIN_CHARS: usize = 2;

/// The sections of the command list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SectionKind {
    Commands,
    Task,
    Files,
    Contents,
}

/// What a section is, as the list draws it.
struct SectionLayout {
    kind: SectionKind,
    label: &'static str,
    /// How many of its rows the section shows; `None` shows them all.
    shown_at_most: Option<usize>,
    /// The mode that lists the whole of what the section shows the start of, which Tab on
    /// one of its rows opens.
    opens: Option<PaletteMode>,
}

/// The sections in the order they are listed in, which never changes: where to look for a
/// file is learnt once. The ones that are there at once come first, so the searches growing
/// below them move nothing.
const SECTION_LAYOUTS: [SectionLayout; 4] = [
    SectionLayout {
        kind: SectionKind::Commands,
        label: "Commands",
        shown_at_most: None,
        opens: None,
    },
    SectionLayout {
        kind: SectionKind::Task,
        label: "Task",
        shown_at_most: None,
        opens: None,
    },
    SectionLayout {
        kind: SectionKind::Files,
        label: "Files",
        shown_at_most: Some(3),
        opens: Some(PaletteMode::Files),
    },
    SectionLayout {
        kind: SectionKind::Contents,
        label: "Contents",
        shown_at_most: Some(2),
        opens: Some(PaletteMode::Contents),
    },
];

fn layout_of(kind: SectionKind) -> &'static SectionLayout {
    SECTION_LAYOUTS
        .iter()
        .find(|layout| layout.kind == kind)
        .expect("every section has a layout")
}

/// How many rows a searched section shows at most.
fn shown_at_most(kind: SectionKind) -> usize {
    layout_of(kind)
        .shown_at_most
        .expect("a searched section shows a few of its matches")
}

/// Which row the highlight is on, by what the row is rather than where it is: a row arriving
/// above it moves it down the list, and the highlight goes with it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct RowKey {
    /// `None` for a list that is not in sections.
    section: Option<SectionKind>,
    title: String,
    description: String,
}

/// The matches a section has shown for the search it is showing, in the places it showed
/// them. A later match the search ranks higher takes an empty place below them, not one of
/// theirs.
struct ShownMatches<T> {
    /// The ticket of the search they came from.
    ticket: u64,
    matches: Vec<T>,
}

impl<T> Default for ShownMatches<T> {
    fn default() -> Self {
        Self {
            ticket: 0,
            matches: Vec::new(),
        }
    }
}

/// What the sections keep from one frame to the next.
#[derive(Default)]
pub(crate) struct SectionsState {
    /// What was typed when the query last changed, and when, on egui's clock: the searches
    /// wait for typing to pause.
    typed: Option<(String, f64)>,
    /// The row the highlight is on, in whichever mode - the lists of the searches move
    /// under it too, as their matches are ranked in.
    highlighted: Option<RowKey>,
    files: ShownMatches<String>,
    contents: ShownMatches<ContentMatch>,
}

/// One part of the list, ready to draw: its header, if the list is in sections, and its
/// rows, each with the command it runs.
pub(super) struct Listed {
    pub(super) kind: Option<SectionKind>,
    pub(super) header: Option<HeaderText>,
    pub(super) commands: Vec<Command>,
    pub(super) rows: Vec<Row>,
    /// Rows still to come - see `egui_command_palette::Section::placeholders`.
    pub(super) placeholders: usize,
}

/// What a section's header says - see `egui_command_palette::SectionHeader`.
pub(super) struct HeaderText {
    pub(super) label: &'static str,
    pub(super) count: String,
    pub(super) busy: bool,
    pub(super) more: Option<String>,
}

impl Listed {
    /// A list not in sections, as every mode but the commands draws it.
    pub(super) fn bare(commands: Vec<Command>) -> Self {
        Self {
            kind: None,
            header: None,
            rows: commands.iter().map(row_of).collect(),
            commands,
            placeholders: 0,
        }
    }

    fn key_of(&self, command: &Command) -> RowKey {
        RowKey {
            section: self.kind,
            title: command.title.clone(),
            description: command.description.clone(),
        }
    }
}

/// Every row of `listed` by its key, in the order the keyboard walks them.
fn keys_of(listed: &[Listed]) -> impl Iterator<Item = RowKey> + '_ {
    listed
        .iter()
        .flat_map(|part| part.commands.iter().map(|command| part.key_of(command)))
}

/// Put the palette's highlight back on the row it was on, wherever the row now is.
pub(super) fn restore_highlight(app: &mut App, listed: &[Listed]) {
    let Some(was_on) = &app.model.palette.sections.highlighted else {
        return;
    };
    if let Some(index) = keys_of(listed).position(|key| key == *was_on) {
        app.model.palette.input.highlight(index);
    }
}

/// Remember which row the highlight is on, for [`restore_highlight`] on the next frame.
pub(super) fn remember_highlight(app: &mut App, listed: &[Listed]) {
    let index = app.model.palette.input.highlighted();
    app.model.palette.sections.highlighted = keys_of(listed).nth(index);
}

/// The mode Tab opens: the whole list of the searched section the highlight is in.
pub(super) fn section_to_open(app: &App) -> Option<PaletteMode> {
    let section = app.model.palette.sections.highlighted.as_ref()?.section?;
    layout_of(section).opens
}

/// The mode the header of the section at `index` of `listed` opens when its right side is
/// clicked.
pub(super) fn opened_by_header(listed: &[Listed], index: usize) -> PaletteMode {
    let kind = listed[index]
        .kind
        .expect("only a section has a header to click");
    layout_of(kind)
        .opens
        .expect("only a searched section's header offers more")
}

/// Whether the repo is searched for what is typed: a query of a few letters, in a window
/// with a repo.
fn searches_repo(app: &App) -> bool {
    app.model.palette.query().trim().chars().count() >= SEARCH_MIN_CHARS
        && !app.model.palette.search_session_id.is_empty()
}

/// Keep the two searches on what is typed: a key typed stops the ones running, and once
/// typing has paused both start again on the query as it stands.
pub(super) fn refresh_section_searches(app: &mut App, ctx: &egui::Context) {
    let now = ctx.input(|input| input.time);
    let query = app.model.palette.query().to_string();
    let typed_since = app
        .model
        .palette
        .sections
        .typed
        .as_ref()
        .is_none_or(|(typed, _)| *typed != query);
    // The lists go with the searches they came from: one stopped part-way would pass for
    // the whole answer if what is typed came back to its query.
    if typed_since {
        let palette = &mut app.model.palette;
        palette.sections.typed = Some((query, now));
        palette.stop_searches();
        palette.files = Search::default();
        palette.contents = Search::default();
    }
    if !searches_repo(app) {
        return;
    }
    let asked = asked_of(app);
    let palette = &app.model.palette;
    if palette.files.searched.as_ref() == Some(&asked)
        && palette.contents.searched.as_ref() == Some(&asked)
    {
        return;
    }
    let (_, typed_at) = palette
        .sections
        .typed
        .as_ref()
        .expect("what was typed was noted above");
    let waited = now - typed_at;
    if waited < SEARCH_PAUSE {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SEARCH_PAUSE - waited));
        return;
    }
    start_both_searches(app, &asked);
}

/// Take the matches that came in since the last frame into the sections' places.
pub(super) fn keep_shown_matches(app: &mut App) {
    let palette = &mut app.model.palette;
    keep_shown(
        &mut palette.sections.files,
        &palette.files,
        shown_at_most(SectionKind::Files),
    );
    keep_shown(
        &mut palette.sections.contents,
        &palette.contents,
        shown_at_most(SectionKind::Contents),
    );
}

/// Fill the empty places of `shown` from what `search` has found, a match already shown
/// keeping its place. A search started since empties them all.
fn keep_shown<T: Clone + PartialEq>(
    shown: &mut ShownMatches<T>,
    search: &Search<T>,
    places: usize,
) {
    if shown.ticket != search.ticket {
        *shown = ShownMatches {
            ticket: search.ticket,
            matches: Vec::new(),
        };
    }
    for found in &search.matches {
        if shown.matches.len() == places {
            break;
        }
        if !shown.matches.contains(found) {
            shown.matches.push(found.clone());
        }
    }
}

/// The command list in its sections, in their order. With nothing typed it is the plain
/// list of every command.
pub(super) fn command_sections(app: &App) -> Vec<Listed> {
    let query = app.model.palette.query();
    // A line of shell lists nothing else.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(line) = super::shell::typed_line(app) {
        return vec![super::shell::listed(app, line)];
    }
    let commands = ranked(commands_for(app), query);
    if query.trim().is_empty() {
        return vec![Listed::bare(commands)];
    }
    let mut listed = Vec::new();
    listed.extend(whole_section(SectionKind::Commands, commands));
    #[cfg(not(target_arch = "wasm32"))]
    listed.extend(task_section(app, query));
    if searches_repo(app) {
        let asked = asked_of(app);
        listed.extend(file_section(app, &asked));
        listed.extend(content_section(app, &asked));
    }
    listed
}

/// What is typed as the title of a quick task: one row, with a chip per agent.
#[cfg(not(target_arch = "wasm32"))]
fn task_section(app: &App, query: &str) -> Option<Listed> {
    let task = crate::native::board::quick_task::offered(app, query)?;
    let command = task.command();
    Some(Listed {
        kind: Some(SectionKind::Task),
        header: Some(HeaderText {
            label: layout_of(SectionKind::Task).label,
            count: "1".to_string(),
            busy: false,
            more: None,
        }),
        rows: vec![quick_task_row(&task, &command)],
        commands: vec![command],
        placeholders: 0,
    })
}

/// The agent the chip at `choice` of the row at `row` stands for. Only the task row has chips.
#[cfg(not(target_arch = "wasm32"))]
fn task_agent_at(
    app: &App,
    listed: &[Listed],
    row: usize,
    choice: usize,
) -> (
    crate::native::board::quick_task::QuickTask,
    crate::api::AgentKind,
) {
    let section = listed
        .iter()
        .flat_map(|part| part.commands.iter().map(|_| part.kind))
        .nth(row)
        .expect("the row with the chip is listed");
    assert_eq!(
        section,
        Some(SectionKind::Task),
        "only the task row has chips"
    );
    let task = crate::native::board::quick_task::offered(app, app.model.palette.query())
        .expect("the task row was listed for what is typed");
    let agent = task.agents[choice];
    (task, agent)
}

/// What a click on a chip of the task row runs: the task, started with that chip's agent.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn task_started_from_chip(
    app: &App,
    listed: &[Listed],
    row: usize,
    choice: usize,
) -> super::CommandAction {
    let (task, agent) = task_agent_at(app, listed, row, choice);
    task.started_with(agent)
}

/// A section that shows every one of its rows, none of them still to come. None without
/// rows.
fn whole_section(kind: SectionKind, commands: Vec<Command>) -> Option<Listed> {
    if commands.is_empty() {
        return None;
    }
    Some(Listed {
        kind: Some(kind),
        header: Some(HeaderText {
            label: layout_of(kind).label,
            count: commands.len().to_string(),
            busy: false,
            more: None,
        }),
        rows: commands.iter().map(row_of).collect(),
        commands,
        placeholders: 0,
    })
}

/// The files found for what is typed, the name each is read by marked where it holds a
/// typed term.
fn file_section(app: &App, asked: &SearchRequest) -> Option<Listed> {
    let terms: Vec<&str> = asked.query.split_whitespace().collect();
    searched_section(
        SectionKind::Files,
        &app.model.palette.files,
        &app.model.palette.sections.files,
        asked,
        |file_path| {
            let command = file_row(app, file_path);
            let row = Row {
                marks: marks_of(&command.title, &terms),
                ..row_of(&command)
            };
            (command, row)
        },
    )
}

/// The lines found for what is typed: the line's number, the line with what was typed
/// marked, and the file it is in under it.
fn content_section(app: &App, asked: &SearchRequest) -> Option<Listed> {
    searched_section(
        SectionKind::Contents,
        &app.model.palette.contents,
        &app.model.palette.sections.contents,
        asked,
        |found| {
            let command = content_row(app, found, &asked.query);
            let row = Row {
                prefix: found.line_number.to_string(),
                marks: marks_of(&command.title, &[asked.query.as_str()]),
                description: found.file_path.clone(),
                ..row_of(&command)
            };
            (command, row)
        },
    )
}

/// The section of one of the searches. Until the search for what is typed has started, or
/// while it runs, the places it has not filled are placeholders. Once it is over a section
/// without matches is left out.
fn searched_section<T>(
    kind: SectionKind,
    search: &Search<T>,
    shown: &ShownMatches<T>,
    asked: &SearchRequest,
    row_of_match: impl Fn(&T) -> (Command, Row),
) -> Option<Listed> {
    let places = shown_at_most(kind);
    let label = layout_of(kind).label;
    let header = |count: String, busy: bool, more: Option<String>| HeaderText {
        label,
        count,
        busy,
        more,
    };
    // Waiting for typing to pause, or what is listed was found for an earlier query.
    if search.searched.as_ref() != Some(asked) {
        return Some(Listed {
            kind: Some(kind),
            header: Some(header("…".to_string(), true, None)),
            commands: Vec::new(),
            rows: Vec::new(),
            placeholders: places,
        });
    }
    if let Some(error) = &search.error {
        return Some(Listed {
            kind: Some(kind),
            header: Some(header(String::new(), false, Some(error.clone()))),
            commands: Vec::new(),
            rows: Vec::new(),
            placeholders: 0,
        });
    }
    if search.done && search.matches.is_empty() {
        return None;
    }
    let found = search.matches.len();
    let plus = if search.truncated { "+" } else { "" };
    let count = if search.done {
        format!("{found}{plus}")
    } else {
        format!("{found}…")
    };
    let more = (found > places).then(|| format!("+{}{plus} · Tab", found - places));
    let (commands, rows) = shown.matches.iter().map(row_of_match).unzip();
    Some(Listed {
        kind: Some(kind),
        header: Some(header(count, !search.done, more)),
        commands,
        rows,
        placeholders: if search.done {
            0
        } else {
            places - shown.matches.len()
        },
    })
}

/// Where in `text` any of `needles` is, ignoring the case of ASCII letters, in order and
/// with overlapping places joined.
pub(super) fn marks_of(text: &str, needles: &[&str]) -> Vec<Range<usize>> {
    // ASCII lower-casing keeps every byte where it was, so a place found in the lower-cased
    // text is the same place in the text.
    let hay = text.to_ascii_lowercase();
    let mut found: Vec<Range<usize>> = needles
        .iter()
        .filter(|needle| !needle.is_empty())
        .flat_map(|needle| {
            let needle = needle.to_ascii_lowercase();
            hay.match_indices(&needle)
                .map(|(at, _)| at..at + needle.len())
                .collect::<Vec<_>>()
        })
        .collect();
    found.sort_by_key(|mark| mark.start);
    let mut joined: Vec<Range<usize>> = Vec::new();
    for mark in found {
        match joined.last_mut() {
            Some(last) if mark.start <= last.end => last.end = last.end.max(mark.end),
            _ => joined.push(mark),
        }
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_join_where_they_overlap_and_ignore_case() {
        assert_eq!(marks_of("AgentAgent", &["agent"]), vec![0..10]);
        assert_eq!(marks_of("Agent agent", &["agent"]), vec![0..5, 6..11]);
        assert_eq!(marks_of("palette.rs", &["pal", "alet"]), vec![0..5]);
        assert!(marks_of("palette.rs", &["zz"]).is_empty());
    }

    /// A match the search ranks higher once more of the repo is read takes an empty place,
    /// below the ones already shown, and none once they are all taken.
    #[test]
    fn a_shown_match_keeps_its_place_as_more_come_in() {
        let mut shown = ShownMatches::default();
        let mut search = Search::<String> {
            ticket: 4,
            matches: vec!["src/deep/b.rs".to_string()],
            ..Search::default()
        };
        keep_shown(&mut shown, &search, 2);
        search.matches.insert(0, "a.rs".to_string());
        keep_shown(&mut shown, &search, 2);
        assert_eq!(shown.matches, ["src/deep/b.rs", "a.rs"]);
        search.matches.insert(0, "0.rs".to_string());
        keep_shown(&mut shown, &search, 2);
        assert_eq!(shown.matches, ["src/deep/b.rs", "a.rs"]);
    }
}
