//! What a shell is called: numbered in the order they were opened, or after the task it was
//! started for.

use crate::api::AppState;

use super::TerminalProgram;

/// How much of a task's title a shell of that task carries: enough to tell one task's shells
/// from another's, short enough that the tab still shows the program and the number after it.
const TITLE_IN_NAME_CHARS: usize = 20;

/// The front of a task's title, as it appears in the names of that task's shells. `None` for
/// a title that is nothing but spaces, which names nothing.
pub(crate) fn title_in_name(title: &str) -> Option<String> {
    let front: String = title.chars().take(TITLE_IN_NAME_CHARS).collect();
    let front = front.trim();
    (!front.is_empty()).then(|| front.to_string())
}

/// The prefix every shell of one task and one program shares - `write the parser claude - `,
/// or `shell - ` for a shell of no task. The number that follows it is counted within it, so
/// each task numbers its own runs.
fn name_prefix(task_title: Option<&str>, label: &str) -> String {
    match task_title.and_then(title_in_name) {
        Some(title) => format!("{title} {label} - "),
        None => format!("{label} - "),
    }
}

/// The name a new shell is given: the task it is being started in, the program, and one past
/// the highest number any shell of that same task and program carries, live or written down
/// on a task - `claude - 3` after `claude - 1` and `claude - 2`, whether or not those are
/// still running. A name someone retyped counts for nothing here, whatever it says.
pub(crate) fn numbered_name(
    task_title: Option<&str>,
    program: &TerminalProgram,
    in_use: impl IntoIterator<Item = String>,
) -> String {
    numbered_name_called(task_title, &program.label(), in_use)
}

/// The same, for a shell named after what it does rather than what runs in it - `write the
/// parser explain - 1` for a login shell an explanation of the change runs in.
pub(crate) fn numbered_name_called(
    task_title: Option<&str>,
    label: &str,
    in_use: impl IntoIterator<Item = String>,
) -> String {
    let prefix = name_prefix(task_title, label);
    let highest = in_use
        .into_iter()
        .filter_map(|name| name.strip_prefix(&prefix)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", highest + 1)
}

/// The name to start a shell under. The numbers in use are read off every shell the server
/// has and every run the repo's tasks have written down, so a number is not handed out twice
/// while the run that had it is still on the board - however many times the server has been
/// restarted in between.
pub(crate) fn name_for_new_shell(
    state: &AppState,
    repo_path: &std::path::Path,
    task_title: Option<&str>,
    program: &TerminalProgram,
) -> anyhow::Result<String> {
    let mut in_use = state.terminals.live_names();
    in_use.extend(crate::moontasks::store::recorded_run_names(repo_path)?);
    Ok(numbered_name(task_title, program, in_use))
}

/// The same, for a shell named after what it does - see [`numbered_name_called`].
pub(crate) fn name_for_new_shell_called(
    state: &AppState,
    repo_path: &std::path::Path,
    task_title: Option<&str>,
    label: &str,
) -> anyhow::Result<String> {
    let mut in_use = state.terminals.live_names();
    in_use.extend(crate::moontasks::store::recorded_run_names(repo_path)?);
    Ok(numbered_name_called(task_title, label, in_use))
}

/// Call a shell something else. A task's run is renamed on the task as well, so the name is
/// still there once the shell is gone and a resumed run takes it back.
pub(crate) fn rename(
    state: &AppState,
    session_id: &str,
    terminal_id: &str,
    name: &str,
) -> anyhow::Result<()> {
    state.terminals.rename(terminal_id, name)?;
    if let Some(task_id) = state.terminals.owner(terminal_id) {
        crate::moontasks::service::record_run_name(state, session_id, &task_id, terminal_id, name)?;
    }
    Ok(())
}
