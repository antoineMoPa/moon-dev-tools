//! Committing what the review has staged, and pushing it.
//!
//! Both run as `git` in a pty rather than as a captured process: commits here are signed, and
//! the only pinentry many machines have is a terminal one, so the passphrase prompt needs a
//! terminal to appear on. See [`crate::terminal::TerminalProgram::Command`].

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{api::FileChangeKind, git};

/// The status letters `git status --porcelain` reports for the index, and what each one is a
/// change of. `--no-renames` is what keeps a rename off this list: it is reported as the
/// delete and the add it is made of.
const MAP_STATUS_LETTER_TO_CHANGE_KIND: &[(char, FileChangeKind)] = &[
    ('A', FileChangeKind::Added),
    ('D', FileChangeKind::Deleted),
    ('M', FileChangeKind::Modified),
    ('T', FileChangeKind::Modified),
];

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct StagedFile {
    pub(crate) file_path: String,
    pub(crate) change_kind: FileChangeKind,
}

/// What the commit pane needs to know about the repo: what a commit would take in, and where
/// a push would send it.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub(crate) struct CommitState {
    /// `None` on a detached HEAD, which is the one state neither action works from.
    pub(crate) branch_name: Option<String>,
    /// The branch this one tracks, e.g. `origin/main`, once it has one.
    pub(crate) upstream_ref: Option<String>,
    /// Commits this branch has that its upstream does not, and the other way round. Both zero
    /// when there is no upstream.
    pub(crate) ahead: usize,
    pub(crate) behind: usize,
    pub(crate) staged_files: Vec<StagedFile>,
    /// How many files have changes that are not staged, untracked ones included. What "stage
    /// all" would take in.
    pub(crate) unstaged_count: usize,
}

/// What the pane can ask for. The message is carried here rather than written to a file: it
/// goes to git as one argument, so a multi-line message arrives as it was written.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "action", rename_all = "lowercase")]
pub(crate) enum CommitAction {
    Commit { message: String },
    Push,
}

/// `pathspec` is what the review is pointed at, when it is pointed at part of the repo: the
/// pane must not offer to commit changes the review beside it does not show.
pub(crate) fn read_commit_state(repo_path: &Path, pathspec: Option<&str>) -> Result<CommitState> {
    let branch_name = git::current_branch_name(repo_path)?;
    let upstream_ref = git::current_branch_upstream_ref(repo_path)?;
    let (ahead, behind) = match &upstream_ref {
        Some(upstream) => ahead_behind(repo_path, upstream)?,
        None => (0, 0),
    };

    let (staged_files, unstaged_count) = read_status(repo_path, pathspec)?;
    Ok(CommitState {
        branch_name,
        upstream_ref,
        ahead,
        behind,
        staged_files,
        unstaged_count,
    })
}

/// The argv one action runs as, or why it cannot be run at all.
pub(crate) fn command_for(action: &CommitAction, state: &CommitState) -> Result<Vec<String>> {
    match action {
        CommitAction::Commit { message } => {
            if state.staged_files.is_empty() {
                bail!("nothing is staged to commit");
            }
            if message.trim().is_empty() {
                bail!("a commit needs a message");
            }
            Ok(vec![
                "commit".to_string(),
                "-m".to_string(),
                message.clone(),
            ])
        }
        // A branch with no upstream gets one from the push that first sends it, which is the
        // only thing the two pushes differ by.
        CommitAction::Push => match (&state.upstream_ref, &state.branch_name) {
            (Some(_), _) => Ok(vec!["push".to_string()]),
            (None, Some(branch)) => Ok(vec![
                "push".to_string(),
                "-u".to_string(),
                "origin".to_string(),
                branch.clone(),
            ]),
            (None, None) => bail!("HEAD is detached, so there is no branch to push"),
        },
    }
}

fn ahead_behind(repo_path: &Path, upstream: &str) -> Result<(usize, usize)> {
    let range = format!("{upstream}...HEAD");
    // `--left-right` counts each side of the range: the upstream's own commits first, then
    // this branch's. An unborn branch has no HEAD to count from, hence the allowed 128.
    let counts = git::run_git_allow_status(
        repo_path,
        &["rev-list", "--left-right", "--count", &range],
        &[0, 128],
    )?;
    Ok(parse_ahead_behind(&counts))
}

/// `behind` then `ahead`, the order `--left-right` reports them in for `upstream...HEAD`.
fn parse_ahead_behind(counts: &str) -> (usize, usize) {
    let mut fields = counts.split_whitespace();
    let behind = fields.next().and_then(|count| count.parse().ok());
    let ahead = fields.next().and_then(|count| count.parse().ok());
    match (behind, ahead) {
        (Some(behind), Some(ahead)) => (ahead, behind),
        _ => (0, 0),
    }
}

