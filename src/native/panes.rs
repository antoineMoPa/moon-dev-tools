//! What a pane of the workspace is, and how the window draws one.
//!
//! The arrangement itself - frames, tabs, splits, drags - belongs to `egui_frames`. This is
//! moonreview's side of it: the four kinds of pane the window has, and the
//! [`egui_frames::PaneView`] that names their tabs and draws their bodies.

use egui::{Color32, Response, Sense, Stroke, Ui, vec2};
use egui_frames::{FrameId, PaneId, PaneView, Tab};
use serde::{Deserialize, Serialize};

use crate::{
    api::AgentKind,
    native::{
        app::App,
        bindings::{self, Action},
        commit_pane, review,
        theme::{self, Palette, ThemeMode},
        widgets,
    },
};

/// Which kind a pane is, for the questions that only care about the kind: where a new one
/// belongs, and whether ⌘F has anything to search.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PaneKind {
    Review,
    Agents,
    Terminal,
    File,
    Tasks,
    /// One task: what it has running, and what it can start - or one being written before it
    /// exists, which is the same pane with nothing to run in it yet.
    Start,
    Commit,
    Submodules,
    Project,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum Pane {
    /// One review of one repo. The window opens on the repo it was launched in; changed
    /// submodules are further reviews, each with its own session and its own tab title.
    Review {
        session_id: String,
        title: String,
    },
    Agents,
    Terminal {
        terminal_id: String,
        #[serde(default)]
        command: Option<AgentKind>,
        /// The moontasks task this shell belongs to, if it is one of a task's. A task's shell
        /// outlives its tab - closing the tab lets go of it rather than ending it.
        #[serde(default)]
        task_id: Option<String>,
    },
    /// One file of the repo being reviewed, open for reading and editing.
    File {
        session_id: String,
        file_path: String,
        /// The moontasks task the file was opened from, if it was opened from a card: its
        /// notes, or a file linked to it.
        #[serde(default)]
        task_id: Option<String>,
    },
    /// The moontasks board of the repo being reviewed.
    Tasks,
    /// One task, in a tab of its own: what a click on its card opens. The title it was opened
    /// under is only the name for the tab until the board answers - the task's own is what the
    /// pane reads and writes.
    Start {
        task_id: String,
        title: String,
    },
    /// A task being written before it exists: what a column's `+` opens. It becomes the task's
    /// own pane, in this very tab, the moment the title box is answered for - see
    /// [`crate::native::start_pane`].
    NewTask {
        /// The column the card will join, and which end of it: the `+` that was pressed.
        column: crate::moontasks::ColumnId,
        joins: crate::moontasks::ColumnEnd,
        /// What the half-written title and notes are kept under in
        /// [`crate::native::model::BoardState::drafts`], since there is no task to key them by.
        draft_id: String,
    },
    /// Committing what one review has staged, and pushing it.
    Commit {
        session_id: String,
    },
    /// Every submodule of the repo, and a way into a review of the changed ones.
    Submodules,
    /// The two commands the Project menu runs, and where they are set.
    Project,
}

impl Pane {
    pub(crate) fn kind(&self) -> PaneKind {
        match self {
            Self::Review { .. } => PaneKind::Review,
            Self::Agents => PaneKind::Agents,
            Self::Terminal { .. } => PaneKind::Terminal,
            Self::File { .. } => PaneKind::File,
            Self::Tasks => PaneKind::Tasks,
            Self::Start { .. } | Self::NewTask { .. } => PaneKind::Start,
            Self::Commit { .. } => PaneKind::Commit,
            Self::Submodules => PaneKind::Submodules,
            Self::Project => PaneKind::Project,
        }
    }

