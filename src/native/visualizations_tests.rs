//! Where a visualization's pane opens, and what the window fetches for it.

use egui_frames::PaneId;

use super::*;
use crate::{
    api::AgentKind,
    native::{
        theme::ThemeMode,
        ui_tests::{Fixture, app_for},
    },
};

const FRAGMENT: &str = "/codex/visualizations/2026/09/16/thread/monthly-revenue.html";

fn announced(fragment_path: &str, announced: u64, modified_unix_ms: u64) -> VisualizationView {
    VisualizationView {
        terminal_id: "terminal-1".to_string(),
        fragment_path: fragment_path.to_string(),
        announced,
        modified_unix_ms,
    }
}

/// A window with the review in one frame and a Codex terminal in its own, which has the
/// keyboard.
fn app_with_codex_terminal(fixture: &Fixture) -> (App, PaneId) {
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let review_frame = app.model.layout.primary_frame();
    let terminal = app.model.layout.add_pane_beside(
        review_frame,
        DropSide::Right,
        Pane::Terminal {
            terminal_id: "terminal-1".to_string(),
            command: Some(AgentKind::Codex),
            task_id: None,
        },
    );
    (app, terminal)
}

fn visualization_panes(app: &App) -> Vec<PaneId> {
    app.model
        .layout
        .panes()
        .filter(|(_, pane)| pane.kind() == PaneKind::Visualization)
        .map(|(pane, _)| pane)
        .collect()
}

#[test]
fn what_was_announced_before_the_window_asked_opens_nothing() {
    let fixture = Fixture::new("visualizations-heard");
    let (mut app, _) = app_with_codex_terminal(&fixture);

    take_visualizations(&mut app.model, vec![announced(FRAGMENT, 1, 10)]);

    assert!(visualization_panes(&app).is_empty());
    assert!(app.model.visualizations.wanted.is_empty());
}

#[test]
fn a_new_visualization_opens_beside_its_terminal_and_leaves_it_the_keyboard() {
    let fixture = Fixture::new("visualizations-beside");
    let (mut app, terminal) = app_with_codex_terminal(&fixture);
    let terminal_frame = app.model.layout.frame_of(terminal).unwrap();
    take_visualizations(&mut app.model, Vec::new());

    take_visualizations(&mut app.model, vec![announced(FRAGMENT, 1, 10)]);

    let [pane] = visualization_panes(&app)[..] else {
        panic!("one pane for one visualization");
    };
    let frame = app.model.layout.frame_of(pane).unwrap();
    assert_ne!(frame, terminal_frame, "the terminal stays in sight");
    assert_eq!(app.model.layout.active_frame(), terminal_frame);
    assert_eq!(app.model.visualizations.wanted.get(FRAGMENT), Some(&10));
    assert_eq!(
        app.model.layout.pane(pane).unwrap().tab_title(),
        "monthly revenue"
    );

    // A second visualization joins the first rather than splitting the terminal again.
    let second = "/codex/visualizations/2026/09/16/thread/churn.html";
    take_visualizations(
        &mut app.model,
        vec![announced(FRAGMENT, 1, 10), announced(second, 1, 20)],
    );
    let panes = visualization_panes(&app);
    assert_eq!(panes.len(), 2);
    assert!(
        panes
            .iter()
            .all(|pane| app.model.layout.frame_of(*pane) == Some(frame))
    );
}

#[test]
fn announced_again_it_is_brought_forward_and_loaded_again() {
    let fixture = Fixture::new("visualizations-again");
    let (mut app, _) = app_with_codex_terminal(&fixture);
    take_visualizations(&mut app.model, Vec::new());
    take_visualizations(&mut app.model, vec![announced(FRAGMENT, 1, 10)]);
    let [pane] = visualization_panes(&app)[..] else {
        panic!("one pane");
    };
    let frame = app.model.layout.frame_of(pane).unwrap();
    // The page arrives, and another tab is put in front of it.
    app.model.visualizations.wanted.clear();
    app.model.visualizations.pages.insert(
        FRAGMENT.to_string(),
        Page {
            html: "<p>first</p>".to_string(),
            arrival: 1,
            modified_unix_ms: 10,
        },
    );
    app.model.layout.add_pane(frame, Pane::Messages, None);

    take_visualizations(&mut app.model, vec![announced(FRAGMENT, 2, 10)]);

    assert_eq!(visualization_panes(&app), [pane]);
    assert_eq!(
        app.model.layout.frame(frame).unwrap().active_pane(),
        Some(pane)
    );
    assert_eq!(app.model.visualizations.wanted.get(FRAGMENT), Some(&10));
}

