//! Everything one task can start, and the two ways it is offered: the `[start]` menu at the
//! foot of a card, and a list of buttons on the task's own pane.
//!
//! Both draw the one list from [`offers`], so what a card can start and what its pane can
//! start cannot drift apart. The card gets a menu because a card has one row to spare; the
//! pane has a column to itself, so it lays every offer out where it can be pressed at once.

use egui::Ui;

use crate::{
    api::AgentKind,
    moontasks::{StartFolder, StartResourceRequest, TaskResourceKind, TaskView},
    native::{
        app::App,
        board::{BoardAction, agent_label, available_agents, gesture::Controls},
        widgets,
    },
};

/// The gap the list leaves before a group, which is what sets one apart from the last.
const GROUP_GAP: f32 = 10.0;

/// One thing a task can start, as the menu and the list both show it.
struct StartOffer {
    label: String,
    hover: Option<&'static str>,
    action: BoardAction,
}

/// What this task can start, in the order it is offered: a review of the repo, a shell in the
/// task and a file of the repo linked to the card; then an agent, one per kind the review
/// knows; then an explanation of the repo's changes, written by the agent the review's
/// selector is set to; then a session one of those agents already has. The last three are not
/// offered when there are no agents, since a session is an agent's and an explanation is
/// written by one.
///
/// Answered in groups, which the menu separates with a rule and the list with a gap.
fn offers(app: &App, task: &TaskView) -> Vec<Vec<StartOffer>> {
    let agents: Vec<AgentKind> = available_agents(app)
        .into_iter()
        .filter(|agent| *agent != AgentKind::None)
        .collect();

    let mut groups = vec![vec![
        StartOffer {
            label: "review".to_string(),
            hover: Some("Open the review of this repo in a tab"),
            action: BoardAction::OpenReview(task.repo_path.clone(), task.title.clone()),
        },
        StartOffer {
            label: "shell".to_string(),
            hover: Some("Open a shell in this task"),
            action: BoardAction::Start(
                task.id.clone(),
                StartResourceRequest {
                    kind: TaskResourceKind::Shell,
                    agent: AgentKind::None,
                    opens_in: StartFolder::Repo,
                },
            ),
        },
        StartOffer {
            label: "file…".to_string(),
            hover: Some("Pick a file of the repo to put on this card, and open it"),
            action: BoardAction::PickFile(task.id.clone()),
        },
    ]];

    if agents.is_empty() {
        return groups;
    }
    groups.push(
        agents
            .into_iter()
            .map(|agent| StartOffer {
                label: agent_label(agent),
                hover: None,
                action: BoardAction::Start(
                    task.id.clone(),
                    StartResourceRequest {
                        kind: TaskResourceKind::Agent,
                        agent,
                        opens_in: StartFolder::Repo,
                    },
                ),
            })
            .collect(),
    );
    groups.push(vec![StartOffer {
        label: "explain".to_string(),
        hover: Some(
            "Have the agent picked in the review write a short PDF about everything \
             uncommitted in the repo and its submodules, and open it - in a shell of \
             this task, which is where to watch it",
        ),
        action: BoardAction::Explain(task.id.clone()),
    }]);
    // The way back when a run's recorded session id stopped pointing anywhere: pick one
    // straight off the agents' own records instead.
    groups.push(vec![StartOffer {
        label: "attach a session…".to_string(),
        hover: Some("Pick a past session of one of the agents and put it on this task"),
        action: BoardAction::OpenAttachPicker {
            task_id: task.id.clone(),
            task_title: task.title.clone(),
        },
    }]);
    groups
}

/// The `[start]` button at the foot of a card, and the menu of [`offers`] it opens. They were
/// three buttons across a card, which is three times the row for three things you press once
/// each.
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
    let groups = offers(app, task);

    // The menu is built from the button rather than the other way round, so it can be one.
    let (button, menu) =
        egui::containers::menu::MenuButton::from_button(egui::Button::new("[start]").frame(false))
            .ui(ui, |ui| {
                for (index, group) in groups.into_iter().enumerate() {
                    if index > 0 {
                        ui.separator();
                    }
                    for offer in group {
                        let mut response = widgets::clickable(ui.button(offer.label));
                        if let Some(hover) = offer.hover {
                            response = response.on_hover_text(hover);
                        }
                        if response.clicked() {
                            actions.push(offer.action);
                            ui.close();
                        }
                    }
                }
            });

    card.pressed(&widgets::clickable(button));
    menu.is_some()
}

/// The same [`offers`] as a list of buttons, one to a line, for the task's pane: every one
/// of them in sight, in the buttons an extension pane's action row is made of. A group after
/// the first stands a gap below the last.
pub(crate) fn draw_list(app: &App, ui: &mut Ui, task: &TaskView, actions: &mut Vec<BoardAction>) {
    ui.vertical(|ui| {
        for (index, group) in offers(app, task).into_iter().enumerate() {
            if index > 0 {
                ui.add_space(GROUP_GAP);
            }
            for offer in group {
                let mut response = widgets::small_button(ui, &offer.label, true);
                if let Some(hover) = offer.hover {
                    response = response.on_hover_text(hover);
                }
                if response.clicked() {
                    actions.push(offer.action);
                }
            }
        }
    });
}
