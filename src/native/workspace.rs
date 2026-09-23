//! The workspace: which panes are open, where they go, and the shells behind them.
//!
//! `egui_frames` draws the arrangement and answers the pointer; `egui_tty` is what a shell pane
//! holds. What is left here is everything only moonreview can decide: where a new pane belongs,
//! what closing one costs, and which shell a tab is showing.

mod opening;
mod shells;
mod tabs;
mod terminals;

use std::time::Duration;

use egui::Ui;
use egui_frames::{DropSide, FrameId, FramesEvent, Layout};

use crate::{
    cli::Frame,
    native::{
        app::App,
        panes::{Pane, PaneKind},
    },
};

/// The narrowest a frame may be left at by opening a shell beside it. Below this, the shell
/// joins a frame's tabs instead of taking a column of its own.
const MIN_COLUMN_WIDTH: f32 = 320.0;

/// A breath between the top of the window and the first frame's border. Frames are separated
/// from each other by their dividers, so only the window's own top edge reads as cramped.
const WORKSPACE_TOP_INSET: f32 = 4.0;

/// Where a new shell's pane lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalPlacement {
    /// Beside the shells already open, or in a new column down the right if there are none.
    WithOtherShells,
    /// A new full-height column down the right of the workspace.
    RightColumn,
    /// A frame of its own against one side of a frame, splitting it in two. This is what the
    /// palette's split commands ask for.
    Beside { frame: FrameId, side: DropSide },
    /// Another tab in this frame, for a workspace with no room left to split.
    Tab(FrameId),
}

/// What a shell being started is to the window, beyond a tab to type in. Both marks are
/// written once the shell is there, which is why they travel with the request rather than
/// being set by whoever asked for it.
#[derive(Default, Clone, Copy)]
struct ShellMarks {
    /// The shell the Project menu's commands go into from here on - see
    /// [`Model::project_shell`].
    takes_project_commands: bool,
    /// The shell whose end restarts the window - see [`Model::restart_on_shell_exit`].
    restarts_window: bool,
}

/// The arrangement a run starts from: the shape the last one left behind, with this run's
/// review in the frame whose tab strip is the app header.
///
/// A stored arrangement is worth keeping for its shape - where the user put their columns and
/// rows. Its panes are not: a review pane names a session that no longer exists, and a shell
/// pane names a process that died with the last run. Whatever the review and the adopted shells
/// do not fill is dropped on the first frame drawn.
pub(crate) fn arrangement_for(
    stored: Option<Layout<Pane>>,
    session_id: &str,
    frame: Frame,
) -> Layout<Pane> {
    let mut layout = match stored {
        Some(mut stored) if stored.is_coherent() => {
            stored.take_panes();
            stored
        }
        _ => Layout::new(),
    };

    let primary = layout.primary_frame();
    match frame {
        Frame::Review => layout.add_pane(
            primary,
            Pane::Review {
                session_id: session_id.to_string(),
                title: "review".to_string(),
            },
            None,
        ),
        Frame::Tasks => layout.add_pane(primary, Pane::Tasks, None),
        // A shell has to be started before there is a pane to show it, so this window opens
        // with nothing in it and the shell arrives a moment later.
        Frame::Shell => return layout,
    };
    layout
}

impl App {
    /// One frame of the arrangement: drawn, dragged, and whatever the user asked of it done.
    pub(crate) fn draw_workspace(&mut self, ui: &mut Ui) {
        // An empty frame is a leftover - whatever emptied it should have taken it with it - and
        // the only way to see one is the hint its body draws. Dropping them here means no path
        // can leave one on screen, whatever it forgot. A workspace with nothing open at all
        // keeps its single frame: that is a state rather than a leftover.
        self.model.layout.drop_empty_frames();
        self.follow_front_tab(ui.ctx());
        self.follow_task_in_front();
        self.stamp_tab_shortcuts();
        *self.frames.style_mut() = self.palette_of().frames_style();

        ui.add_space(WORKSPACE_TOP_INSET);

        // The workspace draws moonreview's own panes, so the view it needs is this app: both it
        // and the arrangement are lent out for the call and put back straight after.
        let mut frames = std::mem::take(&mut self.frames);
        let mut layout = std::mem::take(&mut self.model.layout);
        let events = frames.show(ui, &mut layout, self);
        self.model.layout = layout;
        self.frames = frames;

        for event in events {
            match event {
                // Deferred: a pane must not be taken out of the tree that is drawing it.
                FramesEvent::PaneCloseRequested(pane) => self.pending_close = Some(pane),
                FramesEvent::OtherTabsCloseRequested(kept) => {
                    self.pending_close_of_others = Some(kept);
                }
                FramesEvent::NewTabRequested(frame) => self.open_shell_beside(frame),
                FramesEvent::TabDoubleClicked(pane) => self.open_tab_rename(pane),
            }
        }
    }

