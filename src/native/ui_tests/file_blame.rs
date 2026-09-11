//! The blame of a file tab: who last touched each stretch of it, beside the lines, and the
//! step back through the file's history a click on a stretch takes.
//!
//! One test for the whole of it, since it is one feature as far as the person asking is
//! concerned: the column comes up on the toggle, it is of the file as it is on disk rather
//! than as it was committed - so the line added since reads as not committed - it draws, a
//! click away from the hash opens the version before with its own blame up, and a click on
//! the hash opens the review on the commit.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_frames::PaneId;
use egui_kittest::Harness;

use crate::{
    api::{BlameChunk, Blamed},
    native::{panes::Pane, theme::ThemeMode},
};

use super::{Fixture, app_for, click_at, settle};

/// Whether a tooltip is on screen: egui draws one on a layer of its own.
fn tooltip_is_up(harness: &Harness<'_>) -> bool {
    harness
        .ctx
        .memory(|memory| memory.areas().visible_layer_ids())
        .iter()
        .any(|layer| layer.order == egui::Order::Tooltip)
}

/// What the test reads off the window after each frame.
#[derive(Default, Clone)]
struct Seen {
    /// The tab on the file as it is, and its blame once it is there.
    file_tab: Option<PaneId>,
    chunks: Option<Vec<BlameChunk>>,
    /// The tab on an old version of the file, if one has opened: which version, whether it
    /// is the tab in front, and its blame once it is there.
    version_tab: Option<(String, bool, Option<Vec<BlameChunk>>)>,
    /// Whether the review is the tab in front, and what it is a review of.
    review_in_front: bool,
    review_of: Option<String>,
}

