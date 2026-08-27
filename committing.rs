//! Committing what the review has staged, and pushing it.
//!
//! Both run as `git` in a pty rather than as a captured process: commits here are signed, and
//! the only pinentry many machines have is a terminal one, so the passphrase prompt needs a
//! terminal to appear on. See [`crate::terminal::TerminalProgram::Command`].

use std::path::Path;

use anyhow::{Context, Result, bail};
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
    /// Where a plain `git push` would send this branch, when git can tell from its config:
    /// `None` with no upstream, and under `push.default=simple` when the upstream is not
    /// named like the branch.
    pub(crate) push_ref: Option<String>,
    /// Commits this branch has that its upstream does not, and the other way round. Both zero
    /// when there is no upstream.
    pub(crate) ahead: usize,
    pub(crate) behind: usize,
    pub(crate) staged_files: Vec<StagedFile>,
    /// How many files have changes that are not staged, untracked ones included. What "stage
    /// all" would take in.
    pub(crate) unstaged_count: usize,
    /// Whether `gh` is installed on this machine, which is what the pull request button needs:
    /// without it there is nothing to offer.
    pub(crate) gh_installed: bool,
    /// Whether `opencode` is installed, which is what writes the suggested message. Without it
    /// the pane never asks for one, rather than asking and showing what went wrong.
    pub(crate) opencode_installed: bool,
}

/// What the pane can ask for.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "action", rename_all = "lowercase")]
pub(crate) enum CommitAction {
    Commit { message: String },
    Push,
    /// Open the pull request for the pushed branch, in the browser, through `gh`.
    OpenPr,
}

/// The environment the run's shell is given: where to read the commit message from, and where
/// to write down how the command went. Both are named rather than spelled out in the line that
/// is typed, so what the user reads on that line is the command they would have typed.
const MESSAGE_VARIABLE: &str = "MOONREVIEW_RUN_MESSAGE";
const STATUS_VARIABLE: &str = "MOONREVIEW_RUN_STATUS";

/// The two files one review's run uses. They live in the temp dir rather than in the repo, so
/// nothing a run needs can end up staged by the next one; a review runs one command at a time,
/// so its own id is enough to name them apart from every other review's.
fn run_message_path(session_id: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("moonreview-run-{session_id}.message"))
}

fn run_status_path(session_id: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("moonreview-run-{session_id}.status"))
}

/// What the pane is told when the shell a run was going in disappeared before the command said
/// how it went - the user closed it, or it was killed.
const SHELL_WENT_AWAY: i32 = -1;

/// `pathspec` is what the review is pointed at, when it is pointed at part of the repo: the
/// pane must not offer to commit changes the review beside it does not show.
pub(crate) fn read_commit_state(repo_path: &Path, pathspec: Option<&str>) -> Result<CommitState> {
    let branch_name = git::current_branch_name(repo_path)?;
    let upstream_ref = git::current_branch_upstream_ref(repo_path)?;
    let push_ref = git::current_branch_push_ref(repo_path)?;
    let (ahead, behind) = match &upstream_ref {
        Some(upstream) => ahead_behind(repo_path, upstream)?,
        None => (0, 0),
    };

    let (staged_files, unstaged_count) = read_status(repo_path, pathspec)?;
    Ok(CommitState {
        branch_name,
        upstream_ref,
        push_ref,
        ahead,
        behind,
        staged_files,
        unstaged_count,
        gh_installed: crate::agent::command_exists("gh"),
        opencode_installed: crate::agent::command_exists("opencode"),
    })
}