    /// Where a pane of this kind goes: with the others of its kind, else the frame whose tab
    /// strip is the app header.
    fn frame_for(&self, kind: PaneKind, preferred: FrameId) -> FrameId {
        self.model
            .layout
            .frame_holding(preferred, |pane| pane.kind() == kind)
            .unwrap_or_else(|| self.model.layout.primary_frame())
    }

    /// The review a shell asked for from this frame starts in: the review the pane in front of
    /// the frame belongs to - a review, a file of it, or its commit pane - else wherever the
    /// last shell was started, else the review the window was launched on.
    pub(crate) fn shell_session_for(&self, frame: FrameId) -> String {
        let showing = self
            .model
            .layout
            .frame(frame)
            .and_then(egui_frames::Frame::active_pane)
            .and_then(|pane| self.model.layout.pane(pane));
        if let Some(
            Pane::Review { session_id, .. }
            | Pane::File { session_id, .. }
            | Pane::Commit { session_id },
        ) = showing
        {
            return session_id.clone();
        }
        self.model
            .last_shell_session_id
            .clone()
            .unwrap_or_else(|| self.model.root_session_id.clone())
    }

    /// A new shell goes in its own column down the right of the workspace, unless that would
    /// squeeze that column - or whatever it takes the room from - below a usable width, in
    /// which case it becomes another tab in the frame it was asked for.
    pub(crate) fn room_for_a_column(&self, frame: FrameId) -> TerminalPlacement {
        // A shell takes a column of its own only in a workspace that is still one frame wide.
        // Once the user has split it, whatever they arranged is theirs, and a new shell joins
        // the tabs of the frame it was asked from.
        let split_already = self.model.layout.frame_count() > 1;
        let width = self.frames.frame_rect(frame).map(|rect| rect.width());

        match width {
            Some(width) if !split_already && fits_another_column(width) => {
                TerminalPlacement::RightColumn
            }
            _ => TerminalPlacement::Tab(frame),
        }
    }

    /// Where a shell joining the ones already open goes: their frame, as another tab in it.
    /// Failing that - nothing is open in a shell yet - wherever the workspace has room, which
    /// is a column of its own only while the workspace is one frame wide and wide enough to
    /// give a column up; a window already split into columns takes another tab in the frame
    /// the keyboard is in rather than a column narrower than the last - see
    /// [`Self::room_for_a_column`].
    pub(crate) fn beside_the_other_shells(&self) -> TerminalPlacement {
        let active = self.model.layout.active_frame();
        match self
            .model
            .layout
            .frame_holding(active, |pane| pane.kind() == PaneKind::Terminal)
        {
            Some(frame) => TerminalPlacement::Tab(frame),
            None => self.room_for_a_column(active),
        }
    }

    /// The column down the right to open a pane into, given what that pane is opened to stand
    /// beside - the board for a task's tabs, the review for the pane committing it.
    ///
    /// That is the frame at the right of the workspace, whatever is already in it: a window
    /// that took a column of its own for every tab opened off the one on the left would be a
    /// new column a minute, each of them narrower than the last.
    ///
    /// `None` when the frame at the right is the one holding what the pane stands beside - a
    /// workspace that has not been split yet. A tab landing on top of what it was opened to sit
    /// next to would put that out of sight, so a column is made for it instead.
    fn column_beside(&self, stands_beside: impl Fn(&Pane) -> bool) -> Option<FrameId> {
        let frame = frame_at_the_right(&self.model.layout)?;
        let holds_it = self
            .model
            .layout
            .frame(frame)?
            .panes()
            .iter()
            .filter_map(|pane| self.model.layout.pane(*pane))
            .any(stands_beside);
        (!holds_it).then_some(frame)
    }

    /// The column a task's tabs share: its start window, the shells started from it, the files
    /// opened off its card. They stand beside the board.
    pub(crate) fn task_column(&self) -> Option<FrameId> {
        self.column_beside(|pane| pane.kind() == PaneKind::Tasks)
    }
}

/// The frame down the right of the workspace: the last of a row, and the first of a column, so
/// what is answered is the one at the top right rather than whichever frame happens to be last
/// in the arrangement.
///
/// `None` where there is nothing to the right - a workspace of one frame has no right-hand side
/// yet, only a middle.
fn frame_at_the_right(layout: &Layout<Pane>) -> Option<FrameId> {
    let mut node = layout.root();
    loop {
        match node {
            egui_frames::LayoutNode::Frame { frame } => return Some(*frame),
            egui_frames::LayoutNode::Split {
                direction,
                children,
                ..
            } => {
                node = match direction {
                    egui_frames::SplitDirection::Row => children.last()?,
                    egui_frames::SplitDirection::Column => children.first()?,
                };
            }
        }
    }
}

/// Put a pane in a column of its own down the right of the workspace.
fn add_right_column(layout: &mut Layout<Pane>, pane: Pane) {
    layout.add_pane_against_edge(DropSide::Right, egui_frames::DEFAULT_EDGE_SHARE, pane);
}

