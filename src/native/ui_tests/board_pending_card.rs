//! The empty card a column holds while a task is being named, and the way back to the pane it
//! is being named on.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{app_for, seeded_fixture, settle};

/// Clicking the empty card brings the pane the task is being named on back in front, with the
/// keyboard in its title box.
///
/// The case it is for is the one where the two are not the same tab: the board is in front of
/// you, the half-named task is behind a shell, and the card standing for it is the thing on
/// screen to reach for.
#[test]
fn clicking_the_card_being_written_brings_its_pane_back() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = seeded_fixture("pending-card-click");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    // The `+` is asked for rather than clicked: where it lands depends on the column, and what
    // this is about is getting back to the pane it opens.
    let compose = Arc::new(AtomicBool::new(false));
    let compose_in_ui = Arc::clone(&compose);
    // Something else in the same frame, to stand in for the shell you were in.
    let open_a_shell = Arc::new(AtomicBool::new(false));
    let open_a_shell_in_ui = Arc::clone(&open_a_shell);
    // What is in front, and whether the new-task pane is open at all.
    let in_front = Arc::new(Mutex::new(None::<crate::native::panes::PaneKind>));
    let in_front_in_ui = Arc::clone(&in_front);
    let draft_open = Arc::new(AtomicBool::new(false));
    let draft_open_in_ui = Arc::clone(&draft_open);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if compose_in_ui.swap(false, Ordering::Relaxed) {
                crate::native::board::actions::apply(
                    &mut app,
                    crate::native::board::BoardAction::OpenNewTask(
                        crate::moontasks::ColumnId::new("todo"),
                        crate::moontasks::ColumnEnd::Top,
                    ),
                );
            }
            if open_a_shell_in_ui.swap(false, Ordering::Relaxed) {
                app.open_pane(crate::native::panes::OpenPaneRequest::Terminal { command: None });
            }
            app.draw(ui);

            draft_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::NewTask { .. }))
                    .is_some(),
                Ordering::Relaxed,
            );
            *in_front_in_ui.lock().expect("poisoned") =
                app.model.layout.active_pane().map(|(_, pane)| pane.kind());
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read .moontasks"
    );
    compose.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || draft_open.load(Ordering::Relaxed)),
        "the `+` should have opened a pane to write the new task on"
    );
    harness.run_steps(3);
    assert_eq!(
        *in_front.lock().expect("poisoned"),
        Some(crate::native::panes::PaneKind::Start),
        "the new-task pane opens in front"
    );

    // Away from it, the way opening a shell to look at something takes you away from it.
    open_a_shell.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || *in_front.lock().expect("poisoned")
            == Some(crate::native::panes::PaneKind::Terminal)),
        "the shell should be the tab in front now"
    );
    harness.run_steps(3);

    // And back again by the card the column is holding, which is what is on screen.
    let card = harness
        .ctx
        .read_response(crate::native::board::cards::pending_card_id(
            &crate::moontasks::ColumnId::new("todo"),
        ))
        .expect("expected the column to be holding an empty card")
        .rect;
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(card.center()));
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: card.center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }

    assert!(
        settle(&mut harness, || *in_front.lock().expect("poisoned")
            == Some(crate::native::panes::PaneKind::Start)),
        "clicking the empty card should have brought the naming back in front"
    );
    harness.run_steps(3);
    assert!(
        draft_open.load(Ordering::Relaxed),
        "and it is the same pane, with what was written on it - not a second one"
    );

    // The keyboard is in the title box, so what is typed next goes into the name. The two
    // boxes are the pane's; the upper of them is the title.
    let mut boxes: Vec<_> = harness
        .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
        .map(|node| (node.rect(), node.is_focused()))
        .collect();
    boxes.sort_by(|one, other| one.0.center().y.total_cmp(&other.0.center().y));
    assert!(
        boxes
            .first()
            .expect("expected the new task's pane to draw a title box")
            .1,
        "the title box should have the keyboard back"
    );
}
