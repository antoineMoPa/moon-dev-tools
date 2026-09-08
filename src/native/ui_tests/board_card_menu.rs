//! The menu a right click on a card opens: the task's folder, said and stood in.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::native::theme::ThemeMode;

use super::{app_for, click_at, right_click_at, seeded_fixture, settle};

/// What the right click reached: the two offers, what was copied, and what the shell it
/// started is standing in.
#[derive(Default)]
struct Seen {
    copied: Option<String>,
    shell_says: String,
}

/// A right click is the way to the task's folder: the path on the clipboard, and a shell
/// standing in it - which is the one thing `[start]`'s shell does not offer, since that one
/// comes up in the repo where the work is.
///
/// And it is only that: the card is neither marked nor opened by it, so the menu is read
/// against a board that has not moved under it.
#[test]
fn a_right_click_on_a_card_copies_its_path_and_opens_a_shell_in_its_folder() {
    const TASK: &str = "write-the-parser-1111";

    let fixture = seeded_fixture("card-menu");
    fixture.write(
        &format!(".moontasks/{TASK}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );
    // As the session has the repo, which is the path the board answers with: a temp dir on
    // macOS is reached through a symlink, and the two spellings of it are not the same string.
    let task_dir = std::fs::canonicalize(&fixture.root)
        .expect("the fixture repo is there")
        .join(".moontasks")
        .join(TASK);

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let pane_open = Arc::new(AtomicBool::new(false));
    let pane_open_in_ui = Arc::clone(&pane_open);
    let marked = Arc::new(AtomicBool::new(false));
    let marked_in_ui = Arc::clone(&marked);
    // Asked its folder once the shell is there, and only once: the answer is what the shell
    // prints, and asking every frame would be typing over it.
    let asked = Arc::new(AtomicBool::new(false));
    let asked_in_ui = Arc::clone(&asked);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }

            app.draw(ui);

            let mut seen = seen_in_ui.lock().expect("poisoned");
            if let Some(text) = ui.ctx().output(|output| {
                output.commands.iter().find_map(|command| match command {
                    egui::OutputCommand::CopyText(text) => Some(text.clone()),
                    _ => None,
                })
            }) {
                seen.copied = Some(text);
            }
            // The shell the menu started, asked where it is standing. `basename` rather than
            // `pwd`, so the answer is one short word that cannot be broken over two rows of
            // the terminal - and so a shell in the repo cannot be mistaken for one in the
            // task folder by a path that has both in it.
            for terminal in app.terminals.values_mut() {
                if !asked_in_ui.swap(true, Ordering::Relaxed) {
                    terminal
                        .send(b"basename \"$PWD\"\n")
                        .expect("expected to write to the shell");
                }
                seen.shell_says = terminal.visible_text().unwrap_or_default();
            }
            pane_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| pane.kind() == crate::native::panes::PaneKind::Terminal)
                    .is_some(),
                Ordering::Relaxed,
            );
            marked_in_ui.store(!app.model.board.marked.is_empty(), Ordering::Relaxed);
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read the task out of .moontasks"
    );

    let card = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(TASK))
        .expect("expected the card to have been drawn")
        .rect;
    right_click_at(&mut harness, card.center());

    assert!(
        harness.query_by_label("copy task path").is_some(),
        "a right click on a card should offer its path"
    );
    assert!(
        harness.query_by_label("open shell in the task folder").is_some(),
        "and a shell standing in its folder"
    );
    assert!(
        !marked.load(Ordering::Relaxed),
        "a right click opens the menu and does nothing else to the card"
    );

    let copy = harness.get_by_label("copy task path").rect().center();
    click_at(&mut harness, copy);
    assert!(
        settle(&mut harness, || seen.lock().expect("poisoned").copied
            == Some(task_dir.display().to_string())),
        "copying should have put the task's folder on the clipboard, got {:?}",
        seen.lock().expect("poisoned").copied
    );
    assert!(
        harness.query_by_label("copy task path").is_none(),
        "and the menu should have closed behind the answer"
    );

    right_click_at(&mut harness, card.center());
    let shell = harness
        .get_by_label("open shell in the task folder")
        .rect()
        .center();
    click_at(&mut harness, shell);
    assert!(
        settle(&mut harness, || pane_open.load(Ordering::Relaxed)),
        "the menu's shell should have opened a terminal"
    );
    assert!(
        settle(&mut harness, || seen
            .lock()
            .expect("poisoned")
            .shell_says
            .contains(TASK)),
        "the shell should be standing in the task's folder; it said:\n{}",
        seen.lock().expect("poisoned").shell_says
    );
}
