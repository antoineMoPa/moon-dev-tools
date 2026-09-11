//! The blame of a file tab: who last touched each stretch of it, beside the lines.
//!
//! One test for the whole of it, since it is one feature as far as the person asking is
//! concerned: the column comes up on the toggle, it is of the file as it is on disk rather
//! than as it was committed - so the line added since reads as not committed - and it draws.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_frames::PaneId;
use egui_kittest::Harness;

use crate::{
    api::{BlameChunk, Blamed},
    native::{panes::Pane, theme::ThemeMode},
};

use super::{Fixture, app_for, click_at, settle};

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
    let chunks = Arc::new(Mutex::new(None::<Vec<BlameChunk>>));
    let chunks_in_ui = Arc::clone(&chunks);
    // What the review of the repo is of, and whether its tab is the one in front: what a
    // click on a stretch changes.
    let review_on = Arc::new(Mutex::new(None::<(bool, Option<String>)>));
    let review_on_in_ui = Arc::clone(&review_on);

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
            app.draw(ui);

            *review_on_in_ui.lock().expect("poisoned") = Some((
                matches!(app.active_pane(), Some((_, Pane::Review { .. }))),
                app.model
                    .review_ref(&app.model.root_session_id)
                    .and_then(|review| review.payload.as_ref())
                    .and_then(|payload| payload.active_commit.clone()),
            ));
            let open_pane: Option<PaneId> = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            let Some(pane_id) = open_pane else {
                return;
            };
            let Some(editor) = app.model.file_editors.get(&pane_id) else {
                return;
            };
            // The toggle, once the text is there - the way `[blame]` is only offered then.
            if editor.is_loaded() && !toggled_in_ui.swap(true, Ordering::Relaxed) {
                crate::native::blame::toggle(&mut app, pane_id);
                return;
            }
            *chunks_in_ui.lock().expect("poisoned") = app
                .model
                .file_editors
                .get(&pane_id)
                .and_then(|editor| editor.blaming().chunks_for_test())
                .map(<[BlameChunk]>::to_vec);
        });

    assert!(
        settle(&mut harness, || chunks.lock().expect("poisoned").is_some()),
        "the blame never came back"
    );
    let chunks = chunks
        .lock()
        .expect("poisoned")
        .clone()
        .expect("settled on it");
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

    // The column beside the lines: the sha, the day and the author over the summary for
    // the committed stretches, and the uncommitted line said to be so.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("file-pane-blame");

    // Pointing at a stretch tells the whole of it. The stretches sit where the snapshot above
    // put them: the first one's row is under the header, left of the line numbers.
    let first_stretch = egui::pos2(120.0, 87.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(first_stretch));
    harness.run_steps(3);
    harness.snapshot("file-pane-blame-pointed");

    // Clicking one opens the review on its commit and brings the review forward.
    assert_eq!(
        review_on.lock().expect("poisoned").clone(),
        Some((false, None)),
        "the file tab is in front, on a review of the local changes"
    );
    click_at(&mut harness, first_stretch);
    let sha = commit.sha.clone();
    assert!(
        settle(&mut harness, || {
            *review_on.lock().expect("poisoned") == Some((true, Some(sha.clone())))
        }),
        "the review never came forward on the commit: {:?}",
        review_on.lock().expect("poisoned")
    );
}