    pub(crate) fn tab_title(&self) -> String {
        match self {
            Self::Review { title, .. } => title.clone(),
            Self::Agents => "comment agents".to_string(),
            Self::Terminal { command, .. } => match command {
                Some(AgentKind::Claude) => "claude".to_string(),
                Some(AgentKind::Codex) => "codex".to_string(),
                Some(AgentKind::OpenCode) => "opencode".to_string(),
                _ => "terminal".to_string(),
            },
            // The name alone: the path is on the pane's own header and on the tab's hover.
            Self::File { file_path, .. } => file_path
                .rsplit('/')
                .next()
                .unwrap_or(file_path)
                .to_string(),
            Self::Tasks => "moontasks".to_string(),
            // The task's own name: the tab is that task's, and what it offers is on the pane.
            Self::Start { title, .. } => title.clone(),
            Self::NewTask { .. } => "new task".to_string(),
            Self::Commit { .. } => "commit".to_string(),
            Self::Submodules => "submodules".to_string(),
            Self::Project => "project".to_string(),
        }
    }

    /// The moontasks task this pane is working in, if it is one of a task's: its shell, a file
    /// opened from its card, or the pane of what it can start.
    pub(crate) fn task_id(&self) -> Option<&str> {
        match self {
            Self::Terminal { task_id, .. } | Self::File { task_id, .. } => task_id.as_deref(),
            Self::Start { task_id, .. } => Some(task_id),
            _ => None,
        }
    }

    /// Whether this pane is a review of one particular session.
    pub(crate) fn reviews(&self, session_id: &str) -> bool {
        matches!(self, Self::Review { session_id: open, .. } if open == session_id)
    }

    /// Whether this pane is the commit pane of one particular review.
    pub(crate) fn commits(&self, session_id: &str) -> bool {
        matches!(self, Self::Commit { session_id: open } if open == session_id)
    }
}

/// A pane the user asked for, before it has a name.
#[derive(Clone)]
pub(crate) enum OpenPaneRequest {
    Review {
        session_id: String,
        title: String,
    },
    /// A review of a repo that has no session yet, which is how a changed submodule is opened:
    /// the session gets created on the way to the pane.
    ReviewRepo {
        repo_path: String,
        title: String,
    },
    Agents,
    Terminal {
        command: Option<AgentKind>,
    },
    /// A shell the server already has, opened in a tab of its own. This is how a task's agent
    /// is brought back on screen after its tab was closed.
    AttachTerminal {
        terminal_id: String,
        command: Option<AgentKind>,
        task_id: Option<String>,
    },
    File {
        session_id: String,
        file_path: String,
        /// Where to open the file, for one opened from a content search rather than by name.
        at: Option<OpenAt>,
    },
    Tasks,
    /// One task, in a tab of its own.
    TaskStart {
        task_id: String,
        title: String,
    },
    /// A pane to write a new task on, for the `+` of this column and end.
    NewTask {
        column: crate::moontasks::ColumnId,
        joins: crate::moontasks::ColumnEnd,
        draft_id: String,
    },
    /// Committing what one review has staged.
    Commit {
        session_id: String,
    },
    Submodules,
    /// The two commands the Project menu runs, and where they are set.
    Project,
}

/// The match a file is opened at: the line to bring on screen, and the text that was
/// searched for, which the pane marks the way the find bar does.
#[derive(Clone)]
pub(crate) struct OpenAt {
    /// Counted from one, as the number in the fringe is.
    pub(crate) line: usize,
    pub(crate) query: String,
}

impl App {
    /// What a shell's tab reads: its name, for a shell that has one - see
    /// [`Model::terminal_names`](crate::native::model::Model::terminal_names); failing that
    /// the title the program in it set, the way a terminal's does; failing that what it was
    /// opened as.
    pub(crate) fn shell_tab_title(&self, pane: &Pane) -> String {
        let Pane::Terminal { terminal_id, .. } = pane else {
            panic!("only a shell's tab is titled by its shell");
        };
        if let Some(Some(name)) = self.model.terminal_names.get(terminal_id) {
            return name.clone();
        }
        self.terminals
            .get(terminal_id)
            .and_then(egui_tty::Terminal::title)
            .unwrap_or_else(|| pane.tab_title())
    }
}

