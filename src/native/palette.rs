//! The command palette: everything the workspace can open, searchable.
//!
//! The command set mirrors `web/src/components/workspace/commands.ts` so ⌘⇧P offers the
//! same things in both frontends.

use egui::{Align2, Color32, CornerRadius, Key, RichText, Stroke, StrokeKind, vec2};

use crate::{
    api::AgentKind,
    native::{
        app::App,
        bindings::{self, Action},
        panes::{OpenPaneRequest, PaneKind},
        theme::{Palette, SMALL_SIZE},
    },
};

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
    OpenInBrowser,
    ToggleTheme,
    InstallLaunchers,
    /// Another window of one of the three programs, on its launch screen.
    NewWindow(crate::cli::Frame),
    /// Ask the OS which file of the repo to open for editing, and open it in a tab.
    OpenFile,
    /// Open the script the board picks work up by, which is what autopilot does.
    EditAutopilot,
    /// Open what the board wrote down about its own decisions.
    ViewAutopilotLog,
}

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

pub(crate) fn commands_for(app: &App) -> Vec<Command> {
    let mut commands = Vec::new();
    let root = app.model.root_session_id.clone();

    commands.push(single_pane_command(
        app.model
            .layout
            .find_pane(|pane| pane.reviews(&root))
            .is_some(),
        "review",
        "Open the main review",
        "Bring the main review forward",
        CommandAction::OpenPane(OpenPaneRequest::Review {
            session_id: root.clone(),
            title: "review".to_string(),
        }),
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
    ));
    commands.push(Command {
        title: "edit autopilot".to_string(),
        description: "Open the script the board picks work up by, to read and edit".to_string(),
        action: CommandAction::EditAutopilot,
        shortcut: None,
    });
    commands.push(Command {
        title: "autopilot log".to_string(),
        description: "Read what the board decided, and why it did nothing".to_string(),
        action: CommandAction::ViewAutopilotLog,
        shortcut: None,
    });
    commands.push(Command {
        title: "terminal".to_string(),
        description: "Open a new shell".to_string(),
        action: CommandAction::OpenPane(OpenPaneRequest::Terminal { command: None }),
        shortcut: bindings::chord_of(Action::NewShellTab),
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

    // The window's own actions. On macOS these are in the menu bar too; here is where every
    // platform can reach them.
    if app.serves_web.is_serving() {
        commands.push(Command {
            title: "open in browser".to_string(),
            description: "Open this review in a browser".to_string(),
            action: CommandAction::OpenInBrowser,
            shortcut: None,
        });
    }
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

    // Changed submodules are further reviews the user can open beside this one.
    for submodule in &app.model.submodules {
        commands.push(Command {
            title: submodule.name.clone(),
            description: format!("Review the changed submodule at {}", submodule.repo_path),
            action: CommandAction::OpenPane(OpenPaneRequest::ReviewRepo {
                base: None,
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

/// A pane the workspace keeps one of. It stays on the list once it is open — searching for
/// "review" and finding nothing reads as the review being gone — and running it then brings
/// the open one forward, which `Workspace::open_pane` already does for every one of these.
fn single_pane_command(
    already_open: bool,
    title: &str,
    opens: &str,
    raises: &str,
    action: CommandAction,
) -> Command {
    Command {
        title: title.to_string(),
        description: if already_open { raises } else { opens }.to_string(),
        action,
        shortcut: None,
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

pub(crate) fn draw(app: &mut App, ctx: &egui::Context) {
    if !app.model.palette.open {
        return;
    }
    // A press anywhere else — a shell, a tab, a pane in the next frame over — puts the palette
    // away and belongs to whatever was pressed. It is answered before anything is drawn: the
    // search box asks for the keyboard every frame it exists, so a palette still on screen
    // would take it straight back off the shell that was just clicked.
    if pressed_outside(ctx, app.model.palette.rect) {
        app.model.palette.dismiss();
        return;
    }
    let palette = app.palette_of();
    let matches = filter(commands_for(app), &app.model.palette.query);

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

                    let entry = ui.add(
                        egui::TextEdit::singleline(&mut app.model.palette.query)
                            .hint_text("Execute a command…")
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(7, 5)),
                    );
                    entry.request_focus();

                    ui.add_space(6.0);
                    if matches.is_empty() {
                        ui.label(RichText::new("nothing matches").color(palette.muted));
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
            command("review", "Open the main review"),
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
