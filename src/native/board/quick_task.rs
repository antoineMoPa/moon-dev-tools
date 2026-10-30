//! Quick task - a card made from the palette's query, with an agent already working on it.
//!
//! What is typed into the palette is offered as the title of a new card, on one row with a
//! chip per agent that can be started. Enter starts the agent picked, a chip its own. That
//! makes the card at the top of the board's first column, starts the agent on it - which
//! moves the card to in progress, see `crate::moontasks::service::resources` - and tells the
//! agent the title as its first line, so it starts on the work rather than waiting to be
//! told it. Nothing opens: the palette was
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
    palette_history::{HistoryEntry, HistoryRecord, HistoryThing},
};

/// The quick task the palette offers for what is typed: its title, the agents that can be
/// started on it, and the one Enter starts.
pub(crate) struct QuickTask {
    pub(crate) title: String,
    /// In the order the server lists them, which the row's chips keep whichever is picked.
    pub(crate) agents: Vec<AgentKind>,
    pub(crate) agent: AgentKind,
}

/// The quick task for `query` as a title, with the agent the last quick task was started
/// with, or else the window's selected agent, or else the first of them. None for an empty
/// query, which is no title, or a machine without an agent to start.
pub(crate) fn offered(app: &App, query: &str) -> Option<QuickTask> {
    let title = query.trim();
    if title.is_empty() {
        return None;
    }
    let selected = app
        .model
        .review_ref(&app.model.root_session_id)
        .and_then(|review| review.payload.as_ref())
        .map(|payload| payload.selected_agent);
    let last_started = app
        .model
        .settings
        .as_ref()
        .map(|settings| settings.quick_task_agent);
    let agents: Vec<AgentKind> = available_agents(app)
        .into_iter()
        .filter(|agent| *agent != AgentKind::None)
        .collect();
    let first = *agents.first()?;
    let agent = [last_started, selected]
        .into_iter()
        .flatten()
        .find(|agent| agents.contains(agent))
        .unwrap_or(first);
    Some(QuickTask {
        title: title.to_string(),
        agents,
        agent,
    })
}

impl QuickTask {
    /// The palette's row for it: the title, and the agent it starts with.
    pub(crate) fn command(&self) -> Command {
        Command {
            title: format!(
                "\"{}\" → start with {}",
                cut_title(&self.title),
                agent_label(self.agent)
            ),
            description: String::new(),
            action: self.started_with(self.agent),
            shortcut: None,
        }
    }

    /// Where in the row's title the agent's name is, to write it in the agent's colour.
    pub(crate) fn agent_in_title(&self, title: &str) -> std::ops::Range<usize> {
        let label = agent_label(self.agent);
        assert!(
            title.ends_with(&label),
            "a quick task's title ends with its agent"
        );
        title.len() - label.len()..title.len()
    }

    /// What starting it with `agent` does.
    pub(crate) fn started_with(&self, agent: AgentKind) -> CommandAction {
        CommandAction::StartQuickTask {
            title: self.title.clone(),
            agent,
        }
    }
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

/// Make the card, start `agent` on it and tell it `title`, and keep it in the palette's history
/// by the card - see [`crate::native::palette::history`].
pub(crate) fn start(app: &mut App, title: String, agent: AgentKind) {
    app.remember_quick_task_agent(agent);
    let session_id = app.model.root_session_id.clone();
    let gone = crate::native::palette::history::gone_things(app);
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
            let record = HistoryRecord {
                entry: HistoryEntry {
                    thing: HistoryThing::QuickTask {
                        task_id: task.id,
                        title: title.clone(),
                        agent,
                    },
                    ran_at_unix: crate::native::messages::now_unix(),
                },
                gone: gone.clone(),
            };
            // The task is started either way: a history that could not be written is said
            // apart from it.
            let kept = backend.record_in_palette_history(&session_id, &record);
            Ok((record, kept))
        },
        move |model, result| {
            model.board.refresh_requested = true;
            match result {
                Ok((record, kept)) => {
                    crate::native::palette::history::add_recorded(model, record);
                    model.report(
                        kept,
                        "could not keep the quick task in the palette's history",
                    );
                }
                Err(error) => model.error(format!("could not start the quick task: {error}")),
            }
        },
    );
}
