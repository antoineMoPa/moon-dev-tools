//! The window's side of an agent's visualizations: asking the server which ones its terminals
//! have announced, opening a pane for each beside the terminal, and keeping the page each pane
//! shows current - see [`crate::visualizations`] for where they come from.

use std::{collections::HashMap, time::Instant};

use egui_frames::{DropSide, FrameId};

use crate::{
    native::{
        app::App,
        model::Model,
        panes::{Pane, PaneKind},
    },
    visualizations::VisualizationView,
};

/// What the window knows of visualizations.
#[derive(Default)]
pub(crate) struct Visualizations {
    last_poll: Option<Instant>,
    /// Whether the server has answered once. What it had announced before this window asked is
    /// not news - a window reconnecting to a server - and opens nothing.
    heard: bool,
    /// The server's last answer, by fragment path.
    known: HashMap<String, VisualizationView>,
    /// The page each open pane shows, by fragment path.
    pub(crate) pages: HashMap<String, Page>,
    /// The fragments whose page has to be fetched, with the write it has to be at least as
    /// new as.
    wanted: HashMap<String, u64>,
    /// Counts pages as they arrive, so a pane can tell a page it has from a newer one.
    arrived: u64,
}

/// A visualization's page, as it was when fetched.
pub(crate) struct Page {
    pub(crate) html: String,
    /// Which arrival this is - see [`Visualizations::arrived`].
    pub(crate) arrival: u64,
    /// The write of the fragment it was fetched for.
    modified_unix_ms: u64,
}

impl App {
    /// Ask which visualizations the terminals have announced, on the poll clock, and fetch the
    /// pages the panes are owed.
    pub(crate) fn poll_visualizations(&mut self) {
        if self.model.root_session_id.is_empty() {
            return;
        }
        self.fetch_wanted_pages();
        let state = &mut self.model.visualizations;
        if state
            .last_poll
            .is_some_and(|last| last.elapsed() < super::app::POLL_INTERVAL)
        {
            return;
        }
        state.last_poll = Some(Instant::now());
        let session_id = self.model.root_session_id.clone();
        self.tasks.spawn_keyed(
            Some("visualizations".to_string()),
            move |backend| backend.terminal_visualizations(&session_id),
            |model, result| match result {
                Ok(views) => take_visualizations(model, views),
                Err(error) => model.error(format!("could not read visualizations: {error}")),
            },
        );
    }

    fn fetch_wanted_pages(&mut self) {
        let wanted: Vec<(String, u64)> = self
            .model
            .visualizations
            .wanted
            .iter()
            .map(|(path, modified)| (path.clone(), *modified))
            .collect();
        for (fragment_path, modified_unix_ms) in wanted {
            let session_id = self.model.root_session_id.clone();
            let for_apply = fragment_path.clone();
            // One fetch a fragment at a time. A write landing while one is out is still
            // wanted when it comes back, and fetched again.
            self.tasks.spawn_keyed(
                Some(format!("visualization-page:{fragment_path}")),
                move |backend| backend.visualization_page(&session_id, &fragment_path),
                move |model, result| {
                    let state = &mut model.visualizations;
                    match result {
                        Ok(html) => {
                            state.arrived += 1;
                            state.pages.insert(
                                for_apply.clone(),
                                Page {
                                    html,
                                    arrival: state.arrived,
                                    modified_unix_ms,
                                },
                            );
                            if state.wanted.get(&for_apply) == Some(&modified_unix_ms) {
                                state.wanted.remove(&for_apply);
                            }
                        }
                        Err(error) => {
                            state.wanted.remove(&for_apply);
                            model.error(format!("could not show {for_apply}: {error}"));
                        }
                    }
                },
            );
        }
    }
}

