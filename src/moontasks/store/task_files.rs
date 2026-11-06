//! The files that sit in a task's folder itself, listed for the task's pane under `Files` -
//! beside the documents its `file_attachments.txt` names.
//!
//! Only the top of the folder is listed: a sub-task's folder is a link in `subtasks`, and that
//! folder is a row of its own on the board. What is left out is what moon writes for itself -
//! the task's record, the briefs it hands the agents, the lists it keeps - and what the pane
//! already shows somewhere else, the notes and the attachments list.

use std::{fs, path::Path};

use super::{METADATA_FILE_NAME, task_dir};
use crate::moontasks::{
    ATTACHMENTS_BRIEF_FILE_NAME, ATTACHMENTS_FILE_NAME, BRIEF_FILE_NAME,
    COORDINATION_BRIEF_FILE_NAME, NOTES_FILE_NAME, REVIEW_REQUEST_BRIEF_FILE_NAME,
    TRACKER_LINK_BRIEF_FILE_NAME,
    review_request::REVIEW_REQUEST_FILE_NAME,
};

/// The files in a task's folder that are not documents the task is about, by name.
const NOT_LISTED: &[&str] = &[
    METADATA_FILE_NAME,
    BRIEF_FILE_NAME,
    NOTES_FILE_NAME,
    ATTACHMENTS_FILE_NAME,
    ATTACHMENTS_BRIEF_FILE_NAME,
    REVIEW_REQUEST_FILE_NAME,
    REVIEW_REQUEST_BRIEF_FILE_NAME,
    COORDINATION_BRIEF_FILE_NAME,
    TRACKER_LINK_BRIEF_FILE_NAME,
];

/// The files directly in a task's folder that the task's pane lists under `Files`, by name,
/// sorted. A dot file is left out, as is a folder, and a file that `attached` already lists -
/// it is drawn as an attachment, once.
///
/// A task whose folder cannot be read has none, the way a task without an attachments file has
/// no attachments.
pub(crate) fn read_folder_files(
    repo_path: &Path,
    task_id: &str,
    attached: &[String],
) -> Vec<String> {
    let Ok(dir) = task_dir(repo_path, task_id) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.') && !NOT_LISTED.contains(&name.as_str()))
        .filter(|name| !attached.iter().any(|listed| listed == name))
        // Followed rather than read off the entry, so a link to a document counts as one.
        .filter(|name| fs::metadata(dir.join(name)).is_ok_and(|meta| meta.is_file()))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo whose one task has `names` in its folder, made fresh for the test named `name`.
    fn repo_with_files(name: &str, names: &[&str]) -> std::path::PathBuf {
        let repo = std::env::temp_dir().join(format!(
            "moonreview-task-files-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&repo);
        let dir = repo.join(".moontasks").join("write-the-parser-1111");
        fs::create_dir_all(dir.join("subtasks")).expect("failed to make the task");
        for file in names {
            fs::write(dir.join(file), "x\n").expect("failed to write a file");
        }
        repo
    }

    #[test]
    fn the_files_a_person_left_in_the_folder_are_listed_and_moons_own_are_not() {
        let repo = repo_with_files(
            "person_left_in_folder",
            &[
                "metadata.json",
                "brief.md",
                "notes.md",
                "file_attachments.txt",
                "attach_files.md",
                "request_review.md",
                "Coordination.md",
                "request_for_review.txt",
                ".hidden",
                "scratch.txt",
                "figures.csv",
            ],
        );
        assert_eq!(
            read_folder_files(&repo, "write-the-parser-1111", &[]),
            vec!["figures.csv".to_string(), "scratch.txt".to_string()]
        );
    }

    #[test]
    fn a_file_that_is_attached_is_not_listed_twice() {
        let repo = repo_with_files("attached_once", &["plan.md", "scratch.txt"]);
        assert_eq!(
            read_folder_files(&repo, "write-the-parser-1111", &["plan.md".to_string()]),
            vec!["scratch.txt".to_string()]
        );
    }

    #[test]
    fn a_folder_is_not_a_file_and_a_task_without_a_folder_has_none() {
        let repo = repo_with_files("folder_is_not_a_file", &["scratch.txt"]);
        assert_eq!(
            read_folder_files(&repo, "write-the-parser-1111", &[]),
            vec!["scratch.txt".to_string()]
        );
        assert!(read_folder_files(&repo, "no-such-task-2222", &[]).is_empty());
    }
}
