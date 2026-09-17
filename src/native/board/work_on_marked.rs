//! One task made to do several at once: the marked cards, handed to a single new card.
//!
//! An agent is often given a handful of small tasks in one go. The new card is what that agent
//! works under, and its notes point at the folders of the tasks it is to do - each one keeps
//! its own notes and whatever was left in it, so the new card says where they are rather than
//! copying what they say.

use crate::moontasks::{ColumnId, TaskView};

/// The new card, made out of the marked ones.
#[derive(Debug, PartialEq)]
pub(crate) struct WorkOnMarked {
    pub(crate) title: String,
    pub(crate) notes: String,
    /// The column the first of the marked cards is in: the new card joins the tasks it is for.
    pub(crate) column: ColumnId,
}

/// The card for these marked tasks, read in the order the board holds them. `None` for fewer
/// than two: one task is worked on from its own card.
pub(crate) fn from_marked<'a>(marked: impl Iterator<Item = &'a TaskView>) -> Option<WorkOnMarked> {
    let marked: Vec<&TaskView> = marked.collect();
    let [first, _, ..] = marked.as_slice() else {
        return None;
    };
    let titles: Vec<&str> = marked.iter().map(|task| task.title.as_str()).collect();
    let folders: String = marked
        .iter()
        .map(|task| format!("- {}: {}\n", task.title, task.dir_path))
        .collect();
    Some(WorkOnMarked {
        title: format!("Work on {}", titles.join(" + ")),
        notes: format!(
            "Work on these tasks together. Each folder holds that task's notes.md and anything \
             left in it.\n\n{folders}"
        ),
        column: first.status.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, status: &str) -> TaskView {
        TaskView {
            id: format!("{title}-1111"),
            title: title.to_string(),
            status: ColumnId::new(status),
            created_at_unix: 1700000000,
            dir_path: format!("/repo/.moontasks/{title}-1111"),
            repo_path: "/repo".to_string(),
            tags: Vec::new(),
            notes: String::new(),
            resources: Vec::new(),
        }
    }

    #[test]
    fn one_marked_task_makes_no_card() {
        let tasks = [task("parser", "todo")];

        assert_eq!(from_marked(tasks.iter()), None);
    }

    #[test]
    fn several_marked_tasks_make_a_card_pointing_at_their_folders() {
        let tasks = [task("parser", "doing"), task("login", "todo")];

        assert_eq!(
            from_marked(tasks.iter()),
            Some(WorkOnMarked {
                title: "Work on parser + login".to_string(),
                notes: "Work on these tasks together. Each folder holds that task's notes.md \
                        and anything left in it.\n\n\
                        - parser: /repo/.moontasks/parser-1111\n\
                        - login: /repo/.moontasks/login-1111\n"
                    .to_string(),
                column: ColumnId::new("doing"),
            })
        );
    }
}
