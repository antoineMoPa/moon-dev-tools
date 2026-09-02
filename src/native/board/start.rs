//! The `[start]` button and the menu under it: everything one task can start.
//!
//! It is drawn in two places - at the foot of a card, and on the task's own pane - and it is
//! the same button in both, from here, so the offers cannot drift apart between them.

use egui::Ui;

use crate::{
    api::AgentKind,
    moontasks::{StartResourceRequest, TaskResourceKind, TaskView},
    native::{
        app::App,
        board::{BoardAction, agent_label, available_agents, gesture::Controls},
        widgets,
    },
};

/// The `[start]` button, and the menu it opens: a review of the repo, a shell in the task, an
/// agent, a file of the repo linked to the card, or a session one of the agents already has.
/// They were three buttons across a card, which is three times the row for three things you
/// press once each.
///
/// Answers whether its menu is up, which the card reads to keep its offers out while the
/// pointer is down in the menu rather than on the card.
pub(crate) fn draw_button(
    app: &App,
    ui: &mut Ui,
    task: &TaskView,
    card: &mut Controls,
    actions: &mut Vec<BoardAction>,
) -> bool {
    let agents: Vec<AgentKind> = available_agents(app)
        .into_iter()
        .filter(|agent| *agent != AgentKind::None)
        .collect();

    // The menu is built from the button rather than the other way round, so it can be one.
    let (button, menu) =
        egui::containers::menu::MenuButton::from_button(egui::Button::new("[start]").frame(false))
            .ui(ui, |ui| {
                if widgets::clickable(ui.button("review"))
                    .on_hover_text("Open the review of this repo in a tab")
                    .clicked()
                {
                    actions.push(BoardAction::OpenReview(
                        task.repo_path.clone(),
                        task.title.clone(),
                    ));
                    ui.close();
                }

                if widgets::clickable(ui.button("shell"))
                    .on_hover_text("Open a shell in this task")
                    .clicked()
                {
                    actions.push(BoardAction::Start(
                        task.id.clone(),
                        StartResourceRequest {
                            kind: TaskResourceKind::Shell,
                            agent: AgentKind::None,
                        },
                    ));
                    ui.close();
                }

                if widgets::clickable(ui.button("file…"))
                    .on_hover_text("Pick a file of the repo to put on this card, and open it")
                    .clicked()
                {
                    actions.push(BoardAction::PickFile(task.id.clone()));
                    ui.close();
                }

                if agents.is_empty() {
                    return;
                }
                ui.separator();
                for agent in agents {
                    if widgets::clickable(ui.button(agent_label(agent))).clicked() {
                        actions.push(BoardAction::Start(
                            task.id.clone(),
                            StartResourceRequest {
                                kind: TaskResourceKind::Agent,
                                agent,
                            },
                        ));
                        ui.close();
                    }
                }
                // The way back when a run's recorded session id stopped pointing anywhere:
                // pick one straight off the agents' own records instead.
                ui.separator();
                if widgets::clickable(ui.button("attach a session…"))
                    .on_hover_text(
                        "Pick a past session of one of the agents and put it on this task",
                    )
                    .clicked()
                {
                    actions.push(BoardAction::OpenAttachPicker {
                        task_id: task.id.clone(),
                        task_title: task.title.clone(),
                    });
                    ui.close();
                }
            });

    card.pressed(&widgets::clickable(button));
    menu.is_some()
}