#[test]
fn a_file_tab_shows_who_last_touched_each_stretch_of_it() {
    let fixture = Fixture::new("file-blame");
    fixture.write("src/lib.rs", "pub fn one() {}\npub fn two() {}\n");
    fixture.commit("Add the library");
    // A line put in since the commit, on disk but in no commit.
    fixture.write(
        "src/lib.rs",
        "pub fn one() {}\npub fn typed() {}\npub fn two() {}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let toggled = Arc::new(AtomicBool::new(false));
    let toggled_in_ui = Arc::clone(&toggled);
    // Set to bring the tab on the file as it is back in front.
    let refocus = Arc::new(AtomicBool::new(false));
    let refocus_in_ui = Arc::clone(&refocus);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            let file_tab = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { revision: None, .. }))
                .map(|(pane_id, _)| pane_id);
            if refocus_in_ui.swap(false, Ordering::Relaxed)
                && let Some(pane_id) = file_tab
            {
                app.model.layout.focus_pane(pane_id);
            }
            app.draw(ui);

            let mut now = Seen {
                file_tab,
                review_in_front: matches!(app.active_pane(), Some((_, Pane::Review { .. }))),
                review_of: app
                    .model
                    .review_ref(&app.model.root_session_id)
                    .and_then(|review| review.payload.as_ref())
                    .and_then(|payload| payload.active_commit.clone()),
                ..Seen::default()
            };
            let chunks_of = |app: &crate::native::app::App, pane_id: PaneId| {
                app.model
                    .file_editors
                    .get(&pane_id)
                    .and_then(|editor| editor.blaming().chunks_for_test())
                    .map(<[BlameChunk]>::to_vec)
            };
            if let Some((
                pane_id,
                Pane::File {
                    revision: Some(revision),
                    ..
                },
            )) = app.model.layout.find_pane(|pane| {
                matches!(
                    pane,
                    Pane::File {
                        revision: Some(_),
                        ..
                    }
                )
            }) {
                let in_front = app
                    .active_pane()
                    .is_some_and(|(active, _)| active == pane_id);
                now.version_tab = Some((revision.clone(), in_front, chunks_of(&app, pane_id)));
            }
            if let Some(pane_id) = file_tab
                && let Some(editor) = app.model.file_editors.get(&pane_id)
            {
                // The toggle, once the text is there - the way `[blame]` is only offered then.
                if editor.is_loaded() && !toggled_in_ui.swap(true, Ordering::Relaxed) {
                    crate::native::blame::toggle(&mut app, pane_id);
                } else {
                    now.chunks = chunks_of(&app, pane_id);
                }
            }
            *seen_in_ui.lock().expect("poisoned") = now;
        });
    let seen_now = || seen.lock().expect("poisoned").clone();

    assert!(
        settle(&mut harness, || seen_now().chunks.is_some()),
        "the blame never came back"
    );
    let chunks = seen_now().chunks.expect("settled on it");
    assert_eq!(chunks.len(), 3, "{chunks:?}");
    assert_eq!(chunks[0].lines, 0..1);
    assert_eq!(chunks[1].lines, 1..2);
    assert_eq!(chunks[2].lines, 2..3);
    let Blamed::Committed(commit) = &chunks[0].blamed else {
        panic!("the first line was committed: {chunks:?}");
    };
    assert_eq!(commit.author, "Test User");
    assert_eq!(commit.summary, "Add the library");
    assert_eq!(commit.authored_on, "2024-01-02");
    assert_eq!(chunks[1].blamed, Blamed::NotYetCommitted);
    assert_eq!(chunks[2].blamed, chunks[0].blamed);
    // Before the typed line, the file as the commit has it; before the commit, nothing.
    assert_eq!(
        chunks[1].before.as_ref().map(|before| before.sha.as_str()),
        Some(commit.sha.as_str())
    );
    assert_eq!(chunks[0].before, None);

    // The column beside the lines: the sha, the day and the author over the summary for
    // the committed stretches, and the uncommitted line said to be so.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("file-pane-blame");

    // Pointing at a stretch lights its hash up at once, and tells the whole of it once the
    // pointer has rested there - the way every tooltip waits, which is real time. The
    // stretches sit where the snapshot above put them: the first one's row is under the
    // header, left of the line numbers, its hash the first seven characters of it, and the
    // rows are fourteen points apart.
    let first_hash = egui::pos2(40.0, 87.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(first_hash));
    harness.run_steps(2);
    assert!(
        !tooltip_is_up(&harness),
        "a tooltip the moment the pointer arrives is the distraction this waits out"
    );
    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand,
        "the hash reads as a link under the pointer"
    );
    harness.snapshot("file-pane-blame-hash-pointed");
    let rested = Instant::now() + Duration::from_millis(900);
    settle(&mut harness, || Instant::now() >= rested);
    assert!(tooltip_is_up(&harness), "the tooltip never came up");
    harness.snapshot("file-pane-blame-pointed");

    // A click on the uncommitted stretch, away from where a hash would be, opens the file as
    // the last commit has it: a second tab, read-only, with the blame of that version up -
    // one stretch, since the whole of that version is the one commit's.
    click_at(&mut harness, egui::pos2(170.0, 101.0));
    assert!(
        settle(&mut harness, || {
            matches!(seen_now().version_tab, Some((_, true, Some(_))))
        }),
        "the version before never opened with its blame: {:?}",
        seen_now().version_tab
    );
    let (revision, _, version_chunks) = seen_now().version_tab.expect("settled on it");
    assert_eq!(revision, commit.sha);
    let version_chunks = version_chunks.expect("settled on it");
    assert_eq!(version_chunks.len(), 1, "{version_chunks:?}");
    assert_eq!(version_chunks[0].lines, 0..2);
    assert_eq!(version_chunks[0].blamed, chunks[0].blamed);
    // The pointer off the column, so the snapshot is of the tab and not of a tooltip that may
    // or may not have had its time to come up.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(600.0, 400.0)));
    harness.run_steps(3);
    harness.snapshot("file-pane-at-revision");

    // Back on the file as it is, a click on the hash opens the review on its commit and
    // brings the review forward.
    refocus.store(true, Ordering::Relaxed);
    harness.run_steps(3);
    assert!(
        !seen_now().review_in_front && seen_now().review_of.is_none(),
        "the file tab is in front, on a review of the local changes"
    );
    click_at(&mut harness, first_hash);
    let sha = commit.sha.clone();
    assert!(
        settle(&mut harness, || {
            let seen = seen_now();
            seen.review_in_front && seen.review_of.as_deref() == Some(sha.as_str())
        }),
        "the review never came forward on the commit: {:?}",
        (seen_now().review_in_front, seen_now().review_of)
    );
}
