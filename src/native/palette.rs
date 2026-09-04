//! The command palette: everything the workspace can open, searchable.
//!
//! Everything ⌘⇧P offers, in one list.

use egui::{Align2, Color32, CornerRadius, Key, RichText, Stroke, StrokeKind, vec2};
use egui_frames::DropSide;

use crate::{
    api::AgentKind,
    project::ProjectCommand,
    native::{
        app::App,
        bindings::{self, Action},
        panes::{OpenPaneRequest, PaneKind},
        theme::{Palette, SMALL_SIZE},
    },
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
}

/// What one of the palette's two searches has found. One search at a time, for whatever was
/// typed when it started - `searched` says which query the matches belong to, and a query
/// that has moved on since starts another search.
pub(crate) struct Search<T> {
    pub(crate) searched: Option<String>,
    pub(crate) matches: Vec<T>,
    /// Set when the repo had more matches than the search hands back, so the palette can say
    /// that narrowing the query would show different rows rather than only fewer.
    pub(crate) truncated: bool,
    pub(crate) error: Option<String>,
}

impl<T> Default for Search<T> {
    fn default() -> Self {
        Self {
            searched: None,
            matches: Vec::new(),
            truncated: false,
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
    LinkTaskFile { task_id: String, file_path: String },
    /// Turn the palette into the content search, where what is typed is looked for in the
    /// text of every file of the repo.
    SearchContent,
    /// Split the frame the keyboard is in against this side, with a shell in the new half.
    Split(DropSide),
    /// Run one of the project's own commands in a shell of its own.
    RunProject(ProjectCommand),
}

/// The agents that get a "open X in a terminal" command, when they are installed.
const AGENT_COMMANDS: &[(AgentKind, &str, &str)] = &[
    (AgentKind::OpenCode, "opencode", "Open OpenCode in a terminal"),
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
        app.model.layout.find_pane(|pane| pane.reviews(&root)).is_some(),
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
                app.frame().program()
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
        description: "Find text in the files of the repo, wherever under it they are"
            .to_string(),
        action: CommandAction::SearchContent,
        shortcut: bindings::chord_of(Action::SearchContent),
    });
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

    // Another window of each program that is installed beside this one, opening on its
    // launch screen. The board, the review and a shell are three windows rather than three
    // panes when that is how you want them; on macOS these are in the Window menu as well.
    for frame in crate::cli::NEW_WINDOW_FRAMES {
        if crate::native::programs::executable_for(*frame).is_none() {
            continue;
        }
        commands.push(Command {
            title: format!("new {} window", frame.program()),
            description: format!("Open another window on {}, asking which repo", frame.opens()),
            action: CommandAction::NewWindow(*frame),
            // Only this window's own program has a chord; the other two are named only.
            shortcut: (*frame == app.frame()).then(|| bindings::chord_of(Action::NewWindow)).flatten(),
        });
    }

