//! The command palette: everything the workspace can open, searchable.
//!
//! Everything ⌘⇧P offers, in one list.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use egui::{Align2, Color32, CornerRadius, Key, RichText, Stroke, StrokeKind, vec2};
use egui_frames::DropSide;

use crate::{
    api::{AgentKind, SearchProgress, SearchScope},
    native::{
        app::App,
        bindings::{self, Action},
        model::Model,
        panes::{OpenPaneRequest, PaneKind},
        tasks::ModelEdits,
        theme::{Palette, SMALL_SIZE},
    },
    project::ProjectCommand,
    search::SearchListener,
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
    InstallLaunchers,
    /// Another window of one of the three programs, on its launch screen.
    NewWindow(crate::cli::Frame),
    /// Start this program again on the repo this window is on, and close this window.
    RestartWindow,
    /// Ask the OS which file of the repo to open for editing, and open it in a tab.
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
    /// Start an open extension over from its script - see [`crate::extensions`].
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
}

/// How much of the window's height the palette's rows may take before they scroll. With the
/// palette hung 12% of the way down, this keeps the whole of it on screen.
const ROWS_HEIGHT_OF_SCREEN: f32 = 0.55;

/// The height of one row of the list.
const ROW_HEIGHT: f32 = 34.0;

/// The agents that get a "open X in a terminal" command, when they are installed.
const AGENT_COMMANDS: &[(AgentKind, &str, &str)] = &[
    (
        AgentKind::OpenCode,
        "opencode",
        "Open OpenCode in a terminal",
    ),
    (AgentKind::Claude, "claude", "Open Claude in a terminal"),
    (AgentKind::Codex, "codex", "Open Codex in a terminal"),
];

/// The sides the palette can split the active frame against, and the shell each split opens
/// with - a split has to hold something, and a shell is what the workspace opens beside
/// anything else.
const SPLIT_COMMANDS: &[(DropSide, &str, &str)] = &[
    (
        DropSide::Right,
        "split right",
        "Split this frame and open a shell in the half to the right",
    ),
    (
        DropSide::Bottom,
        "split bottom",
        "Split this frame and open a shell in the half below",
    ),
];

