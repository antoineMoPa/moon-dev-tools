//! Sub-task links - which cards are sub-tasks of which, kept as symlinks in the parent's folder.
//!
//! A card's sub-tasks are listed in its `subtasks` folder, one symlink each, named after the
//! sub-task and pointing at its folder beside the parent's:
//!
//! ```text
//! .moontasks/write-the-parser-1111/subtasks/drop-the-old-api-3333 -> ../../drop-the-old-api-3333
//! ```
//!
//! The links are the only record of it, so an agent listing a task's folder sees the work under
//! it, and no task folder ever moves - the paths a running agent or shell was given stay good.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::*;

/// The folder in a task's folder that holds the links to its sub-tasks.
pub(crate) const SUB_TASKS_DIR_NAME: &str = "subtasks";

fn sub_tasks_dir(repo_path: &Path, parent: &str) -> Result<PathBuf> {
    Ok(task_dir(repo_path, parent)?.join(SUB_TASKS_DIR_NAME))
}

/// Where a link to `child` points: its folder, from inside a sibling's `subtasks` folder.
fn link_target(child: &str) -> PathBuf {
    Path::new("..").join("..").join(child)
}

/// The card each sub-task is under, by the sub-task's id, read off every task's `subtasks`
/// folder.
///
/// A `subtasks` folder holds links of the shape [`link_sub_task`] makes and nothing else, and a
/// task is the sub-task of one card at most: anything else is refused rather than guessed at.
pub(crate) fn parents_by_link(repo_path: &Path) -> Result<HashMap<String, String>> {
    let mut parents: HashMap<String, String> = HashMap::new();
    for parent in list_task_ids(repo_path)? {
        let dir = sub_tasks_dir(repo_path, &parent)?;
        if !dir.is_dir() {
            continue;
        }
        for entry in
            fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let child = entry.file_name().to_string_lossy().to_string();
            let target = fs::read_link(&path)
                .with_context(|| format!("{} is not a link to a sub-task", path.display()))?;
            if target != link_target(&child) {
                bail!(
                    "{} points at {}, not at the task it is named after",
                    path.display(),
                    target.display()
                );
            }
            if let Some(other) = parents.insert(child.clone(), parent.clone()) {
                bail!("{child} is linked as a sub-task of both {other} and {parent}");
            }
        }
    }
    Ok(parents)
}

/// Make `child` a sub-task of `parent`, with a link in the parent's `subtasks` folder.
pub(crate) fn link_sub_task(repo_path: &Path, parent: &str, child: &str) -> Result<()> {
    let dir = sub_tasks_dir(repo_path, parent)?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let link = dir.join(child);
    std::os::unix::fs::symlink(link_target(child), &link)
        .with_context(|| format!("failed to link {}", link.display()))
}

/// Take `child` out of `parent`'s sub-tasks, and the `subtasks` folder with it once it is empty.
pub(crate) fn unlink_sub_task(repo_path: &Path, parent: &str, child: &str) -> Result<()> {
    let dir = sub_tasks_dir(repo_path, parent)?;
    let link = dir.join(child);
    fs::remove_file(&link).with_context(|| format!("failed to remove {}", link.display()))?;
    let empty = fs::read_dir(&dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .next()
        .is_none();
    if empty {
        fs::remove_dir(&dir).with_context(|| format!("failed to remove {}", dir.display()))?;
    }
    Ok(())
}