    // Starting again is how a window picks up a rebuilt executable: the one it is running is
    // the one it started with. On macOS this is the Window menu's Restart.
    commands.push(Command {
        title: "restart window".to_string(),
        description: format!(
            "Start {} again on this repo, and close this window",
            app.frame().program()
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
            description: "Give each installed executable an entry the OS offers".to_string(),
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
    }
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
                    session_id: app.model.root_session_id.clone(),
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
        .clone()
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
                session_id: app.model.root_session_id.clone(),
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

/// Whether the list on screen is only the start of what the repo matched.
fn truncated_of(app: &App) -> bool {
    match app.model.palette.mode {
        PaletteMode::Commands => false,
        PaletteMode::Files => app.model.palette.files.truncated,
        PaletteMode::Contents => app.model.palette.contents.truncated,
    }
}

fn file_name_of(file_path: &str) -> &str {
    file_path.rsplit('/').next().unwrap_or(file_path)
}

fn hint_of(app: &App) -> &'static str {
    match app.model.palette.mode {
        PaletteMode::Commands => "Execute a command…",
        PaletteMode::Files if app.model.palette.files_link_to_task.is_some() => {
            "Link a file to the task by name…"
        }
        PaletteMode::Files => "Open a file by name…",
        PaletteMode::Contents => "Find text in the files…",
    }
}

/// What the palette says when it has no rows to show.
fn empty_message(app: &App) -> String {
    let query = app.model.palette.query.as_str();
    match app.model.palette.mode {
        PaletteMode::Commands => "nothing matches".to_string(),
        PaletteMode::Files => {
            let files = &app.model.palette.files;
            searching_message(files.error.as_deref(), files.searched.as_deref(), query)
                .unwrap_or_else(|| "no file of the repo has that name".to_string())
        }
        PaletteMode::Contents => {
            let contents = &app.model.palette.contents;
            if query.is_empty() {
                return "type what to look for in the files".to_string();
            }
            searching_message(contents.error.as_deref(), contents.searched.as_deref(), query)
                .unwrap_or_else(|| "no file of the repo holds that text".to_string())
        }
    }
}

/// What a search has to say for itself before it has an answer to the query on screen, if
/// anything: a search that has not answered for this query yet is still running - what was
/// found for the query before it is gone, and saying "no matches" would be a lie.
fn searching_message(error: Option<&str>, searched: Option<&str>, query: &str) -> Option<String> {
    match error {
        Some(error) => Some(error.to_string()),
        None if searched != Some(query) => Some("searching…".to_string()),
        None => None,
    }
}

/// Keep the file list on the query that is typed.
fn refresh_file_matches(app: &mut App) {
    refresh_search(
        app,
        PaletteMode::Files,
        "palette-files",
        |backend, session_id, query| {
            let payload = backend.find_files(session_id, query)?;
            Ok((payload.files, payload.truncated))
        },
        |model| &mut model.palette.files,
    );
}

/// The same for the lines the content search found.
fn refresh_content_matches(app: &mut App) {
    refresh_search(
        app,
        PaletteMode::Contents,
        "palette-content",
        |backend, session_id, query| {
            let payload = backend.search_contents(session_id, query)?;
            Ok((payload.matches, payload.truncated))
        },
        |model| &mut model.palette.contents,
    );
}

/// Keep one of the searches on the query that is typed.
///
/// The repo can be on another machine, so this is a backend call on a worker thread like
/// reading a file is. One search runs at a time; anything typed while it is out is searched
/// for on the frame after it lands, which is what keeps a held key from starting a search a
/// frame.
fn refresh_search<T: Send + 'static>(
    app: &mut App,
    mode: PaletteMode,
    key: &str,
    find: fn(&dyn crate::backend::Backend, &str, &str) -> anyhow::Result<(Vec<T>, bool)>,
    search_of: fn(&mut crate::native::model::Model) -> &mut Search<T>,
) {
    let query = app.model.palette.query.clone();
    let search = search_of(&mut app.model);
    if search.searched.as_deref() == Some(query.as_str()) {
        return;
    }
    if app.model.root_session_id.is_empty() {
        let search = search_of(&mut app.model);
        search.searched = Some(query);
        search.error = Some("no repo is open in this window yet".to_string());
        return;
    }

    let session_id = app.model.root_session_id.clone();
    let for_call = query.clone();
    app.tasks.spawn_keyed(
        Some(key.to_string()),
        move |backend| find(backend, &session_id, &for_call),
        move |model, result| {
            // The palette may have been put away, or turned to another of its lists, while
            // the search was out. Its answer belongs to neither.
            if !model.palette.open || model.palette.mode != mode {
                return;
            }
            let search = search_of(model);
            search.searched = Some(query);
            match result {
                Ok((matches, truncated)) => {
                    search.matches = matches;
                    search.truncated = truncated;
                    search.error = None;
                }
                Err(error) => {
                    search.matches.clear();
                    search.truncated = false;
                    search.error = Some(format!("{error}"));
                }
            }
        },
    );
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
        PaletteMode::Commands => {}
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
    if app.model.palette.highlight_query != app.model.palette.query {
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

                    ui.add_space(6.0);
                    if matches.is_empty() {
                        ui.label(RichText::new(empty_message(app)).color(palette.muted));
                        return;
                    }

                    for (index, command) in matches.iter().enumerate() {
                        let highlighted = index == app.model.palette.highlighted;
                        let row = draw_row(ui, command, highlighted, &palette);
                        if row.clicked() {
                            chosen = Some(index);
                        }
                        if row.hovered() {
                            app.model.palette.highlighted = index;
                        }
                    }
                    // A cut-short list is not the whole answer, and the rows alone cannot say
                    // so: the files left out could be the one being looked for.
                    if truncated_of(app) {
                        ui.label(
                            RichText::new(format!(
                                "the first {} matches - narrow the search for the rest",
                                matches.len()
                            ))
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
    let (rect, response) = ui.allocate_exact_size(vec2(width, 34.0), egui::Sense::click());
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