pub(crate) fn commands_for(app: &App) -> Vec<Command> {
    let mut commands = Vec::new();
    let root = app.model.root_session_id.clone();
    // The review the window was launched on is named after its repo, the way the submodule
    // reviews further down are named after theirs.
    let root_repo = root_repo_name(app);

    commands.push(single_pane_command(
        app.model
            .layout
            .find_pane(|pane| pane.reviews(&root))
            .is_some(),
        "review",
        &format!("Open the {root_repo} review"),
        &format!("Bring the {root_repo} review forward"),
        CommandAction::OpenPane(OpenPaneRequest::Review {
            session_id: root.clone(),
            title: "review".to_string(),
        }),
        bindings::chord_of(Action::OpenReview),
    ));
    commands.push(single_pane_command(
        app.model
            .layout
            .find_pane(|pane| pane.kind() == PaneKind::Agents)
            .is_some(),
        "comment agents",
        "Open the comment agent monitor",
        "Bring the comment agent monitor forward",
        CommandAction::OpenPane(OpenPaneRequest::Agents),
        None,
    ));
    commands.push(single_pane_command(
        app.model
            .layout
            .find_pane(|pane| pane.kind() == PaneKind::Tasks)
            .is_some(),
        "moontasks",
        "Open the task board and the agents working on it",
        "Bring the task board forward",
        CommandAction::OpenPane(OpenPaneRequest::Tasks),
        None,
    ));
    commands.extend(work_log_commands());
    commands.push(single_pane_command(
        app.model
            .layout
            .find_pane(|pane| pane.kind() == PaneKind::Submodules)
            .is_some(),
        "submodules",
        "Open the submodules of this repo, and the reviews of the changed ones",
        "Bring the submodules forward",
        CommandAction::OpenPane(OpenPaneRequest::Submodules),
        bindings::chord_of(Action::OpenSubmodules),
    ));
    // The project's own commands, and the pane they are set in. Only the ones the project
    // has set are offered: an item that runs nothing is worse than no item. Build and run
    // needs both halves, which its line saying nothing already answers for.
    let restarts = app.model.project.run_restarts_window();
    for which in [
        ProjectCommand::Build,
        ProjectCommand::Run,
        ProjectCommand::BuildAndRun,
    ] {
        let Some(line) = app.model.project.line(which) else {
            continue;
        };
        let description = match which {
            // A run command of the restart word is not a line of shell - see
            // `crate::project::RESTART_RUN_COMMAND`.
            ProjectCommand::Run if restarts => format!(
                "Start {} again on this repo, and close this window",
                app.frame().command()
            ),
            ProjectCommand::BuildAndRun if restarts => {
                format!("Run {line} in a shell, and restart this window when it ends")
            }
            _ => format!("Run {line} in a shell"),
        };
        commands.push(Command {
            title: which.label().to_string(),
            description,
            action: CommandAction::RunProject(which),
            shortcut: None,
        });
    }
    commands.push(single_pane_command(
        app.model
            .layout
            .find_pane(|pane| pane.kind() == PaneKind::Project)
            .is_some(),
        "project",
        "Open the project settings: the build and run commands",
        "Bring the project settings forward",
        CommandAction::OpenPane(OpenPaneRequest::Project),
        None,
    ));
    // Aimed at the review being read rather than at the window's own: a changed submodule is a
    // review of its own repo, with its own branch to commit, and committing while reading one
    // means that repo. The repo is named on the item, so the list says which one it will be.
    let committing = app.review_in_front();
    let committing_repo = repo_name_of(app, &committing);
    commands.push(single_pane_command(
        app.model
            .layout
            .find_pane(|pane| pane.commits(&committing))
            .is_some(),
        "commit",
        &format!("Commit what is staged in {committing_repo}, and push it"),
        &format!("Bring the {committing_repo} commit pane forward"),
        CommandAction::OpenPane(OpenPaneRequest::Commit {
            session_id: committing,
        }),
        None,
    ));
    commands.push(single_pane_command(
        app.model
            .layout
            .find_pane(|pane| pane.kind() == PaneKind::Messages)
            .is_some(),
        "messages",
        "Open every message this window has posted",
        "Bring the messages forward",
        CommandAction::OpenPane(OpenPaneRequest::Messages),
        None,
    ));
    // Every extension, shipped or the person's own - read again each time, so one written a
    // moment ago is offered without a restart. See `crate::extensions`.
    for extension in crate::extensions::all() {
        let name = extension.name;
        let opens = match extension.about.is_empty() {
            true => format!("Open the {name} extension"),
            false => extension.about,
        };
        let open = app
            .model
            .layout
            .find_pane(|pane| pane.runs_extension(&name))
            .is_some();
        commands.push(single_pane_command(
            open,
            &name,
            &opens,
            &format!("Bring {name} forward"),
            CommandAction::OpenPane(OpenPaneRequest::Extension { name: name.clone() }),
            None,
        ));
        if open {
            commands.push(Command {
                title: format!("restart {name}"),
                description: format!(
                    "Read the {name} extension again and start it over from init, dropping what it holds"
                ),
                action: CommandAction::RestartExtension(name),
                shortcut: None,
            });
        }
    }
    commands.push(Command {
        title: "terminal".to_string(),
        description: "Open a new shell".to_string(),
        action: CommandAction::OpenPane(OpenPaneRequest::Terminal { command: None }),
        shortcut: bindings::chord_of(Action::NewShellTab),
    });
    for (side, title, description) in SPLIT_COMMANDS {
        commands.push(Command {
            title: (*title).to_string(),
            description: (*description).to_string(),
            action: CommandAction::Split(*side),
            shortcut: None,
        });
    }
    commands.push(Command {
        title: "find file".to_string(),
        description: "Open a file of the repo by name, from any directory under it".to_string(),
        action: CommandAction::FindFile,
        shortcut: bindings::chord_of(Action::FindFile),
    });
    commands.push(Command {
        title: "search content".to_string(),
        description: "Find text in the files of the repo, wherever under it they are".to_string(),
        action: CommandAction::SearchContent,
        shortcut: bindings::chord_of(Action::SearchContent),
    });
    // Only while they can do something: a file tab in front with a language server behind it.
    if crate::native::places::front_tab_finds_places(app) {
        for kind in crate::native::places::KINDS {
            commands.push(Command {
                title: kind.command.to_string(),
                description: kind.about.to_string(),
                action: CommandAction::FindPlaces(kind.which),
                shortcut: bindings::chord_of(Action::FindPlaces(kind.which)),
            });
        }
    }
    // Only while it can do something: a file tab in front with a language server behind it.
    if crate::native::formatting::front_tab_formats(app) {
        commands.push(Command {
            title: "format file".to_string(),
            description: "Lay the file out the way its language's formatter does".to_string(),
            action: CommandAction::FormatFile,
            shortcut: bindings::chord_of(Action::FormatFile),
        });
    }
    // Only while it can do something: a file tab in front on a file of the repo's own.
    if crate::native::blame::front_tab_blames(app) {
        let showing = crate::native::blame::front_tab_shows_blame(app);
        commands.push(Command {
            title: if showing { "hide blame" } else { "show blame" }.to_string(),
            description: if showing {
                "Take the column of who last touched each stretch of the file down"
            } else {
                "Show who last touched each stretch of the file, and in which commit, beside the lines"
            }
            .to_string(),
            action: CommandAction::ToggleBlame,
            shortcut: bindings::chord_of(Action::ToggleBlame),
        });
    }
    if crate::native::code_actions::front_tab_has_actions(app) {
        commands.push(Command {
            title: "code actions".to_string(),
            description:
                "Fix what the language server found at the caret, or rewrite the code there"
                    .to_string(),
            action: CommandAction::CodeActions,
            shortcut: bindings::chord_of(Action::CodeActions),
        });
    }
    if crate::native::renaming::front_tab_renames(app) {
        commands.push(Command {
            title: "rename symbol".to_string(),
            description:
                "Rename the name at the caret, everywhere the language server knows it is used"
                    .to_string(),
            action: CommandAction::RenameSymbol,
            shortcut: bindings::chord_of(Action::RenameSymbol),
        });
    }
    // Only when the repo is on this machine: the picker is the OS's, and it cannot browse a
    // repo that lives on the far side of a `--remote` connection.
    if app.backend().reads_this_machine() {
        commands.push(Command {
            title: "open file".to_string(),
            description: "Open a file of the repo in a tab, to read and edit".to_string(),
            action: CommandAction::OpenFile,
            shortcut: None,
        });
    }

    // Another window of each frame, opening on its launch screen. The board, the review and
    // a shell are three windows rather than three panes when that is how you want them; on
    // macOS these are in the Window menu as well.
    for frame in crate::cli::NEW_WINDOW_FRAMES {
        commands.push(Command {
            title: format!("new {} window", frame.command()),
            description: format!(
                "Open another window on {}, asking which repo",
                frame.opens()
            ),
            action: CommandAction::NewWindow(*frame),
            // Only this window's own program has a chord; the other two are named only.
            shortcut: (*frame == app.frame())
                .then(|| bindings::chord_of(Action::NewWindow))
                .flatten(),
        });
    }

    // Starting again is how a window picks up a rebuilt executable: the one it is running is
    // the one it started with. On macOS this is the Window menu's Restart.
    commands.push(Command {
        title: "restart window".to_string(),
        description: format!(
            "Start {} again on this repo, and close this window",
            app.frame().command()
        ),
        action: CommandAction::RestartWindow,
        shortcut: None,
    });

    // The window's own actions. On macOS these are in the menu bar too; here is where every
    // platform can reach them.
    // Only the two platforms that have a launcher to write are offered it.
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        commands.push(Command {
            title: "install desktop launchers".to_string(),
            // Where they land is said by the toast the install leaves, rather than here: it
            // depends on what this account can write.
            description: "Give each of the three windows an entry the OS offers".to_string(),
            action: CommandAction::InstallLaunchers,
            shortcut: None,
        });
    }
    commands.push(Command {
        title: format!("switch to {}", app.model.theme.toggled().label()),
        description: "Change between the light and dark palette".to_string(),
        action: CommandAction::ToggleTheme,
        shortcut: bindings::chord_of(Action::ToggleTheme),
    });

    // One entry per color rather than a cycling command: with several windows open, the
    // point is to give this one a color that is not the color of the others, which means
    // picking it rather than stepping through until it comes round.
    for color in crate::native::workspace_color::ALL
        .into_iter()
        .filter(|color| *color != app.model.workspace_color)
    {
        commands.push(Command {
            title: format!("workspace color: {}", color.label()),
            description: "Paint this window's background, so it is told from the others"
                .to_string(),
            action: CommandAction::MarkWorkspace(color),
            shortcut: None,
        });
    }

    // Changed submodules are further reviews the user can open beside this one.
    for submodule in app
        .model
        .submodules
        .iter()
        .filter(|submodule| submodule.changed_files > 0)
    {
        commands.push(Command {
            title: submodule.name.clone(),
            description: format!("Review the changed submodule at {}", submodule.repo_path),
            action: CommandAction::OpenPane(OpenPaneRequest::ReviewRepo {
                repo_path: submodule.repo_path.clone(),
                title: submodule.name.clone(),
            }),
            shortcut: None,
        });
    }

    let available: Vec<AgentKind> = app
        .model
        .review_ref(&root)
        .and_then(|review| review.payload.as_ref())
        .map(|payload| {
            payload
                .available_agents
                .iter()
                .filter(|agent| agent.available)
                .map(|agent| agent.kind)
                .collect()
        })
        .unwrap_or_default();

    for (kind, title, description) in AGENT_COMMANDS {
        if available.contains(kind) {
            commands.push(Command {
                title: (*title).to_string(),
                description: (*description).to_string(),
                action: CommandAction::OpenPane(OpenPaneRequest::Terminal {
                    command: Some(*kind),
                }),
                shortcut: None,
            });
        }
    }

    commands
}

