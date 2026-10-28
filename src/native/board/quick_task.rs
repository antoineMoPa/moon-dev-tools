//! Quick task - a card made from the palette's query, with an agent already working on it.
//!
//! What is typed into the palette is offered as the title of a new card, once per agent that
//! can be started. Picking one makes the card at the top of the board's first column, starts
//! the agent on it - which moves the card to in progress, see
//! `crate::moontasks::service::resources` - and tells the agent the title as its first line,
//! so it starts on the work rather than waiting to be told it. Nothing opens: the palette was
//! opened over something else, and that is what stays in front. The card's agent dot says it
//! started; only a failure gets a message.

use anyhow::Context;

use crate::{
    api::AgentKind,
    moontasks::{
        ColumnEnd, CreateTaskRequest, StartFolder, StartResourceRequest, TaskResourceKind,
    },
    native::{
        app::App,
        board::{agent_label, available_agents},
        palette::{Command, CommandAction},
    },
};

/// The palette's rows for `query` as a new task's title: one per agent that can be started,
/// the window's selected agent first, so Enter on the first of them starts that one. None for
/// an empty query, which is no title.
pub(crate) fn commands(app: &App, query: &str) -> Vec<Command> {
    let title = query.trim();
    if title.is_empty() {
        return Vec::new();
    }
    let selected = app
        .model
        .review_ref(&app.model.root_session_id)
        .and_then(|review| review.payload.as_ref())
        .map(|payload| payload.selected_agent);
    let mut agents: Vec<AgentKind> = available_agents(app)
        .into_iter()
        .filter(|agent| *agent != AgentKind::None)
        .collect();
    // A stable sort: the rest keep the order the server lists them in.
    agents.sort_by_key(|agent| Some(*agent) != selected);
    agents
        .into_iter()
        .map(|agent| Command {
            title: format!(
                "\"{}\" → start with {}",
                cut_title(title),
                agent_label(agent)
            ),
            description: "Start task right away".to_string(),
            action: CommandAction::StartQuickTask {
                title: title.to_string(),
                agent,
            },
            shortcut: None,
        })
        .collect()
}

/// How much of the typed title a row shows. The card and the agent get the whole of it.
const ROW_TITLE_CHARS: usize = 20;

/// The typed title as a row shows it: its first [`ROW_TITLE_CHARS`], and an ellipsis where
/// the rest was.
fn cut_title(title: &str) -> String {
    if title.chars().count() <= ROW_TITLE_CHARS {
        return title.to_string();
    }
    let kept: String = title.chars().take(ROW_TITLE_CHARS).collect();
    format!("{kept}…")
}

/// Make the card, start `agent` on it and tell it `title`.
pub(crate) fn start(app: &mut App, title: String, agent: AgentKind) {
    let session_id = app.model.root_session_id.clone();
    // The filter goes, so the new card is on the board rather than behind a query that was
    // asked before it existed.
    app.model.board.filter.clear();
    app.tasks.spawn(
        move |backend| {
            let column = backend
                .list_columns(&session_id)?
                .into_iter()
                .next()
                .context("the board has no columns to put a card in")?
                .id;
            let task = backend.create_task(
                &session_id,
                &CreateTaskRequest {
                    title: title.clone(),
                    status: column,
                    joins: ColumnEnd::Top,
                },
            )?;
            let terminal_id = backend.start_task_resource(
                &session_id,
                &task.id,
                StartResourceRequest {
                    kind: TaskResourceKind::Agent,
                    agent,
                    opens_in: StartFolder::Repo,
                    unattended: false,
                },
            )?;
            // Waits in the shell until the agent has a box to take it - see `told` in
            // `crate::terminal`.
            backend.tell_terminal(&terminal_id, &title)?;
            Ok(())
        },
        move |model, result| {
            model.board.refresh_requested = true;
            if let Err(error) = result {
                model.error(format!("could not start the quick task: {error}"));
            }
        },
    );
}