impl PaneView<Pane> for App {
    fn tab(&mut self, pane_id: PaneId, pane: &Pane) -> Tab {
        // A shell's name is asked for the first time its tab is drawn without one, so a tab
        // that is never brought to the front - and so never attached - still reads it.
        if let Pane::Terminal { terminal_id, .. } = pane
            && !self.model.terminal_names.contains_key(terminal_id)
        {
            self.read_terminal_name(terminal_id);
        }
        let title = match pane {
            Pane::Terminal { .. } => self.shell_tab_title(pane),
            // The task's title as the board has it now, so a task renamed after its start
            // window opened is renamed on the tab too.
            Pane::Start { task_id, .. } => self
                .model
                .board
                .tasks
                .iter()
                .find(|task| task.id == *task_id)
                .map(|task| task.title.clone())
                .unwrap_or_else(|| pane.tab_title()),
            _ => pane.tab_title(),
        };
        // A file with edits that are not on disk carries a dot before its name.
        let unsaved = matches!(pane, Pane::File { .. }) && self.file_pane_is_dirty(pane_id);
        let hover = match pane {
            Pane::File { file_path, .. } => file_path.clone(),
            Pane::Start { title, .. } => format!("Start something in {title}"),
            Pane::NewTask { .. } => "Name this task to make its card".to_string(),
            // The title the program set, which the tab of a named shell does not show - a
            // plain shell's directory, an agent's own status line - and how the tab is renamed.
            Pane::Terminal { terminal_id, .. } => {
                let program_title = self
                    .terminals
                    .get(terminal_id)
                    .and_then(egui_tty::Terminal::title)
                    .unwrap_or_else(|| title.clone());
                format!("{program_title}\n\nDouble click to rename this tab")
            }
            _ => title.clone(),
        };
        let editing = self
            .model
            .renaming_tab
            .as_ref()
            .is_some_and(|rename| rename.pane_id == pane_id);

        let mut tab = Tab::new(title)
            .with_marker(unsaved)
            .with_hover(hover)
            .editing(editing);
        // The chord that raises this tab, for tabs cmd+1..cmd+9 can reach - worked out in
        // `stamp_tab_shortcuts` before the strips are drawn.
        if let Some(shortcut) = self.tab_shortcuts.get(&pane_id) {
            tab = tab.with_indicator(shortcut.clone());
        }
        tab
    }

    /// The tab title being retyped. Enter and clicking away keep it, Escape throws it away -
    /// the same shape a column's heading has.
    fn tab_editor_ui(&mut self, ui: &mut Ui, pane_id: PaneId, pane: &Pane) {
        let Some(rename) = self
            .model
            .renaming_tab
            .as_mut()
            .filter(|rename| rename.pane_id == pane_id)
        else {
            panic!("a tab is drawn editing only while it is the one being renamed");
        };
        let entry = ui.add_sized(
            ui.available_size(),
            egui::TextEdit::singleline(&mut rename.name)
                .font(egui::FontId::proportional(theme::SMALL_SIZE + 1.0))
                .margin(egui::Margin::symmetric(3, 1))
                .hint_text("Tab name"),
        );
        if std::mem::take(&mut rename.focus) {
            entry.request_focus();
        }

        let name = rename.name.clone();
        let abandon = ui.input(|input| input.key_pressed(egui::Key::Escape));
        let keep = entry.lost_focus() && !abandon;
        if !(keep || abandon) {
            return;
        }
        self.model.renaming_tab = None;

        let Pane::Terminal { terminal_id, .. } = pane else {
            panic!("only a shell's tab is renamed");
        };
        if keep && !name.trim().is_empty() && name != self.shell_tab_title(pane) {
            self.rename_terminal(terminal_id.clone(), name);
        }
    }