/// Put a shell's pane where the placement says, falling back to a column of its own.
fn place_shell(layout: &mut Layout<Pane>, placement: &TerminalPlacement, pane: Pane) {
    let column = add_right_column;

    match placement {
        TerminalPlacement::WithOtherShells => {
            let active = layout.active_frame();
            match layout.frame_holding(active, |pane| pane.kind() == PaneKind::Terminal) {
                Some(frame) => {
                    layout.add_pane(frame, pane, None);
                }
                None => column(layout, pane),
            }
        }
        TerminalPlacement::RightColumn => column(layout, pane),
        TerminalPlacement::Beside { frame, side } if layout.frame(*frame).is_some() => {
            layout.add_pane_beside(*frame, *side, pane);
        }
        TerminalPlacement::Beside { .. } => column(layout, pane),
        // The frame is gone if it was closed while the shell was starting.
        TerminalPlacement::Tab(frame) if layout.frame(*frame).is_some() => {
            layout.add_pane(*frame, pane, None);
        }
        TerminalPlacement::Tab(_) => column(layout, pane),
    }
}

/// Whether a frame this wide can give up the share a new right-hand column takes without
/// leaving either side too narrow to work in.
fn fits_another_column(frame_width: f32) -> bool {
    let new_column = frame_width * egui_frames::DEFAULT_EDGE_SHARE;
    let left_behind = frame_width * (1.0 - egui_frames::DEFAULT_EDGE_SHARE);
    new_column >= MIN_COLUMN_WIDTH && left_behind >= MIN_COLUMN_WIDTH
}

/// How often a window with a shell in it redraws. The terminal widget asks for its own frames
/// while its program is alive; this is the floor under everything else.
pub(crate) const SHELL_REPAINT_INTERVAL: Duration = Duration::from_millis(33);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_frame_has_room_for_a_shell_beside_it() {
        assert!(fits_another_column(1440.0));
        assert!(fits_another_column(1000.0));
    }

    #[test]
    fn a_narrow_frame_takes_the_shell_as_a_tab() {
        // At the window's minimum width, a column would leave both sides too thin to read.
        assert!(!fits_another_column(720.0));
        assert!(!fits_another_column(900.0));
    }

    fn shell(id: &str) -> Pane {
        Pane::Terminal {
            terminal_id: id.to_string(),
            command: None,
            task_id: None,
        }
    }

    #[test]
    fn the_first_shell_takes_a_column_and_the_next_one_joins_it() {
        let mut layout = Layout::with_pane(Pane::Review {
            session_id: "session".to_string(),
            title: "review".to_string(),
        });

        place_shell(&mut layout, &TerminalPlacement::WithOtherShells, shell("a"));
        assert_eq!(layout.frame_count(), 2, "the first shell takes a column");

        place_shell(&mut layout, &TerminalPlacement::WithOtherShells, shell("b"));
        assert_eq!(layout.frame_count(), 2, "the second joins its tabs");
        assert_eq!(layout.pane_count(), 3);
        assert!(layout.is_coherent());
    }

    /// What the palette's split commands ask for: the frame in two, the shell in the new half.
    #[test]
    fn a_split_puts_the_shell_in_a_frame_of_its_own_beside_the_one_asked_for() {
        let mut layout = Layout::with_pane(Pane::Review {
            session_id: "session".to_string(),
            title: "review".to_string(),
        });
        let frame = layout.active_frame();

        place_shell(
            &mut layout,
            &TerminalPlacement::Beside {
                frame,
                side: DropSide::Bottom,
            },
            shell("a"),
        );

        assert_eq!(layout.frame_count(), 2, "the frame was split in two");
        assert_eq!(
            layout.frame(frame).map(|frame| frame.panes().len()),
            Some(1),
            "the review kept its half to itself"
        );
        assert!(layout.is_coherent());
    }

    /// A shell takes a moment to start, and the frame it was asked from may be gone by then.
    #[test]
    fn a_shell_asked_for_from_a_frame_that_has_since_closed_still_opens() {
        let mut layout = Layout::with_pane(Pane::Agents);
        let doomed = layout.add_pane_beside(layout.active_frame(), DropSide::Right, shell("gone"));
        let gone = layout.frame_of(doomed).expect("expected a frame");
        layout.close_pane(doomed);
        assert!(
            layout.frame(gone).is_none(),
            "the frame went with its shell"
        );

        place_shell(&mut layout, &TerminalPlacement::Tab(gone), shell("a"));
        place_shell(
            &mut layout,
            &TerminalPlacement::Beside {
                frame: gone,
                side: DropSide::Right,
            },
            shell("b"),
        );

        assert_eq!(layout.pane_count(), 3, "both shells landed somewhere");
        assert!(layout.is_coherent());
    }
}
