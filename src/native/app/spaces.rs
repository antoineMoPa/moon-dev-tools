//! Spaces: the tabs and splits of several projects kept in one window, and a switch between
//! them - what a desktop's workspaces are, with a project in each. A window has
//! [`SPACE_COUNT`] of them from the start.
//!
//! A space is a whole [`App`]: its panes, its reviews, its board, its shells, and the
//! project they are on. The one in front is `self`; the others are parked, boxed, in
//! [`Spaces`], and nothing of theirs is drawn or polled until they come forward again. A space
//! nobody has been to is not an app at all - [`Slot::Untouched`] - and costs nothing: it is
//! built, on the project of the space it is reached from, the first time it is gone to. Going
//! to one swaps it with `self`, the way [`App::switch_project`] swaps `self` with a fresh
//! window - which is why what belongs to the window rather than to a space is handed across
//! in [`App::take_window_from`], the same list for both.
//!
//! A space holds its own set of panes, and pane ids are only unique within one set, so
//! nothing of one space can be told apart from another's while they share a map. Keeping each
//! space a whole app is what stops that from mattering.

use std::sync::Arc;

use egui_frames::Layout;

use crate::{
    api::OpenSessionRequest,
    native::{Launch, panes::Pane, workspace_color::WorkspaceColor},
};

use super::App;

/// How many spaces a window has.
pub(crate) const SPACE_COUNT: usize = 4;

/// One place in the list of spaces.
enum Slot {
    /// The space in front: the [`App`] this is a field of.
    Front,
    Parked(Box<App>),
    /// Never gone to, so nothing of it exists yet.
    Untouched,
}

/// Every space of the window, in the order they are listed.
pub(crate) struct Spaces {
    slots: Vec<Slot>,
    front: usize,
}

impl Default for Spaces {
    fn default() -> Self {
        let mut slots: Vec<Slot> = (0..SPACE_COUNT).map(|_| Slot::Untouched).collect();
        slots[0] = Slot::Front;
        Self { slots, front: 0 }
    }
}

impl Spaces {
    pub(crate) fn count(&self) -> usize {
        self.slots.len()
    }

    /// Where in the list the space in front is.
    pub(crate) fn front(&self) -> usize {
        self.front
    }

    /// The nearest space that has been used, searching leftwards from the one in front and
    /// wrapping round: where a desktop lands when the one it is on goes.
    fn nearest_parked(&self) -> Option<usize> {
        let count = self.slots.len();
        (1..count)
            .map(|step| (self.front + count - step) % count)
            .find(|index| matches!(self.slots[*index], Slot::Parked(_)))
    }
}

/// What a space looks like from outside it: enough to draw its square in the toolbar and to
/// name it.
pub(crate) struct SpaceView<'a> {
    pub(crate) color: WorkspaceColor,
    pub(crate) project_path: Option<&'a str>,
    /// `None` for a space nobody has been to, which has no arrangement yet.
    pub(crate) arrangement: Option<&'a Layout<Pane>>,
    /// Whether a shell in the space is asking for a person.
    pub(crate) wants_attention: bool,
}

impl<'a> SpaceView<'a> {
    fn of(app: &'a App) -> Self {
        Self {
            color: app.model.workspace_color,
            project_path: app.model.project_path.as_deref(),
            arrangement: Some(&app.model.layout),
            wants_attention: !app.model.shells_wanting_attention.is_empty(),
        }
    }

    /// The directory name of the project, which is how a space is called.
    pub(crate) fn name(&self) -> String {
        match self.project_path {
            Some(path) => std::path::Path::new(path)
                .file_name()
                .map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned()),
            None => "empty".to_string(),
        }
    }

    pub(crate) fn is_untouched(&self) -> bool {
        self.arrangement.is_none()
    }
}

