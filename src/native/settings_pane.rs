//! Settings pane - the person's own settings, the ones kept for every project: the agent
//! moon hands its AI jobs to, and the model that agent runs with.
//!
//! The file is `~/.moonreview/settings.json` on the server - see [`crate::settings`] - and
//! each pick here is one [`crate::settings::SettingsChange`] sent to it. "Project
//! Settings…" is the other page, for what is kept in the repo.
//!
//! One agent and one model for every job: comments sent from a review and the commit pane's
//! `[Use AI]` alike. The agent is the review's own selector, so a pick here is the same pick
//! made there. The model is kept per agent, and picked from a dropdown: `opencode` lists its
//! models, so for it the dropdown filters that list; the other agents have no such command,
//! and the dropdown takes what is typed as the model.

use egui::{RichText, Ui};

use crate::{
    api::AgentKind,
    native::{
        app::App,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How wide the model's dropdown and its filter are: a `provider/model` is longer than a
/// word and shorter than a sentence.
const MODEL_PICKER_WIDTH: f32 = 260.0;
/// How tall the dropdown's list gets before it scrolls.
const MODEL_LIST_HEIGHT: f32 = 300.0;
/// The key the listing of `opencode`'s models goes under while it is being asked for.
const OPENCODE_MODELS_KEY: &str = "opencode-models";

/// What the page holds between frames.
#[derive(Default)]
pub(crate) struct SettingsPage {
    /// What `opencode models` listed, or why it would not. `None` until the model dropdown is
    /// first opened on `opencode`.
    pub(crate) opencode_models: Option<Result<Vec<String>, String>>,
    /// What is typed in the dropdown's filter: what narrows `opencode`'s list, or the model
    /// typed for an agent that has none.
    filter: String,
    /// Whether the dropdown was open on the last frame, so the one it opens on empties the
    /// filter and gives it the keyboard.
    model_picker_was_open: bool,
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui) {
    let palette = app.palette_of();

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new("Settings")
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
            widgets::divider(ui, &palette);
            ui.add_space(8.0);

            // A model pick is a change to what the server's file says, so there is nothing to
            // pick from until it has been read.
            if app.model.settings.is_none() {
                ui.horizontal(|ui| {
                    widgets::small_spinner(ui, palette.muted);
                    ui.label(
                        RichText::new("reading the settings…")
                            .color(palette.muted)
                            .size(SMALL_SIZE),
                    );
                });
                return;
            }

            let Some(agent) = draw_agent_row(app, ui, &palette) else {
                return;
            };
            if agent == AgentKind::None {
                return;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("model").size(SMALL_SIZE).color(palette.muted));
                draw_model_picker(app, ui, &palette, agent);
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("used for comments sent from a review and for commit messages")
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
        });
}

/// The agent row, which is the review's own selector, answering the agent it is set to -
/// `None` until the review has come, when there is no selector to show yet.
fn draw_agent_row(app: &mut App, ui: &mut Ui, palette: &Palette) -> Option<AgentKind> {
    let session_id = app.model.root_session_id.clone();
    let Some(payload) = app
        .model
        .review_ref(&session_id)
        .and_then(|review| review.payload.as_ref())
    else {
        ui.label(
            RichText::new("open a project to pick an agent")
                .size(SMALL_SIZE)
                .color(palette.muted),
        );
        return None;
    };
    let selected = payload.selected_agent;
    let agents = payload.available_agents.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("agent").size(SMALL_SIZE).color(palette.muted));
        crate::native::review::header::draw_agent_select(
            app,
            ui,
            &session_id,
            0,
            selected,
            &agents,
            palette,
        );
    });
    Some(selected)
}

/// What the dropdown says for the model picked, or the agent's default.
fn model_said(model: Option<&str>, agent: AgentKind) -> String {
    model.map_or_else(|| format!("{}'s default", agent.label()), str::to_string)
}

