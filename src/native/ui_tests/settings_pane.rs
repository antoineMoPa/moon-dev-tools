//! The settings page: the model dropdown, narrowed by what is typed in its filter.

use egui_kittest::{Harness, kittest::Queryable};

use crate::{
    api::AgentKind,
    native::{settings_pane::draw_model_picker, theme::ThemeMode},
    settings::Settings,
};

use super::{app_for, seeded_fixture};

/// The dropdown drawn on its own, set to `opencode`, with models handed to it rather than
/// asked of an `opencode` the machine running the tests may not have.
#[test]
fn typing_in_the_model_filter_narrows_the_models_listed() {
    let fixture = seeded_fixture("settings-pane");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.model.settings = Some(Settings::default());
    app.model.settings_page.opencode_models = Some(Ok(vec![
        "openai/gpt-6-sol".to_string(),
        "opencode/big-pickle".to_string(),
    ]));
    let palette = app.palette_of();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 500.0))
        .build_ui(move |ui| {
            draw_model_picker(&mut app, ui, &palette, AgentKind::OpenCode);
        });
    harness.run_steps(2);

    // Opening the dropdown lists every model, and gives its filter the keyboard.
    harness.get_by_value("OpenCode's default").click();
    harness.run_steps(3);
    assert!(
        harness.query_by_label("openai/gpt-6-sol").is_some(),
        "every model should be listed before anything is typed"
    );

    harness
        .input_mut()
        .events
        .push(egui::Event::Text("PICK".to_string()));
    harness.run_steps(3);

    assert!(harness.query_by_label("opencode/big-pickle").is_some());
    assert!(
        harness.query_by_label("openai/gpt-6-sol").is_none(),
        "a model that does not match the filter should be left out"
    );
}
