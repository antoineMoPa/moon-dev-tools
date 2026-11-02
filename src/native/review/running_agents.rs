//! Running agents - the agents already working on a task that a review's comments can go to,
//! instead of a run started for them. The task is found from the repo: a review comes from an
//! agent when a task's `request_for_review.txt` asks for it. Comments are pasted into the
//! agent's shell by the server - see `crate::comments`.

use crate::{
    api::{AgentKind, RunningAgent},
    moontasks::TaskResourceKind,
    native::{app::App, model::ReviewState, palette::CommandAction, panes::OpenPaneRequest},
};

/// Where a review's comments go, as the person last chose in a composer's menu.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) enum CommentTarget {
    /// Nothing was chosen: the first running agent, or a new run when none is.
    #[default]
    FirstRunningAgent,
    /// This run, for as long as it is running. Gone, comments go to a new run rather than to
    /// some other agent nobody chose.
    RunningAgent {
        terminal_id: String,
    },
    NewRun,
}

/// A running agent as a composer's menu lists it.
#[derive(Clone)]
pub(crate) struct OfferedAgent {
    pub(crate) agent: RunningAgent,
    pub(crate) needs_you: bool,
    command: Option<AgentKind>,
    task_id: String,
}

/// The agents a review's comments can go to: the running agents of the tasks asking for this
/// repo to be reviewed, those of the task whose review this is first, then the task that wrote
/// its request last, since that is whose work is in the repo now.
pub(crate) fn offered(app: &App, session_id: &str, repo_path: &str) -> Vec<OfferedAgent> {
    if !app.backend().tells_running_agents() {
        return Vec::new();
    }
    let reviewing_task = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.reviewing_task.as_deref());

    let mut requests: Vec<_> = app
        .model
        .review_requests
        .iter()
        .filter(|request| request.repo_path == repo_path && request.pending())
        .collect();
    requests.sort_by_key(|request| {
        (
            Some(request.task_id.as_str()) != reviewing_task,
            std::cmp::Reverse(request.written_at_unix),
        )
    });

    let mut task_ids: Vec<&str> = Vec::new();
    for request in requests {
        if !task_ids.contains(&request.task_id.as_str()) {
            task_ids.push(&request.task_id);
        }
    }

    task_ids
        .into_iter()
        .filter_map(|task_id| app.model.board.tasks.iter().find(|task| task.id == task_id))
        .flat_map(|task| task.resources.iter().map(move |resource| (task, resource)))
        .filter(|(_, resource)| {
            resource.kind == TaskResourceKind::Agent
                && resource.running
                && resource.going_elsewhere_in.is_none()
        })
        .filter_map(|(task, resource)| {
            Some(OfferedAgent {
                agent: RunningAgent {
                    terminal_id: resource.terminal_id.clone()?,
                    name: resource.label.clone(),
                },
                needs_you: resource.attention.is_some(),
                command: (resource.agent != AgentKind::None).then_some(resource.agent),
                task_id: task.id.to_string(),
            })
        })
        .collect()
}

/// The running agent a review's comments go to, or `None` for a new run.
pub(crate) fn chosen<'offered>(
    review: &ReviewState,
    offered: &'offered [OfferedAgent],
) -> Option<&'offered OfferedAgent> {
    match &review.comment_target {
        CommentTarget::FirstRunningAgent => offered.first(),
        CommentTarget::RunningAgent { terminal_id } => offered
            .iter()
            .find(|offered| offered.agent.terminal_id == *terminal_id),
        CommentTarget::NewRun => None,
    }
}

/// Bring the agent comments were just sent to on screen, in place of the agent shown in the
/// Agent tab if another one is.
pub(crate) fn show_told_agent(app: &mut App, told: &OfferedAgent) {
    app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::ToldAgent {
        terminal_id: told.agent.terminal_id.clone(),
        command: told.command,
        task_id: told.task_id.clone(),
    }));
}