/// The model dropdown: the model picked, and once opened, a filter that has the keyboard and
/// the models under it - the agent's default first. Picking one closes it, as Escape does.
pub(in crate::native) fn draw_model_picker(
    app: &mut App,
    ui: &mut Ui,
    palette: &Palette,
    agent: AgentKind,
) {
    let current = app
        .model
        .settings
        .as_ref()
        .and_then(|settings| settings.agent_model(agent))
        .map(str::to_string);

    let mut open = false;
    let mut picked = None;
    let mut ask_again = false;
    egui::ComboBox::from_id_salt("settings-model")
        .selected_text(RichText::new(model_said(current.as_deref(), agent)).size(SMALL_SIZE))
        .width(MODEL_PICKER_WIDTH)
        .height(MODEL_LIST_HEIGHT)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            open = true;
            let page = &mut app.model.settings_page;
            let opening = !page.model_picker_was_open;
            if opening {
                page.filter.clear();
            }
            let filter = ui.add(
                egui::TextEdit::singleline(&mut page.filter)
                    .hint_text("Filter models")
                    .desired_width(MODEL_PICKER_WIDTH)
                    .margin(egui::Margin::symmetric(6, 3)),
            );
            if opening {
                filter.request_focus();
            }
            let typed = page.filter.trim().to_string();
            let query = typed.to_lowercase();

            let mut row = |ui: &mut Ui, model: Option<&str>| {
                if ui
                    .selectable_label(current.as_deref() == model, model_said(model, agent))
                    .clicked()
                {
                    picked = Some(model.map(str::to_string));
                    ui.close();
                }
            };
            row(ui, None);
            match agent {
                // The one agent with a command that lists its models.
                AgentKind::OpenCode => match &page.opencode_models {
                    None => {
                        ui.horizontal(|ui| {
                            widgets::small_spinner(ui, palette.muted);
                            ui.label(
                                RichText::new("asking opencode for its models…")
                                    .color(palette.muted)
                                    .size(SMALL_SIZE),
                            );
                        });
                    }
                    Some(Err(error)) => {
                        ui.label(RichText::new(error).color(palette.warn).size(SMALL_SIZE));
                        ask_again = widgets::clickable(ui.button("ask again")).clicked();
                    }
                    Some(Ok(models)) => {
                        for model in models
                            .iter()
                            .filter(|model| model.to_lowercase().contains(&query))
                        {
                            row(ui, Some(model));
                        }
                    }
                },
                // The rest are told whatever model id is typed, and keep showing the one they
                // were told last.
                _ => {
                    if let Some(current) = &current
                        && current.to_lowercase().contains(&query)
                    {
                        row(ui, Some(current));
                    }
                    if !typed.is_empty() && current.as_deref() != Some(typed.as_str()) {
                        let use_typed = ui.selectable_label(false, format!("use '{typed}'"));
                        if use_typed.clicked() {
                            picked = Some(Some(typed.clone()));
                            ui.close();
                        }
                    }
                }
            }
        })
        .response
        .on_hover_text("the model the agent runs with");
    app.model.settings_page.model_picker_was_open = open;

    if ask_again {
        app.model.settings_page.opencode_models = None;
    }
    if open && agent == AgentKind::OpenCode {
        load_opencode_models(app);
    }
    if let Some(model) = picked {
        app.select_agent_model(agent, model);
    }
}

/// Ask `opencode` for its models, once: the list is what was installed and logged in to when
/// it was asked, and "ask again" is there for after a login.
fn load_opencode_models(app: &mut App) {
    if app.tasks.is_busy(OPENCODE_MODELS_KEY) || app.model.settings_page.opencode_models.is_some() {
        return;
    }
    app.tasks.spawn_keyed(
        Some(OPENCODE_MODELS_KEY.to_string()),
        |backend| backend.opencode_models(),
        |model, result| {
            model.settings_page.opencode_models = Some(result.map_err(|error| format!("{error}")));
        },
    );
}
