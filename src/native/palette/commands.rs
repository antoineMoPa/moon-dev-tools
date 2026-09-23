//! The palette's list of commands: everything the window can do from where it is, named the
//! way a person would search for it.

use egui_frames::DropSide;

use crate::{
    api::AgentKind,
    native::{
        app::App,
        bindings::{self, Action},
        panes::{OpenPaneRequest, PaneKind},
    },
    project::ProjectCommand,
};

use super::{Command, CommandAction};

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
pub(super) fn repo_name_of(app: &App, session_id: &str) -> String {
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
pub(super) fn work_log_commands() -> [Command; 2] {
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