/// The line one action is run as, or why it cannot be run at all.
///
/// A line for a shell rather than an argv, because the shell is the point: it is still there
/// when the command is done, with the output above it, for whoever wants to carry on in the
/// repo from where the run left off.
pub(crate) fn command_for(action: &CommitAction, state: &CommitState) -> Result<String> {
    match action {
        CommitAction::Commit { .. } if state.staged_files.is_empty() => {
            bail!("nothing is staged to commit")
        }
        CommitAction::Commit { message } if message.trim().is_empty() => {
            bail!("a commit needs a message")
        }
        // The message goes in a file rather than on the line: it is the one thing a person
        // writes here, it runs to several lines, and a shell would have to be told to leave
        // every character of it alone.
        CommitAction::Commit { .. } => Ok(format!("git commit -F \"${MESSAGE_VARIABLE}\"")),
        // A branch git cannot push as it stands - no upstream, or one named differently, which
        // `push.default=simple` refuses - is sent to origin under its own name and left
        // tracking that. `HEAD` rather than the branch name keeps the name, which git lets
        // hold `$` and quotes, off a line a shell reads.
        CommitAction::Push => match (&state.branch_name, &state.push_ref) {
            (None, _) => bail!("HEAD is detached, so there is no branch to push"),
            (Some(_), Some(_)) => Ok("git push".to_string()),
            (Some(_), None) => Ok("git push -u origin HEAD".to_string()),
        },
        // `-w` hands the filled-in form to the browser rather than asking for a title and a
        // body in the pty: the description is written where the pull request is read.
        CommitAction::OpenPr => {
            if !state.gh_installed {
                bail!("gh is not installed, so there is nothing to open a pull request with");
            }
            Ok("gh pr create -w".to_string())
        }
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
/// workspace does not list a commit as one of its shells - and so the one before it is reaped
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

/// Start one action in a login shell in the repo, and answer with the terminal it runs in.
///
/// A pty rather than a captured process because of signing: `gpg` asks for the passphrase
/// through pinentry, and a terminal pinentry has nowhere to ask without one. The same goes for
/// anything a hook, or a push over ssh, wants typed.
///
/// A login shell rather than the program itself because of what happens after: the shell is
/// still there when the command is done, in the repo, with the output above it, so the pane's
/// terminal is one to work in rather than a transcript to read. The shell has no exit code to
/// report for the command it ran - it outlives it - so the line it is given writes that status
/// down where [`commit_run_outcome`] reads it.
pub(crate) fn start_commit_run(
    state: &crate::api::AppState,
    session_id: &str,
    action: &CommitAction,
) -> Result<String> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, pathspec) = repo_of(state, session_id)?;
    let command = command_for(action, &read_commit_state(&repo_path, pathspec.as_deref())?)?;

    // Whatever the last run left behind is not this run's answer, and a message left from a
    // commit that is over must not be what the next one takes.
    let message_path = run_message_path(session_id);
    let status_path = run_status_path(session_id);
    let _ = std::fs::remove_file(&status_path);
    match action {
        CommitAction::Commit { message } => std::fs::write(&message_path, message)
            .with_context(|| format!("failed to write the commit message to {message_path:?}"))?,
        _ => {
            let _ = std::fs::remove_file(&message_path);
        }
    }

    // One run at a time per review: the pane shows one, and the last one's shell has nothing
    // left to say once the next starts.
    let owner = run_owner(session_id);
    state.terminals.remove_owned_by(&owner);

    state.terminals.spawn(crate::terminal::TerminalSpec {
        cwd: repo_path,
        program: crate::terminal::TerminalProgram::LoginShell,
        args: Vec::new(),
        env: vec![
            (
                MESSAGE_VARIABLE.to_string(),
                message_path.display().to_string(),
            ),
            (
                STATUS_VARIABLE.to_string(),
                status_path.display().to_string(),
            ),
        ],
        owner: Some(owner),
        type_ahead: Some(format!(
            "{command} ; echo $? > \"${STATUS_VARIABLE}\"\r"
        )),
    })
}

