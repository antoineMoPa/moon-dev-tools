//! The Files section of a task's pane: the files left in the task's folder, listed beside the
//! documents its attachments file names, and the button that opens the folder itself.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{app_for, seeded_fixture, settle};

/// A task's pane lists the files in its folder under `Files`, after the documents it attaches,
/// and leaves out what moon writes for itself. A click on a text file in the folder opens it in
/// a tab, the way a click on an attachment does.
#[test]
fn a_task_pane_lists_the_files_in_its_folder_under_files() {
    use egui_kittest::kittest::Queryable as _;

    const TASK: &str = "write-the-parser-1111";
    let fixture = seeded_fixture("board-task-folder-files");
    fixture.write(
        &format!(".moontasks/{TASK}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );
    fixture.write(
        &format!(".moontasks/{TASK}/file_attachments.txt"),
        "plan.md\n",
    );
    fixture.write(&format!(".moontasks/{TASK}/plan.md"), "# The plan\n");
    fixture.write(
        &format!(".moontasks/{TASK}/scratch.txt"),
        "a scratch note\n",
    );
    // What moon writes for the agents, which the pane is not for.
    fixture.write(&format!(".moontasks/{TASK}/brief.md"), "# The brief\n");
    fixture.write(
        &format!(".moontasks/{TASK}/attach_files.md"),
        "# Attaching\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // The files open in the window, each with whether it was opened in the window's own session.
    let file_panes = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let file_panes_in_ui = Arc::clone(&file_panes);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::TaskStart {
                    task_id: TASK.to_string(),
                    title: "Write the parser".to_string(),
                });
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model.board.loaded
                    && app
                        .model
                        .board
                        .tasks
                        .iter()
                        .any(|task| task.folder_files.contains(&"scratch.txt".to_string())),
                Ordering::Relaxed,
            );
            let root_session_id = app.model.root_session_id.clone();
            *file_panes_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter_map(|(_, pane)| match pane {
                    Pane::File {
                        file_path,
                        session_id,
                        ..
                    } => Some((file_path.clone(), *session_id == root_session_id)),
                    _ => None,
                })
                .collect();
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the board never read the task's folder"
    );
    harness.run_steps(3);

    assert!(
        harness.query_by_label("Files").is_some(),
        "the task's pane should have a Files section"
    );
    assert!(
        harness.query_by_label("open folder").is_some(),
        "the Files section should offer to open the task's folder"
    );
    assert!(
        harness.query_by_label("plan.md").is_some(),
        "the attached document should be listed"
    );
    for hidden in ["brief.md", "attach_files.md", "metadata.json"] {
        assert!(
            harness.query_by_label(hidden).is_none(),
            "{hidden} is moon's own, and should not be listed under Files"
        );
    }

    harness.get_by_label("scratch.txt").click();
    let files_open = || file_panes.lock().expect("poisoned").clone();
    assert!(
        settle(&mut harness, || !files_open().is_empty()),
        "clicking a file in the folder should have opened a tab on it"
    );
    assert_eq!(
        files_open(),
        vec![(format!(".moontasks/{TASK}/scratch.txt"), true)]
    );
}
