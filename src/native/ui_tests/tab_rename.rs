//! Renaming a shell from its tab: the double click that opens the title for retyping, and
//! the name that is typed going to the server, which is where the tab reads it back from.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use egui_kittest::Harness;

use crate::{
    api::OpenSessionRequest,
    backend::{Backend, local::LocalBackend},
    native::{Launch, app::App, panes::Pane, theme::ThemeMode},
};

use super::{seeded_fixture, settle};

/// A double click on a shell's tab opens its title for retyping, with the keyboard in the
/// box. Enter keeps what was typed - on the tab, and on the server the board reads from -
/// and Escape throws it away.
#[test]
fn a_shells_tab_is_renamed_by_double_clicking_it() {
    let fixture = seeded_fixture("tab-rename");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = Arc::new(LocalBackend::new(state));
    let open = || OpenSessionRequest {
        repo_path: fixture.root.display().to_string(),
        diff_target: None,
        active_commit: None,
    };
    let opened = backend
        .open_session(open())
        .expect("expected the session to open");
    let terminal_id = backend
        .create_terminal(&opened.session_id, None)
        .expect("expected a shell to start");
    // Named before the window ever sees it, the way a shell reattached after a restart is:
    // the tab reads the server's name rather than the shell's own title.
    backend
        .rename_terminal(&opened.session_id, &terminal_id, "first")
        .expect("expected the rename");

    let launch = Launch {
        backend: Arc::clone(&backend) as Arc<dyn Backend>,
        open: Some(open()),
        frame: crate::cli::Frame::Review,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);

    let placed = Arc::new(AtomicBool::new(false));
    let placed_in_ui = Arc::clone(&placed);
    // Where the shell's tab was drawn, once its shell is attached.
    let tab: Arc<Mutex<Option<egui::Rect>>> = Arc::new(Mutex::new(None));
    let tab_in_ui = Arc::clone(&tab);
    // The title as the tab's rename box has it, while one is open.
    let renaming: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let renaming_in_ui = Arc::clone(&renaming);
    // Whether that box has the keyboard yet: it asks for it as it is drawn, and only has it
    // from the frame after, which is the one it is safe to type into.
    let typing_lands = Arc::new(AtomicBool::new(false));
    let typing_lands_in_ui = Arc::clone(&typing_lands);
    // What the window has the shell called.
    let named: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let named_in_ui = Arc::clone(&named);
    let for_pane = terminal_id.clone();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .build_ui(move |ui| {
            if !placed_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let frame = app.model.layout.active_frame();
                app.model.layout.add_pane(
                    frame,
                    Pane::Terminal {
                        terminal_id: for_pane.clone(),
                        command: None,
                        task_id: None,
                    },
                    None,
                );
                placed_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            *tab_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .find_pane(|pane| {
                    matches!(pane, Pane::Terminal { terminal_id, .. } if *terminal_id == for_pane)
                })
                .filter(|_| app.terminals.contains_key(&for_pane))
                .and_then(|(pane, _)| app.frames.tab_rect(pane));
            *renaming_in_ui.lock().expect("poisoned") =
                app.model.renaming_tab.as_ref().map(|rename| rename.name.clone());
            typing_lands_in_ui.store(
                ui.ctx().memory(|memory| memory.focused()).is_some(),
                Ordering::Relaxed,
            );
            *named_in_ui.lock().expect("poisoned") = app.model.terminal_names.get(&for_pane).cloned().flatten();
        });

    assert!(
        settle(&mut harness, || tab.lock().expect("poisoned").is_some()),
        "the shell's tab should be drawn with its shell attached"
    );
    assert_eq!(
        named.lock().expect("poisoned").as_deref(),
        Some("first"),
        "the name is read from the server as the shell attaches"
    );

    let at = tab
        .lock()
        .expect("poisoned")
        .expect("the tab was drawn")
        .center();
    let press_and_release = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let double_click = |harness: &mut Harness<'_>| {
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(at),
            press_and_release(true),
            press_and_release(false),
        ]);
        harness.step();
        harness
            .input_mut()
            .events
            .extend([press_and_release(true), press_and_release(false)]);
        harness.step();
    };
    // Whatever the box holds is replaced wholesale: the name is typed over all of it.
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    let retype = |harness: &mut Harness<'_>, name: &str, then: egui::Key| {
        harness.input_mut().events.extend([
            key(egui::Key::A, egui::Modifiers::COMMAND),
            egui::Event::Text(name.to_string()),
            key(then, egui::Modifiers::NONE),
        ]);
        harness.step();
    };

    double_click(&mut harness);
    assert!(
        settle(&mut harness, || renaming.lock().expect("poisoned").is_some()
            && typing_lands.load(Ordering::Relaxed)),
        "a double click on the tab opens its title for retyping, with the keyboard in it"
    );
    assert_eq!(
        renaming.lock().expect("poisoned").as_deref(),
        Some("first"),
        "and the box opens on the title as it stands"
    );

    retype(&mut harness, "build", egui::Key::Enter);
    assert!(
        settle(&mut harness, || renaming.lock().expect("poisoned").is_none()
            && named.lock().expect("poisoned").as_deref() == Some("build")),
        "Enter keeps the name on the tab"
    );
    assert!(
        settle(&mut harness, || backend
            .terminal_name(&opened.session_id, &terminal_id)
            .expect("expected the shell's name")
            .as_deref()
            == Some("build")),
        "and on the server, which is where the board reads it from"
    );

    double_click(&mut harness);
    assert!(
        settle(&mut harness, || renaming.lock().expect("poisoned").as_deref()
            == Some("build")
            && typing_lands.load(Ordering::Relaxed)),
        "the tab can be renamed again, from the name it has now"
    );
    retype(&mut harness, "nope", egui::Key::Escape);
    assert!(
        settle(&mut harness, || renaming.lock().expect("poisoned").is_none()),
        "Escape closes the box"
    );
    assert_eq!(
        named.lock().expect("poisoned").as_deref(),
        Some("build"),
        "and throws away what was typed"
    );
    assert_eq!(
        backend
            .terminal_name(&opened.session_id, &terminal_id)
            .expect("expected the shell's name")
            .as_deref(),
        Some("build")
    );
}