#[test]
fn a_rewritten_fragment_is_fetched_again_and_a_closed_pane_keeps_nothing() {
    let fixture = Fixture::new("visualizations-rewritten");
    let (mut app, _) = app_with_codex_terminal(&fixture);
    take_visualizations(&mut app.model, Vec::new());
    take_visualizations(&mut app.model, vec![announced(FRAGMENT, 1, 10)]);
    app.model.visualizations.wanted.clear();
    app.model.visualizations.pages.insert(
        FRAGMENT.to_string(),
        Page {
            html: "<p>first</p>".to_string(),
            arrival: 1,
            modified_unix_ms: 10,
        },
    );

    take_visualizations(&mut app.model, vec![announced(FRAGMENT, 1, 10)]);
    assert!(
        app.model.visualizations.wanted.is_empty(),
        "nothing changed"
    );

    take_visualizations(&mut app.model, vec![announced(FRAGMENT, 1, 30)]);
    assert_eq!(app.model.visualizations.wanted.get(FRAGMENT), Some(&30));

    let [pane] = visualization_panes(&app)[..] else {
        panic!("one pane");
    };
    app.model.layout.close_pane(pane);
    take_visualizations(&mut app.model, vec![announced(FRAGMENT, 1, 40)]);
    assert!(app.model.visualizations.pages.is_empty());
    assert!(app.model.visualizations.wanted.is_empty());
    assert!(
        visualization_panes(&app).is_empty(),
        "a rewrite reopens nothing"
    );
}

/// The window's stored arrangement, as eframe keeps it.
struct StoredLayout(String);

impl eframe::Storage for StoredLayout {
    fn get_string(&self, _key: &str) -> Option<String> {
        Some(self.0.clone())
    }
    fn set_string(&mut self, _key: &str, _value: String) {}
    fn remove_string(&mut self, _key: &str) {}
    fn flush(&mut self) {}
}

/// A layout saved while the pane was a bare test page names a kind of pane there is no longer:
/// the window opens on the default arrangement rather than failing to.
#[test]
fn a_layout_holding_the_old_test_page_pane_is_set_aside() {
    let fixture = Fixture::new("visualizations-stale-layout");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let mut layout = egui_frames::Layout::with_pane(Pane::Messages);
    layout.add_pane(layout.primary_frame(), Pane::Messages, None);
    let stored = serde_json::to_string(&layout).unwrap();
    app.restore_layout_from(Some(&StoredLayout(stored.clone())));
    assert!(app.model.restored_layout.take().is_some());
    let stale = stored.replacen(r#"{"kind":"messages"}"#, r#"{"kind":"webview"}"#, 1);
    assert!(stale.contains("webview"));

    app.restore_layout_from(Some(&StoredLayout(stale)));

    assert!(app.model.restored_layout.is_none());
}

#[test]
fn a_visualization_kept_on_a_task_opens_in_front_and_has_its_page_fetched() {
    let fixture = Fixture::new("visualizations-kept");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let kept = "/repo/.moontasks/task/visualizations/monthly-revenue.html";

    open_kept_visualization(&mut app.model, kept);
    let pane = visualization_panes(&app)[0];
    assert_eq!(
        app.model.layout.frame_of(pane),
        Some(app.model.layout.active_frame())
    );

    // Looked away from - another frame given the keyboard, another tab put in front of it -
    // and opened again from the task: the same tab, in front, with the keyboard.
    let review = app.model.layout.primary_frame();
    app.model.layout.set_active_frame(review);
    let beside = app.model.layout.add_pane(
        app.model.layout.frame_of(pane).unwrap(),
        Pane::Messages,
        None,
    );
    app.model.layout.set_active_frame(review);
    open_kept_visualization(&mut app.model, kept);

    assert_eq!(visualization_panes(&app), vec![pane]);
    let frame = app.model.layout.frame_of(pane).unwrap();
    assert_eq!(app.model.layout.active_frame(), frame);
    assert_eq!(
        app.model.layout.frame(frame).unwrap().active_pane(),
        Some(pane)
    );
    assert_ne!(
        app.model.layout.frame(frame).unwrap().active_pane(),
        Some(beside)
    );
    assert!(app.model.visualizations.wanted.contains_key(kept));
}