fn read_status(repo_path: &Path, pathspec: Option<&str>) -> Result<(Vec<StagedFile>, usize)> {
    let mut args = vec!["status", "--porcelain", "-z", "--no-renames"];
    git::append_pathspec(&mut args, pathspec);
    let output = git::run_git_bytes(repo_path, &args)?;
    Ok(parse_status(&String::from_utf8_lossy(&output)))
}

/// `git status --porcelain -z` is a run of NUL-terminated records, each a two-letter status
/// and the path it is the status of. The first letter is what the index has, the second what
/// the working tree has beyond it; `??` is a file git has never been told about.
fn parse_status(output: &str) -> (Vec<StagedFile>, usize) {
    let mut staged = Vec::new();
    let mut unstaged = 0;

    for record in output.split('\0').filter(|record| record.len() > 3) {
        let mut letters = record.chars();
        let index = letters.next().expect("a record has an index letter");
        let worktree = letters.next().expect("a record has a worktree letter");
        let file_path = record[3..].to_string();

        if index == '?' {
            unstaged += 1;
            continue;
        }
        if worktree != ' ' {
            unstaged += 1;
        }
        if index == ' ' {
            continue;
        }
        staged.push(StagedFile {
            change_kind: MAP_STATUS_LETTER_TO_CHANGE_KIND
                .iter()
                .find(|(known, _)| *known == index)
                .map(|(_, kind)| *kind)
                .unwrap_or_default(),
            file_path,
        });
    }

    (staged, unstaged)
}

/// The shells one review's commit runs belong to. Owned rather than free-standing, so the
/// workspace does not list a commit as one of its shells — and so the one before it is reaped
/// when the next starts.
fn run_owner(session_id: &str) -> String {
    format!("commit:{session_id}")
}

/// What the commit pane draws: what a commit would take in, and where a push would send it.
pub(crate) fn commit_state(state: &crate::api::AppState, session_id: &str) -> Result<CommitState> {
    let (repo_path, pathspec) = repo_of(state, session_id)?;
    read_commit_state(&repo_path, pathspec.as_deref())
}

/// The repo a review is of, and the part of it the review is pointed at.
fn repo_of(
    state: &crate::api::AppState,
    session_id: &str,
) -> Result<(std::path::PathBuf, Option<String>)> {
    crate::api::with_session(state, session_id, |session| {
        Ok((
            session.repo_path.clone(),
            session.diff_target.pathspec.clone(),
        ))
    })
}

/// Start `git` on one action in a pty, and answer with the terminal it runs in.
///
/// A pty rather than a captured process because of signing: `gpg` asks for the passphrase
/// through pinentry, and a terminal pinentry has nowhere to ask without one. The same goes for
/// anything a hook, or a push over ssh, wants typed.
pub(crate) fn start_commit_run(
    state: &crate::api::AppState,
    session_id: &str,
    action: &CommitAction,
) -> Result<String> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, pathspec) = repo_of(state, session_id)?;
    let command = command_for(action, &read_commit_state(&repo_path, pathspec.as_deref())?)?;

    // One run at a time per review: the pane shows one, and the last one's pty has nothing
    // left to say once the next starts.
    let owner = run_owner(session_id);
    state.terminals.remove_owned_by(&owner);

    state.terminals.spawn(crate::terminal::TerminalSpec {
        cwd: repo_path,
        program: crate::terminal::TerminalProgram::Command("git".to_string()),
        args: command,
        env: Vec::new(),
        owner: Some(owner),
        type_ahead: None,
    })
}

