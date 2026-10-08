//! Task shell - which task of which board a command was typed in a shell of.
//!
//! Read out of the environment the shell was started with rather than out of the folder it
//! is in: an agent may have moved into a submodule, which is another repo with another
//! board, or none.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::moontasks::{
    TASK_DIR_ENV_VAR,
    store::{self, TASKS_DIR_NAME},
};

/// The task a command is run for, read off the folder its shell was told it is in.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TaskOfShell {
    /// The repo whose board the task is on.
    pub(super) repo_path: PathBuf,
    pub(super) task_id: String,
}

impl TaskOfShell {
    /// The task whose folder this is: `<repo>/.moontasks/<task id>`, as every process moon
    /// starts for a task is given it. Anything else is not a task's shell, and is refused
    /// rather than read as the nearest thing to one. `command` is what was typed, for the
    /// refusal to name: `moon wire post`.
    pub(super) fn of(command: &str, task_dir: Option<&OsStr>) -> Result<Self> {
        let Some(task_dir) = task_dir.filter(|task_dir| !task_dir.is_empty()) else {
            bail!(
                "`{command}` is run from a task's shell, and this is not one: \
                 {TASK_DIR_ENV_VAR} is not set"
            );
        };
        let task_dir = Path::new(task_dir);
        let not_a_task_folder = || {
            format!(
                "{TASK_DIR_ENV_VAR} is {}, which is not a task's folder in a board's \
                 {TASKS_DIR_NAME}",
                task_dir.display()
            )
        };
        let task_id = task_dir
            .file_name()
            .and_then(OsStr::to_str)
            .with_context(not_a_task_folder)?;
        let board_dir = task_dir.parent().with_context(not_a_task_folder)?;
        if board_dir.file_name() != Some(OsStr::new(TASKS_DIR_NAME)) {
            bail!(not_a_task_folder());
        }
        let repo_path = board_dir.parent().with_context(not_a_task_folder)?;
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            task_id: task_id.to_string(),
        })
    }

    /// The folder of every task of its board, which is what a handle is worked out against.
    /// A board with no task in the folder the shell was told - one deleted since the shell
    /// was started - is refused.
    pub(super) fn folders_of_its_board(&self) -> Result<Vec<String>> {
        let folders = store::list_task_ids(&self.repo_path)?;
        if !folders.contains(&self.task_id) {
            bail!(
                "{} is not a task of the board in {}",
                self.task_id,
                store::tasks_root(&self.repo_path).display()
            );
        }
        Ok(folders)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_is_the_one_whose_folder_the_shell_was_told() {
        let task = TaskOfShell::of(
            "moon wire post",
            Some(OsStr::new(
                "/repos/project/.moontasks/fix-the-races-6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a",
            )),
        )
        .expect("expected a task");

        assert_eq!(
            task,
            TaskOfShell {
                repo_path: PathBuf::from("/repos/project"),
                task_id: "fix-the-races-6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a".to_string(),
            }
        );
    }

    #[test]
    fn a_folder_that_is_no_tasks_is_refused() {
        for not_a_task_folder in ["/repos/project", "/repos/project/src/task", "/"] {
            let error = TaskOfShell::of("moon wire post", Some(OsStr::new(not_a_task_folder)))
                .expect_err("expected a refusal");
            assert!(
                error.to_string().contains("is not a task's folder"),
                "{error}"
            );
        }
    }
}