/// The name of the repo the window was launched on - the same name the review header shows.
/// A window whose review has not loaded yet has no repo to name, and says "repo" until it has.
fn root_repo_name(app: &App) -> String {
    repo_name_of(app, &app.model.root_session_id)
}

/// The same, for any one review the window has open: the repo it is a review of. A review
/// whose first answer has not arrived yet says "repo" until it has.
fn repo_name_of(app: &App, session_id: &str) -> String {
    app.model
        .review_ref(session_id)
        .and_then(|review| review.payload.as_ref())
        .map(|payload| payload.repo_name.clone())
        .unwrap_or_else(|| "repo".to_string())
}

/// The work log, under its name and under `wl`: the second is the same command, there for
/// the hands that type `wl` into the palette without reading it - two letters and Enter.
/// `wl` is a row of its own rather than a word in the first row's description because the
/// list is what the palette types over, and a term matched in a description would still
/// leave the row behind whatever came before it.
fn work_log_commands() -> [Command; 2] {
    [
        Command {
            title: "work log".to_string(),
            description: "Open work log".to_string(),
            action: CommandAction::OpenWorkLog,
            shortcut: None,
        },
        Command {
            title: "wl".to_string(),
            description: "Open work log".to_string(),
            action: CommandAction::OpenWorkLog,
            shortcut: None,
        },
    ]
}

