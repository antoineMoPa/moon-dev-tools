//! The wire's direct messages: which shells a line for one task is typed into.
//!
//! A task's agents are the runs written on it, and a run that is going says which shell it is
//! in and which moon holds that shell - see [`TaskResource::terminal_owner`]. That moon is the
//! one that can type into it, and it is asked to over its window's socket - see
//! `crate::instances`.

use crate::moontasks::store::{TaskMetadata, TaskResource, TaskResourceKind};

/// One agent of a task that is running right now, as a line reaches it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RunningAgent {
    /// The shell it is in, by the id the moon holding it knows it under.
    pub(crate) terminal_id: String,
    /// The process of that moon.
    pub(crate) held_by: u32,
}

/// The agents of a task that are running: the runs with a shell written down whose moon is
/// still there. A run whose moon has gone is one no board has read since, and it is over.
pub(crate) fn running_agents(metadata: &TaskMetadata) -> Vec<RunningAgent> {
    metadata
        .resources
        .iter()
        .filter_map(running_agent)
        .collect()
}

fn running_agent(resource: &TaskResource) -> Option<RunningAgent> {
    if resource.kind != TaskResourceKind::Agent {
        return None;
    }
    let held_by = resource
        .terminal_owner
        .filter(|owner| crate::moontasks::service::process_is_running(*owner))?;
    Some(RunningAgent {
        terminal_id: resource.terminal_id.clone()?,
        held_by,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AgentKind;

    /// A pid nothing is running under: higher than any pid a system hands out.
    const PID_OF_NOTHING: u32 = 4_194_303;

    fn run(
        kind: TaskResourceKind,
        terminal_id: Option<&str>,
        terminal_owner: Option<u32>,
    ) -> TaskResource {
        TaskResource {
            id: "run".to_string(),
            kind,
            agent: AgentKind::Claude,
            file_path: None,
            terminal_id: terminal_id.map(str::to_string),
            terminal_owner,
            agent_session_id: None,
            name: None,
            started_at_unix: 0,
        }
    }

    #[test]
    fn only_an_agent_in_a_shell_a_running_moon_holds_is_running() {
        let this_process = std::process::id();
        let metadata = TaskMetadata {
            title: "Fix the races".to_string(),
            status: None,
            created_at_unix: 0,
            entered_column_at_unix: None,
            position: 0,
            tags: Vec::new(),
            remote_task_tracker_url: String::new(),
            resources: vec![
                run(
                    TaskResourceKind::Agent,
                    Some("terminal-a"),
                    Some(this_process),
                ),
                // Ended: the board has taken its shell off the record.
                run(TaskResourceKind::Agent, None, None),
                // Its moon is gone, and no board has read the record since.
                run(
                    TaskResourceKind::Agent,
                    Some("terminal-b"),
                    Some(PID_OF_NOTHING),
                ),
                // Written down before runs said which moon holds them.
                run(TaskResourceKind::Agent, Some("terminal-c"), None),
                run(TaskResourceKind::File, None, None),
                run(
                    TaskResourceKind::Agent,
                    Some("terminal-d"),
                    Some(this_process),
                ),
            ],
        };

        assert_eq!(
            running_agents(&metadata),
            vec![
                RunningAgent {
                    terminal_id: "terminal-a".to_string(),
                    held_by: this_process,
                },
                RunningAgent {
                    terminal_id: "terminal-d".to_string(),
                    held_by: this_process,
                },
            ]
        );
    }
}
