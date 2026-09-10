//! Adding a column to the moontasks board, anywhere along the row.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use super::{app_for, seeded_fixture, settle, type_letter};

/// A column is added beside another one, not only at the end: the heading's menu offers either
/// side of itself, the box to name it stands where the column will go, and that is where the
/// board puts it.
#[test]
fn a_column_is_added_beside_the_one_its_menu_was_opened_on() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = seeded_fixture("column-insert");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

    // The board's columns and where IN PROGRESS's heading is drawn, both read back out of the
    // window: the order is what the insert has to change, the heading is what opens the menu.
    let seen = Arc::new(Mutex::new((Vec::<String>::new(), egui::Rect::NOTHING)));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            let heading = ui
                .ctx()
                .read_response(egui::Id::new(("moontask-column", "in_progress")));
            if let Ok(mut seen) = seen_in_ui.lock() {
                seen.0 = app
                    .model
                    .board
                    .columns
                    .iter()
                    .map(|column| column.label.clone())
                    .collect();
                seen.1 = heading
                    .map(|response| response.rect)
                    .unwrap_or(egui::Rect::NOTHING);
            }
        });

    let read = || seen.lock().expect("poisoned").clone();
    assert!(
        settle(&mut harness, || read().1.is_positive()),
        "the IN PROGRESS heading was never drawn"
    );
    harness.run_steps(3);
    assert_eq!(read().0, ["TODO", "IN PROGRESS", "DONE"]);

    // A right click on the heading, which is what offers a column to either side of it.
    let on_the_heading = read().1.center();
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: on_the_heading,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run_steps(3);

    harness.get_by_label("new column to the left").click();
    harness.run_steps(3);

    for (key, letter) in [
        (egui::Key::B, "B"),
        (egui::Key::L, "l"),
        (egui::Key::O, "o"),
        (egui::Key::C, "c"),
        (egui::Key::K, "k"),
    ] {
        type_letter(&mut harness, key, letter);
    }
    harness.run_steps(3);

    harness.get_by_label("add").click();
    assert!(
        settle(&mut harness, || read().0.len() == 4),
        "the board never took the new column, got {:?}",
        read().0
    );
    assert_eq!(
        read().0,
        ["TODO", "Block", "IN PROGRESS", "DONE"],
        "the column belongs where the menu was opened, not on the end"
    );
}