/// A pane the workspace keeps one of. It stays on the list once it is open - searching for
/// "review" and finding nothing reads as the review being gone - and running it then brings
/// the open one forward, which `Workspace::open_pane` already does for every one of these.
fn single_pane_command(
    already_open: bool,
    title: &str,
    opens: &str,
    raises: &str,
    action: CommandAction,
    shortcut: Option<&'static [bindings::Press]>,
) -> Command {
    Command {
        title: title.to_string(),
        description: if already_open { raises } else { opens }.to_string(),
        action,
        shortcut,
    }
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

/// One row per code action the language server offered, in the order it put them.
fn code_action_rows(app: &App) -> Vec<Command> {
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
fn place_rows(app: &App) -> Vec<Command> {
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
fn rename_rows(app: &App) -> Vec<Command> {
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
fn file_rows(app: &App) -> Vec<Command> {
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
fn content_rows(app: &App) -> Vec<Command> {
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
fn footnote(app: &App, shown: usize) -> Option<String> {
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

fn hint_of(app: &App) -> String {
    match app.model.palette.mode {
        PaletteMode::Commands => "Execute a command…".to_string(),
        PaletteMode::Files if app.model.palette.files_link_to_task.is_some() => {
            "Link a file to the task by name…".to_string()
        }
        PaletteMode::Files => format!("Open a file{} by name…", searched_repo_note(app)),
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
fn empty_message(app: &App) -> String {
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
fn refresh_file_matches(app: &mut App) {
    refresh_search(
        app,
        |backend, session_id, asked, listener| {
            backend.find_files(session_id, &asked.query, asked.scope, listener)
        },
        |model| &mut model.palette.files,
    );
}

/// The same for the lines the content search found.
fn refresh_content_matches(app: &mut App) {
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

pub(crate) fn draw(app: &mut App, ctx: &egui::Context) {
    if !app.model.palette.open {
        return;
    }
    // A press anywhere else - a shell, a tab, a pane in the next frame over - puts the palette
    // away and belongs to whatever was pressed. It is answered before anything is drawn: the
    // search box asks for the keyboard every frame it exists, so a palette still on screen
    // would take it straight back off the shell that was just clicked.
    if pressed_outside(ctx, app.model.palette.rect) {
        app.model.palette.dismiss();
        return;
    }
    match app.model.palette.mode {
        PaletteMode::Commands
        | PaletteMode::Rename
        | PaletteMode::Places
        | PaletteMode::CodeActions => {}
        PaletteMode::Files => refresh_file_matches(app),
        PaletteMode::Contents => refresh_content_matches(app),
    }
    let palette = app.palette_of();
    let matches = rows_for(app);

    let (dismiss, move_down, move_up, accept) = ctx.input_mut(|input| {
        (
            input.key_pressed(Key::Escape),
            input.key_pressed(Key::ArrowDown),
            input.key_pressed(Key::ArrowUp),
            input.key_pressed(Key::Enter),
        )
    });

    if dismiss {
        app.model.palette.dismiss();
        return;
    }
    // Typed since the highlight was picked: the list underneath it is a different list, and
    // the first match of the new one is what Enter runs.
    let retyped = app.model.palette.highlight_query != app.model.palette.query;
    if retyped {
        app.model.palette.highlighted = 0;
        app.model.palette.highlight_query = app.model.palette.query.clone();
    }
    if !matches.is_empty() {
        let last = matches.len() - 1;
        if move_down {
            app.model.palette.highlighted = (app.model.palette.highlighted + 1).min(last);
        }
        if move_up {
            app.model.palette.highlighted = app.model.palette.highlighted.saturating_sub(1);
        }
        app.model.palette.highlighted = app.model.palette.highlighted.min(last);
    }

    let mut chosen: Option<usize> = None;
    if accept && !matches.is_empty() {
        chosen = Some(app.model.palette.highlighted);
    }

    let screen = ctx.viewport_rect();
    let area = egui::Area::new("moonreview-palette".into())
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, screen.height() * 0.12))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.line))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(9))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 10],
                    blur: 28,
                    spread: 0,
                    color: Color32::from_black_alpha(60),
                })
                .show(ui, |ui| {
                    ui.set_width((screen.width() * 0.5).clamp(360.0, 560.0));

                    let hint = hint_of(app);
                    let entry = ui.add(
                        egui::TextEdit::singleline(&mut app.model.palette.query)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(7, 5)),
                    );
                    entry.request_focus();
                    // Opened on a name to type over, which is selected so the first key typed
                    // replaces it - see `PaletteState::show_rename`.
                    if std::mem::take(&mut app.model.palette.select_query) {
                        select_all(ui.ctx(), entry.id, &app.model.palette.query);
                    }
                    if matches!(
                        app.model.palette.mode,
                        PaletteMode::Files | PaletteMode::Contents
                    ) {
                        draw_scope_box(ui, &mut app.model.palette.search_scope, &palette);
                    }

                    ui.add_space(6.0);
                    if matches.is_empty() {
                        ui.label(RichText::new(empty_message(app)).color(palette.muted));
                        return;
                    }

                    // The rows scroll rather than the box growing: a list taller than the
                    // window would be pushed up to fit, search line and all, every time the
                    // matches came in. The height is the rows' own, capped - an area hands
                    // its contents last frame's rect to fit in, so a scroll area left to size
                    // itself would stay as short as the empty palette was. The highlight is
                    // kept in view when the keyboard moves it; a pointer over a row is
                    // already looking at it.
                    let keep_highlight_in_view = move_down || move_up || retyped;
                    let rows_height = rows_height_of(matches.len(), ui.spacing().item_spacing.y)
                        .min(screen.height() * ROWS_HEIGHT_OF_SCREEN);
                    egui::ScrollArea::vertical()
                        .max_height(rows_height)
                        .min_scrolled_height(rows_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (index, command) in matches.iter().enumerate() {
                                let highlighted = index == app.model.palette.highlighted;
                                let row = draw_row(ui, command, highlighted, &palette);
                                if highlighted && keep_highlight_in_view {
                                    row.scroll_to_me(None);
                                }
                                if row.clicked() {
                                    chosen = Some(index);
                                }
                                if row.hovered() {
                                    app.model.palette.highlighted = index;
                                }
                            }
                        });
                    if let Some(footnote) = footnote(app, matches.len()) {
                        ui.label(
                            RichText::new(footnote)
                                .size(SMALL_SIZE - 1.0)
                                .color(palette.muted),
                        );
                    }
                });
        });
    app.model.palette.rect = Some(area.response.rect);

    if let Some(index) = chosen
        && let Some(command) = matches.into_iter().nth(index)
    {
        app.model.palette.dismiss();
        app.pending_action = Some(command.action);
    }
}