/// How a run ended: `None` while it is still going, the exit code once it is over. Asked for
/// once — see [`crate::terminal::TerminalRegistry::take_outcome`].
pub(crate) fn commit_run_outcome(
    state: &crate::api::AppState,
    session_id: &str,
    terminal_id: &str,
) -> Result<Option<i32>> {
    crate::api::with_session(state, session_id, |_| Ok(()))?;
    Ok(state.terminals.take_outcome(terminal_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(upstream: Option<&str>, branch: Option<&str>, staged: usize) -> CommitState {
        CommitState {
            branch_name: branch.map(ToOwned::to_owned),
            upstream_ref: upstream.map(ToOwned::to_owned),
            ahead: 0,
            behind: 0,
            unstaged_count: 0,
            staged_files: (0..staged)
                .map(|index| StagedFile {
                    file_path: format!("file{index}.rs"),
                    change_kind: FileChangeKind::Modified,
                })
                .collect(),
        }
    }

    #[test]
    fn a_commit_hands_the_whole_message_over_as_one_argument() {
        let message = "subject\n\nand a body that mentions a \"quote\"";
        let command = command_for(
            &CommitAction::Commit {
                message: message.to_string(),
            },
            &state_with(None, Some("main"), 1),
        )
        .expect("expected a command");

        assert_eq!(command, vec!["commit", "-m", message]);
    }

    #[test]
    fn a_commit_with_nothing_staged_is_refused() {
        let refused = command_for(
            &CommitAction::Commit {
                message: "a message".to_string(),
            },
            &state_with(None, Some("main"), 0),
        );

        assert!(refused.is_err(), "nothing was staged");
    }

    #[test]
    fn a_commit_with_a_blank_message_is_refused() {
        let refused = command_for(
            &CommitAction::Commit {
                message: "   \n".to_string(),
            },
            &state_with(None, Some("main"), 1),
        );

        assert!(refused.is_err(), "the message was blank");
    }

    #[test]
    fn a_branch_that_tracks_one_is_pushed_as_it_stands() {
        let command = command_for(
            &CommitAction::Push,
            &state_with(Some("origin/main"), Some("main"), 0),
        )
        .expect("expected a command");

        assert_eq!(command, vec!["push"]);
    }

    #[test]
    fn a_branch_with_no_upstream_gets_one_from_the_push() {
        let command = command_for(&CommitAction::Push, &state_with(None, Some("work"), 0))
            .expect("expected a command");

        assert_eq!(command, vec!["push", "-u", "origin", "work"]);
    }

    #[test]
    fn a_detached_head_has_nothing_to_push() {
        let refused = command_for(&CommitAction::Push, &state_with(None, None, 0));

        assert!(refused.is_err(), "there was no branch");
    }

    #[test]
    fn the_status_reads_a_change_kind_and_a_path_per_staged_file() {
        let (staged, unstaged) =
            parse_status("M  src/git.rs\0A  src/committing.rs\0D  old.rs\0");

        assert_eq!(unstaged, 0);
        assert_eq!(
            staged,
            vec![
                StagedFile {
                    file_path: "src/git.rs".to_string(),
                    change_kind: FileChangeKind::Modified
                },
                StagedFile {
                    file_path: "src/committing.rs".to_string(),
                    change_kind: FileChangeKind::Added
                },
                StagedFile {
                    file_path: "old.rs".to_string(),
                    change_kind: FileChangeKind::Deleted
                },
            ]
        );
    }

    #[test]
    fn a_file_changed_on_both_sides_is_staged_and_unstaged_at_once() {
        let (staged, unstaged) = parse_status("MM src/git.rs\0 M src/lib.rs\0?? new.rs\0");

        assert_eq!(
            staged,
            vec![StagedFile {
                file_path: "src/git.rs".to_string(),
                change_kind: FileChangeKind::Modified
            }]
        );
        assert_eq!(unstaged, 3, "each of the three has work outside the index");
    }

    #[test]
    fn the_upstream_side_of_the_range_is_what_the_branch_is_behind_by() {
        assert_eq!(parse_ahead_behind("3\t1\n"), (1, 3));
    }

    #[test]
    fn a_branch_with_no_upstream_counts_as_level() {
        assert_eq!(parse_ahead_behind(""), (0, 0));
    }

    #[test]
    fn the_commit_state_of_a_repo_reads_what_is_staged() {
        let root = std::env::temp_dir().join(format!("moonreview-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("failed to create the fixture");
        git::run_git_no_output(&root, &["init", "--initial-branch=main"]).expect("failed to init");
        for (key, value) in [("user.email", "test@example.com"), ("user.name", "Test")] {
            git::run_git_no_output(&root, &["config", key, value]).expect("failed to configure");
        }
        std::fs::write(root.join("first.txt"), "one\n").expect("failed to write");
        git::run_git_no_output(&root, &["add", "first.txt"]).expect("failed to stage");

        let state = read_commit_state(&root, None).expect("expected a commit state");

        assert_eq!(state.branch_name.as_deref(), Some("main"));
        assert_eq!(state.upstream_ref, None);
        assert_eq!(
            state.staged_files,
            vec![StagedFile {
                file_path: "first.txt".to_string(),
                change_kind: FileChangeKind::Added
            }]
        );
        assert_eq!(state.unstaged_count, 0);

        let _ = std::fs::remove_dir_all(&root);
    }
}