/// How a run ended: `None` while it is still going, the status the shell wrote down once it is
/// over. Read once - the answer is taken away with it, so a pane that asks again is asking
/// about the next run rather than being told about this one twice.
pub(crate) fn commit_run_outcome(
    state: &crate::api::AppState,
    session_id: &str,
    terminal_id: &str,
) -> Result<Option<i32>> {
    crate::api::with_session(state, session_id, |_| Ok(()))?;

    let status_path = run_status_path(session_id);
    // An empty file is `echo` caught halfway through writing it, which is still going.
    let written = std::fs::read_to_string(&status_path).ok();
    let Some(status) = written
        .as_deref()
        .map(str::trim)
        .filter(|status| !status.is_empty())
    else {
        // A shell that is gone said all it is going to say. The command may well have worked,
        // but nothing wrote that down, and the pane is owed an ending either way.
        if written.is_none() && !state.terminals.is_live(terminal_id) {
            return Ok(Some(SHELL_WENT_AWAY));
        }
        return Ok(None);
    };

    let exit_code = status.parse().unwrap_or(SHELL_WENT_AWAY);
    let _ = std::fs::remove_file(&status_path);
    let _ = std::fs::remove_file(run_message_path(session_id));
    Ok(Some(exit_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `push_ref` follows `upstream` the way git's default config makes it: the upstream when
    /// it is named like the branch, nothing otherwise.
    fn state_with(upstream: Option<&str>, branch: Option<&str>, staged: usize) -> CommitState {
        let push_ref = match (upstream, branch) {
            (Some(upstream), Some(branch)) if upstream.ends_with(&format!("/{branch}")) => {
                Some(upstream.to_string())
            }
            _ => None,
        };
        CommitState {
            branch_name: branch.map(ToOwned::to_owned),
            upstream_ref: upstream.map(ToOwned::to_owned),
            push_ref,
            ahead: 0,
            behind: 0,
            unstaged_count: 0,
            gh_installed: true,
            opencode_installed: true,
            staged_files: (0..staged)
                .map(|index| StagedFile {
                    file_path: format!("file{index}.rs"),
                    change_kind: FileChangeKind::Modified,
                })
                .collect(),
        }
    }

    #[test]
    fn a_commit_reads_its_message_from_the_file_the_run_wrote_it_to() {
        let message = "subject\n\nand a body that mentions a \"quote\"";
        let command = command_for(
            &CommitAction::Commit {
                message: message.to_string(),
            },
            &state_with(None, Some("main"), 1),
        )
        .expect("expected a command");

        assert_eq!(command, "git commit -F \"$MOONREVIEW_RUN_MESSAGE\"");
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

        assert_eq!(command, "git push");
    }

    #[test]
    fn a_branch_with_no_upstream_gets_one_from_the_push() {
        let command = command_for(&CommitAction::Push, &state_with(None, Some("work"), 0))
            .expect("expected a command");

        assert_eq!(command, "git push -u origin HEAD");
    }

    /// The state `git switch -c feature origin/dev` leaves a branch in: it tracks the branch it
    /// was started from, and git's default `push.default=simple` refuses a plain `git push`
    /// from there.
    #[test]
    fn a_branch_whose_upstream_has_another_name_is_pushed_under_its_own() {
        let state = state_with(Some("origin/dev"), Some("feature"), 0);
        assert_eq!(state.push_ref, None, "git has nowhere to send a plain push");

        let command = command_for(&CommitAction::Push, &state).expect("expected a command");

        assert_eq!(command, "git push -u origin HEAD");
    }

    #[test]
    fn a_detached_head_has_nothing_to_push() {
        let refused = command_for(&CommitAction::Push, &state_with(None, None, 0));

        assert!(refused.is_err(), "there was no branch");
    }

    #[test]
    fn the_pull_request_is_opened_in_the_browser_by_gh() {
        let command = command_for(
            &CommitAction::OpenPr,
            &state_with(Some("origin/work"), Some("work"), 0),
        )
        .expect("expected a command");

        assert_eq!(command, "gh pr create -w");
    }

    #[test]
    fn a_pull_request_is_refused_where_gh_is_not_installed() {
        let mut state = state_with(Some("origin/work"), Some("work"), 0);
        state.gh_installed = false;

        let refused = command_for(&CommitAction::OpenPr, &state);

        assert!(refused.is_err(), "gh was not installed");
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