    fn pane_ui(&mut self, ui: &mut Ui, pane_id: PaneId, pane: &Pane) {
        match pane {
            Pane::Review { session_id, .. } => {
                let session_id = session_id.clone();
                self.apply_review_find(pane_id, &session_id);
                review::draw(self, ui, &session_id);
            }
            Pane::Agents => review::draw_agents(self, ui),
            Pane::Terminal { terminal_id, .. } => {
                let terminal_id = terminal_id.clone();
                self.draw_terminal(ui, pane_id, &terminal_id);
            }
            Pane::File {
                session_id,
                file_path,
                ..
            } => {
                let (session_id, file_path) = (session_id.clone(), file_path.clone());
                self.draw_file_pane(ui, pane_id, &session_id, &file_path);
            }
            Pane::Tasks => crate::native::board::draw(self, ui),
            Pane::Start { task_id, .. } => {
                let task_id = task_id.clone();
                crate::native::start_pane::draw(self, ui, &task_id);
            }
            Pane::NewTask {
                column,
                joins,
                draft_id,
            } => {
                let (column, joins, draft_id) = (column.clone(), *joins, draft_id.clone());
                crate::native::start_pane::draw_new_task(self, ui, &column, joins, &draft_id);
            }
            Pane::Commit { session_id } => {
                let session_id = session_id.clone();
                commit_pane::draw(self, ui, pane_id, &session_id);
            }
            Pane::Submodules => crate::native::submodules::draw(self, ui),
            Pane::Project => crate::native::project_pane::draw(self, ui),
        }
    }

    fn empty_frame_ui(&mut self, ui: &mut Ui, _frame: FrameId) {
        let palette = self.palette_of();
        ui.painter().text(
            ui.max_rect().center(),
            egui::Align2::CENTER_CENTER,
            // Read out of the binding table, so the hint cannot name a chord the keyboard
            // has stopped answering to.
            &format!(
                "{} to execute a command",
                bindings::describe(
                    bindings::chord_of(Action::OpenPalette).expect("the palette is bound")
                )
            ),
            egui::FontId::proportional(theme::UI_SIZE),
            palette.muted,
        );
    }

    /// The light/dark switch, on the tab strip that doubles as the app header.
    fn tab_strip_end(&mut self, ui: &mut Ui, _frame: FrameId, primary: bool) {
        if !primary {
            return;
        }
        // The bundled fonts have no sun or moon glyph, so the switch is drawn rather than
        // typeset - see the glyph test in `ui_tests`.
        let palette = self.palette_of();
        let next = self.model.theme.toggled();
        if theme_switch(ui, self.model.theme, &palette)
            .on_hover_text(format!("switch to {} (⌘J)", next.label()))
            .clicked()
        {
            self.set_theme(next);
        }
    }
}

/// The light/dark switch: a moon in light mode, a sun in dark mode.
fn theme_switch(ui: &mut Ui, theme: ThemeMode, palette: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(17.0, 15.0), Sense::click());
    let response = widgets::clickable(response);
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let ink = if response.hovered() {
        palette.accent
    } else {
        palette.muted
    };
    let center = rect.center();
    match theme {
        ThemeMode::Light => draw_moon(ui.painter(), center, 5.5, ink, palette.header_bg),
        ThemeMode::Dark => {
            ui.painter().circle_filled(center, 3.5, ink);
            for step in 0..8 {
                let angle = std::f32::consts::TAU * step as f32 / 8.0;
                let direction = vec2(angle.cos(), angle.sin());
                ui.painter().line_segment(
                    [center + direction * 5.0, center + direction * 7.0],
                    Stroke::new(1.0, ink),
                );
            }
        }
    }
    response
}

/// The moon, drawn: a filled disc with a second disc punched out of it in the panel color.
/// This is the app's mark, and it must not depend on an emoji font having a glyph for it.
fn draw_moon(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    ink: Color32,
    behind: Color32,
) {
    painter.circle_filled(center, radius, ink);
    painter.circle_filled(
        center + vec2(radius * 0.55, -radius * 0.3),
        radius * 0.85,
        behind,
    );
}