/// How tall `rows` rows are when laid out one under the other: the rows themselves, and the
/// layout's gap between each pair. Counting the rows alone left the area a gap short per row,
/// which put a scrollbar on a list of three rows that all fit.
fn rows_height_of(rows: usize, gap: f32) -> f32 {
    rows as f32 * ROW_HEIGHT + rows.saturating_sub(1) as f32 * gap
}

/// The box under the two searches' line that widens them to the files the repo's
/// `.gitignore` leaves out - the submodules of a repo that ignores them, say. The line keeps
/// the keyboard; this is for the pointer.
fn draw_scope_box(ui: &mut egui::Ui, scope: &mut SearchScope, palette: &Palette) {
    ui.add_space(3.0);
    let mut included = scope.includes_ignored();
    let label = RichText::new("include gitignored")
        .size(SMALL_SIZE - 1.0)
        .color(palette.muted);
    if ui.checkbox(&mut included, label).changed() {
        *scope = SearchScope::including_ignored(included);
    }
}

/// Select the whole of the palette's line.
fn select_all(ctx: &egui::Context, id: egui::Id, query: &str) {
    let Some(mut state) = egui::TextEdit::load_state(ctx, id) else {
        return;
    };
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(query.chars().count()),
        )));
    state.store(ctx, id);
}