/// The server's answer: a pane for each visualization announced since the last one, a new
/// page for each fragment rewritten, and nothing kept for panes that were closed.
fn take_visualizations(model: &mut Model, views: Vec<VisualizationView>) {
    let heard = std::mem::replace(&mut model.visualizations.heard, true);
    let known = std::mem::take(&mut model.visualizations.known);
    for view in views {
        let previous = known.get(&view.fragment_path);
        let announced_again = previous.is_none_or(|previous| view.announced > previous.announced);
        // Shown again, it is loaded again, whether or not the file was rewritten.
        if heard && announced_again {
            // Without taking the keyboard off the terminal, which is where the person is
            // talking to the agent.
            let keyboard = model.layout.active_frame();
            show_visualization(model, &view.fragment_path, Some(&view.terminal_id));
            model.layout.set_active_frame(keyboard);
            model
                .visualizations
                .wanted
                .insert(view.fragment_path.clone(), view.modified_unix_ms);
        }
        model
            .visualizations
            .known
            .insert(view.fragment_path.clone(), view);
    }

    let shown: Vec<String> = model
        .layout
        .panes()
        .filter_map(|(_, pane)| match pane {
            Pane::Visualization { fragment_path } => Some(fragment_path.clone()),
            _ => None,
        })
        .collect();
    let state = &mut model.visualizations;
    state.pages.retain(|path, _| shown.contains(path));
    state.wanted.retain(|path, _| shown.contains(path));
    // A page older than the fragment is now: the agent rewrote the file.
    for path in &shown {
        if let Some(view) = state.known.get(path)
            && state
                .pages
                .get(path)
                .is_none_or(|page| page.modified_unix_ms != view.modified_unix_ms)
        {
            state.wanted.insert(path.clone(), view.modified_unix_ms);
        }
    }
}

fn pane_of(model: &Model, fragment_path: &str) -> Option<egui_frames::PaneId> {
    model
        .layout
        .find_pane(
            |pane| matches!(pane, Pane::Visualization { fragment_path: open } if open == fragment_path),
        )
        .map(|(pane, _)| pane)
}

/// Open a visualization kept on a task, from its row on the card or the task's pane - the
/// agent that showed it may be long gone, so there is no terminal to open it beside.
pub(crate) fn open_kept_visualization(model: &mut Model, fragment_path: &str) {
    show_visualization(model, fragment_path, None);
    // Nothing tells the window when the copy was written, and nothing needs to: a page is
    // fetched for a pane that has none, and a running agent's rewrite still arrives as news.
    if !model.visualizations.pages.contains_key(fragment_path) {
        model
            .visualizations
            .wanted
            .insert(fragment_path.to_string(), 0);
    }
}

/// Put a visualization in front, in the frame beside the terminal of the agent that showed it.
///
/// One pane a fragment: shown again, it is brought forward. A new one joins the frame other
/// visualizations are already in; failing that, the terminal's frame is split for it; failing
/// that - no terminal, or its tab is not open - it gets a column down the right.
fn show_visualization(model: &mut Model, fragment_path: &str, terminal_id: Option<&str>) {
    match pane_of(model, fragment_path) {
        Some(pane) => model.layout.focus_pane(pane),
        None => {
            let pane = Pane::Visualization {
                fragment_path: fragment_path.to_string(),
            };
            let terminal_frame = terminal_id
                .and_then(|terminal_id| {
                    model.layout.find_pane(|pane| {
                        matches!(pane, Pane::Terminal { terminal_id: open, .. } if open == terminal_id)
                    })
                })
                .and_then(|(pane, _)| model.layout.frame_of(pane));
            let with_visualizations = model.layout.frame_ids().into_iter().find(|frame| {
                Some(*frame) != terminal_frame && frame_holds_visualizations(model, *frame)
            });
            match (with_visualizations, terminal_frame) {
                (Some(frame), _) => {
                    model.layout.add_pane(frame, pane, None);
                }
                (None, Some(frame)) => {
                    model.layout.add_pane_beside(frame, DropSide::Right, pane);
                }
                (None, None) => {
                    model.layout.add_pane_against_edge(
                        DropSide::Right,
                        egui_frames::DEFAULT_EDGE_SHARE,
                        pane,
                    );
                }
            }
        }
    }
}

fn frame_holds_visualizations(model: &Model, frame: FrameId) -> bool {
    model.layout.frame(frame).is_some_and(|frame| {
        frame.panes().iter().any(|pane| {
            model
                .layout
                .pane(*pane)
                .is_some_and(|pane| pane.kind() == PaneKind::Visualization)
        })
    })
}

#[cfg(test)]
#[path = "visualizations_tests.rs"]
mod tests;