impl App {
    /// Every space in list order, the one in front included. A space nobody has been to is
    /// shown as the project of the one in front, which is the project it will open on.
    pub(crate) fn space_views(&self) -> Vec<SpaceView<'_>> {
        self.spaces
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Front => SpaceView::of(self),
                Slot::Parked(parked) => SpaceView::of(parked),
                Slot::Untouched => SpaceView {
                    arrangement: None,
                    wants_attention: false,
                    ..SpaceView::of(self)
                },
            })
            .collect()
    }

    /// The space this one opens as, when nobody has been to it: on the project of the space it
    /// is reached from, which is what a second window on the same work is.
    fn build_untouched_space(&self, ctx: &egui::Context) -> App {
        let open = self
            .model
            .project_path
            .clone()
            .map(|repo_path| OpenSessionRequest {
                repo_path,
                diff_target: None,
                active_commit: None,
            });
        App::new(
            ctx.clone(),
            Launch {
                backend: Arc::clone(self.backend()),
                open,
                frame: self.frame,
            },
        )
    }

    /// Bring the space at this place in the list to the front, building it first if nobody has
    /// been to it.
    pub(crate) fn go_to_space(&mut self, ctx: &egui::Context, index: usize) {
        if index == self.spaces.front || index >= self.spaces.slots.len() {
            return;
        }
        if matches!(self.spaces.slots[index], Slot::Untouched) {
            let built = self.build_untouched_space(ctx);
            self.spaces.slots[index] = Slot::Parked(Box::new(built));
        }
        // Off the screen before anything else: what is laid over the window - a webview, the
        // window of another program - is not something the panes drawn next would cover.
        self.take_overlays_off_the_screen();

        let Slot::Parked(mut arriving) =
            std::mem::replace(&mut self.spaces.slots[index], Slot::Front)
        else {
            unreachable!("the space is parked by now");
        };
        let mut slots = std::mem::take(&mut self.spaces.slots);
        let leaving = self.spaces.front;

        std::mem::swap(self, &mut *arriving);
        // `arriving` is now the space that was in front.
        self.take_window_from(&mut arriving);
        slots[leaving] = Slot::Parked(arriving);
        self.spaces = Spaces {
            slots,
            front: index,
        };
        // Its own color, which the window is painted in.
        self.needs_style = true;
    }

    /// Take the space at `from` out of the list and put it back at `to`. The space in front
    /// stays the one in front, wherever it ends up.
    pub(crate) fn move_space(&mut self, from: usize, to: usize) {
        let count = self.spaces.count();
        if from >= count || to >= count || from == to {
            return;
        }
        let moved = self.spaces.slots.remove(from);
        self.spaces.slots.insert(to, moved);
        let front = self.spaces.front;
        self.spaces.front = if front == from {
            to
        } else if from < front && front <= to {
            front - 1
        } else if to <= front && front < from {
            front + 1
        } else {
            front
        };
    }

    /// Go to the next space, wrapping round at the end.
    pub(crate) fn go_to_next_space(&mut self, ctx: &egui::Context) {
        let count = self.spaces.count();
        self.go_to_space(ctx, (self.spaces.front + 1) % count);
    }

    /// Go to the previous space, wrapping round at the start.
    pub(crate) fn go_to_previous_space(&mut self, ctx: &egui::Context) {
        let count = self.spaces.count();
        self.go_to_space(ctx, (self.spaces.front + count - 1) % count);
    }

    /// Empty a space: its tabs, its reviews and its board go, and its shells are left running
    /// on the server, as they are by [`App::switch_project`]. It stays in the list, untouched.
    ///
    /// A file with edits that are not on disk refuses it, since closing would throw them away
    /// without a tab left to close twice. The space in front cannot be emptied while it is the
    /// only one in use: that is closing the window.
    pub(crate) fn close_space(&mut self, ctx: &egui::Context, index: usize) {
        if index >= self.spaces.count() {
            return;
        }
        let closing: &App = match &self.spaces.slots[index] {
            Slot::Untouched => return,
            Slot::Parked(parked) => parked,
            Slot::Front => self,
        };
        let unsaved: Vec<&str> = closing
            .model
            .file_editors
            .values()
            .filter(|editor| editor.is_dirty())
            .map(|editor| editor.file_path.as_str())
            .collect();
        if !unsaved.is_empty() {
            let message = format!(
                "unsaved edits in {} - save or close them to close the space",
                unsaved.join(", ")
            );
            self.model.error(message);
            return;
        }

        if index == self.spaces.front {
            let Some(landing) = self.spaces.nearest_parked() else {
                return;
            };
            self.go_to_space(ctx, landing);
        }
        // Whatever was closed is parked now, in the slot it was in.
        self.spaces.slots[index] = Slot::Untouched;
    }

    /// What happens when the workspace in front has nothing left in it - its last tab was
    /// closed, or ⌘W was pressed on its launch screen: another space in use comes forward if
    /// there is one, and the window closes if there is not.
    pub(crate) fn close_space_or_window(&mut self, ctx: &egui::Context) {
        if self.spaces.nearest_parked().is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let front = self.spaces.front;
        self.close_space(ctx, front);
    }

    /// Give a parked space what it needs to stay as it was: a shell's output is read, and so
    /// cannot pile up in a channel nobody is drawing from.
    pub(crate) fn keep_parked_spaces_alive(&mut self) {
        for slot in &mut self.spaces.slots {
            if let Slot::Parked(parked) = slot {
                for terminal in parked.terminals.values_mut() {
                    terminal.poll();
                }
            }
        }
    }

    /// Hide what this space has laid over the window. A webview or the window of another
    /// program is not clipped by the panes around it, so a space put down with one showing
    /// would go on showing it over the next.
    fn take_overlays_off_the_screen(&mut self) {
        #[cfg(target_os = "macos")]
        self.webviews.hide_all();
        #[cfg(target_os = "linux")]
        self.applications.take_windows_off_the_screen(&self.model.layout);
    }

    /// The clients of the application panes of the spaces that are not in front. They are not
    /// on screen, and they are not windows nothing shows either.
    #[cfg(target_os = "linux")]
    pub(crate) fn application_clients_in_parked_spaces(&self) -> std::collections::HashSet<u32> {
        self.spaces
            .slots
            .iter()
            .filter_map(|slot| match slot {
                Slot::Parked(parked) => Some(parked),
                _ => None,
            })
            .flat_map(|parked| parked.model.layout.panes())
            .filter_map(|(_, pane)| match pane {
                Pane::Application { client, .. } => Some(*client),
                _ => None,
            })
            .collect()
    }
}