/// Whether a pointer button went down this frame away from where the palette drew last frame.
/// Before it has drawn once there is nowhere to be outside of, and the press is somebody
/// else's business.
fn pressed_outside(ctx: &egui::Context, drawn_at: Option<egui::Rect>) -> bool {
    let Some(drawn_at) = drawn_at else {
        return false;
    };
    ctx.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|at| !drawn_at.contains(at))
    })
}

fn draw_row(
    ui: &mut egui::Ui,
    command: &Command,
    highlighted: bool,
    palette: &Palette,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), egui::Sense::click());
    let response = crate::native::widgets::clickable(response);

    if ui.is_rect_visible(rect) {
        if highlighted {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(5), palette.control_active_bg);
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(5),
                Stroke::new(1.0, palette.accent),
                StrokeKind::Inside,
            );
        }
        ui.painter().text(
            rect.min + vec2(9.0, 5.0),
            Align2::LEFT_TOP,
            &command.title,
            egui::FontId::proportional(crate::native::theme::UI_SIZE),
            palette.ink,
        );
        ui.painter().text(
            rect.min + vec2(9.0, 19.0),
            Align2::LEFT_TOP,
            &command.description,
            egui::FontId::proportional(SMALL_SIZE - 1.0),
            palette.muted,
        );
        // The keyboard's own way to the same command, against the right edge.
        if let Some(chord) = command.shortcut {
            ui.painter().text(
                egui::pos2(rect.max.x - 9.0, rect.center().y),
                Align2::RIGHT_CENTER,
                bindings::describe(chord),
                egui::FontId::proportional(SMALL_SIZE - 1.0),
                palette.muted,
            );
        }
    }
    response
}

#[cfg(test)]
mod tests {
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
