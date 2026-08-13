//! What a pane of the workspace is, and how the window draws one.
//!
//! The arrangement itself — frames, tabs, splits, drags — belongs to `egui_frames`. This is
//! moonreview's side of it: the four kinds of pane the window has, and the
//! [`egui_frames::PaneView`] that names their tabs and draws their bodies.

use egui::{Color32, Response, Sense, Stroke, Ui, vec2};
use egui_frames::{FrameId, PaneId, PaneView, Tab};
use serde::{Deserialize, Serialize};

use crate::{
    api::AgentKind,
    native::{
        app::App,
        review,
        theme::{self, Palette, ThemeMode},
        widgets,
    },
};

/// Which of the four a pane is, for the questions that only care about the kind: where a new
/// one belongs, and whether ⌘F has anything to search.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PaneKind {
    Review,
    Agents,
    Terminal,
    File,
    Tasks,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum Pane {
    /// One review of one repo. The window opens on the repo it was launched in; changed
    /// submodules are further reviews, each with its own session and its own tab title.
    Review { session_id: String, title: String },
    Agents,
    Terminal {
        terminal_id: String,
        #[serde(default)]
        command: Option<AgentKind>,
        /// The moontasks task this shell belongs to, if it is one of a task's. A task's shell
        /// outlives its tab — closing the tab lets go of it rather than ending it.
        #[serde(default)]
        task_id: Option<String>,
    },
    /// One file of the repo being reviewed, open for reading and editing.
    File {
        session_id: String,
        file_path: String,
    },
    /// The moontasks board of the repo being reviewed.
    Tasks,
}

impl Pane {
    pub(crate) fn kind(&self) -> PaneKind {
        match self {
            Self::Review { .. } => PaneKind::Review,
            Self::Agents => PaneKind::Agents,
            Self::Terminal { .. } => PaneKind::Terminal,
            Self::File { .. } => PaneKind::File,
            Self::Tasks => PaneKind::Tasks,
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
        }
    }

    /// Whether this pane is a review of one particular session.
    pub(crate) fn reviews(&self, session_id: &str) -> bool {
        matches!(self, Self::Review { session_id: open, .. } if open == session_id)
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
        /// What the working tree is read against. `None` is an ordinary review of what has not
        /// been committed; a base is how a task's branch is reviewed whole, because once that
        /// branch is checked out there is nothing uncommitted left to show.
        base: Option<String>,
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
    },
    Tasks,
}

impl PaneView<Pane> for App {
    fn tab(&mut self, pane_id: PaneId, pane: &Pane) -> Tab {
        // A shell's tab takes the title the running program set, the way a terminal does.
        let title = match pane {
            Pane::Terminal { terminal_id, .. } => self
                .terminals
                .get(terminal_id)
                .and_then(egui_tty::Terminal::title)
                .unwrap_or_else(|| pane.tab_title()),
            _ => pane.tab_title(),
        };
        // A file with edits that are not on disk carries a dot before its name.
        let unsaved = matches!(pane, Pane::File { .. }) && self.file_pane_is_dirty(pane_id);
        let hover = match pane {
            Pane::File { file_path, .. } => file_path.clone(),
            _ => title.clone(),
        };

        let mut tab = Tab::new(title).with_marker(unsaved).with_hover(hover);
        // The chord that raises this tab, for tabs cmd+1..cmd+9 can reach — worked out in
        // `stamp_tab_shortcuts` before the strips are drawn.
        if let Some(shortcut) = self.tab_shortcuts.get(&pane_id) {
            tab = tab.with_indicator(shortcut.clone());
        }
        tab
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
            } => {
                let (session_id, file_path) = (session_id.clone(), file_path.clone());
                self.draw_file_pane(ui, pane_id, &session_id, &file_path);
            }
            Pane::Tasks => crate::native::board::draw(self, ui),
        }
    }

    fn empty_frame_ui(&mut self, ui: &mut Ui, _frame: FrameId) {
        let palette = self.palette_of();
        ui.painter().text(
            ui.max_rect().center(),
            egui::Align2::CENTER_CENTER,
            "shift ⌘P to execute a command",
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
        // typeset — see the glyph test in `ui_tests`.
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
