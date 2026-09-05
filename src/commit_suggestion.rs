//! A commit message written for the pane by an agent, from what is staged.
//!
//! The same idea as the `commitwriter` command: hand the staged diff to `opencode`, ask for a
//! conventional-commit subject and a short paragraph, and print them. Here they arrive in the
//! commit pane instead, under the message box, for the `[use]` button beside them to put in it.
//!
//! Only the staged changes go in the prompt. The commit the pane is about to make is what is
//! staged, so that is what the message is written from - unstaged work belongs to the commit
//! after this one.

use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::git;

/// How much of the staged diff the prompt carries. A commit of a whole vendored directory runs
/// to megabytes, which is a slow read for an answer no better than the one the first pages give
/// - so the diff is cut here, and the prompt says it was cut.
const DIFF_LIMIT: usize = 60_000;

/// How long the run is given before it is taken to have got stuck. A model reading a diff
/// answers in seconds; a run still going after this is one waiting for something that is never
/// going to come - a provider that never answers, a prompt for a login - and the pane behind it
/// would spin for as long as the window is open.
const ANSWER_LIMIT: Duration = Duration::from_secs(180);
/// How often the run is asked whether it is done, while it is inside that limit.
const ANSWER_POLL: Duration = Duration::from_millis(100);

/// How much of what `opencode` printed goes in an error. Enough to show what went wrong, and
/// not the whole of a help page or a stack of JSON: the error is read on one line of the pane.
const ERROR_DETAIL_LIMIT: usize = 300;

/// How the two lines of the answer are labelled, which is both what the prompt asks for and
/// what [`parse_suggestion`] reads back.
const SUBJECT_LABEL: &str = "commit:";
const PARAGRAPH_LABEL: &str = "pr_paragraph:";

/// A message written for a commit that has not been made yet.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct CommitSuggestion {
    /// One conventional commit subject, the first line of the message.
    pub(crate) subject: String,
    /// A sentence or two under it, which is also what a pull request would say.
    pub(crate) paragraph: String,
}

impl CommitSuggestion {
    /// The two of them as one commit message: subject, blank line, paragraph - which is what
    /// `[use]` puts in the message box.
    pub(crate) fn as_message(&self) -> String {
        format!("{}\n\n{}", self.subject, self.paragraph)
    }
}

/// Write a message for what this review has staged. Blocking: `opencode` is a program that
/// reads a diff and thinks about it, so this is called from a worker thread.
pub(crate) fn suggest_commit_message(
    state: &crate::api::AppState,
    session_id: &str,
) -> Result<CommitSuggestion> {
    let (repo_path, pathspec) = crate::api::with_session(state, session_id, |session| {
        Ok((
            session.repo_path.clone(),
            session.diff_target.pathspec.clone(),
        ))
    })?;

    let mut name_status_args = vec!["diff", "--cached", "--name-status", "--no-renames"];
    git::append_pathspec(&mut name_status_args, pathspec.as_deref());
    let name_status = git::run_git(&repo_path, &name_status_args)?;
    if name_status.trim().is_empty() {
        bail!("nothing is staged to write a message from");
    }

    let mut diff_args = vec![
        "diff",
        "--cached",
        "--no-color",
        "--no-ext-diff",
        "--unified=3",
    ];
    git::append_pathspec(&mut diff_args, pathspec.as_deref());
    let diff = git::run_git(&repo_path, &diff_args)?;
    if diff.trim().is_empty() {
        bail!("the staged diff is empty");
    }

    let prompt = build_prompt(&repo_path, name_status.trim_end(), &diff);
    let raw = run_opencode(&repo_path, &prompt)?;
    parse_suggestion(&raw)
}

/// The diff as the prompt carries it, and whether it had to be cut to get there.
fn diff_for_prompt(diff: &str) -> (&str, bool) {
    if diff.len() <= DIFF_LIMIT {
        return (diff.trim_end(), false);
    }
    // On a line, so the last hunk in the prompt is one the model can read.
    let cut = diff[..DIFF_LIMIT]
        .rfind('\n')
        .expect("a diff longer than the limit has a newline in its first pages");
    (&diff[..cut], true)
}

fn build_prompt(repo_root: &Path, name_status: &str, diff: &str) -> String {
    let (diff, was_cut) = diff_for_prompt(diff);

    let mut prompt = String::new();
    prompt.push_str("You are writing a semantic commit message from the staged git changes.\n");
    prompt.push_str("Return exactly two lines and nothing else:\n");
    prompt.push_str(&format!(
        "{SUBJECT_LABEL} <one conventional commit subject>\n"
    ));
    prompt.push_str(&format!(
        "{PARAGRAPH_LABEL} <one short paragraph, 1-2 sentences, for the commit body>\n"
    ));
    prompt.push_str("Rules:\n");
    prompt.push_str("- Use only the staged changes below; unstaged work is not in this commit.\n");
    prompt.push_str("- Keep the subject concise and specific.\n");
    prompt.push_str(
        "- Keep the paragraph short, factual, and a little more detailed than the subject.\n\n",
    );
    prompt.push_str("Repository root:\n");
    prompt.push_str(&format!("{}\n\n", repo_root.display()));
    prompt.push_str("Staged files:\n");
    prompt.push_str(name_status);
    prompt.push_str("\n\nStaged diff:\n");
    prompt.push_str(diff);
    if was_cut {
        prompt.push_str(
            "\n\n(The diff was cut off here because of its size. Write the message from what is above it.)",
        );
    }
    prompt.push('\n');
    prompt
}

/// Ask `opencode` in the repo, and answer with everything it printed. The prompt goes in on
/// stdin rather than on the command line: a staged diff is far longer than a command line is
/// allowed to be.
fn run_opencode(repo_root: &Path, prompt: &str) -> Result<String> {
    let mut child = Command::new("opencode")
        // No model of our own: whichever one `opencode` is set up to use is the one the user
        // has already chosen, signed into and paid for, and naming another here would be a
        // model that is not there on some other machine.
        .arg("run")
        .arg("--dir")
        .arg(repo_root)
        .current_dir(repo_root)
        // The PATH the availability check found opencode on, so a window opened from a
        // desktop launcher starts it rather than failing to find it.
        .env("PATH", crate::shell_path::installed_tools_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start opencode")?;

    child
        .stdin
        .take()
        .expect("opencode was spawned with a piped stdin")
        .write_all(prompt.as_bytes())
        .context("failed to write the prompt to opencode")?;

    // Both pipes are drained while the run goes: a run that filled one and blocked on it would
    // never reach the exit this waits for.
    let stdout_pipe = child
        .stdout
        .take()
        .expect("opencode was spawned with a piped stdout");
    let stderr_pipe = child
        .stderr
        .take()
        .expect("opencode was spawned with a piped stderr");
    let reading_stdout = thread::spawn(move || read_all(stdout_pipe));
    let reading_stderr = thread::spawn(move || read_all(stderr_pipe));

    let deadline = Instant::now() + ANSWER_LIMIT;
    let status = loop {
        match child.try_wait().context("failed to wait for opencode")? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "opencode did not answer within {} seconds",
                    ANSWER_LIMIT.as_secs()
                );
            }
            None => thread::sleep(ANSWER_POLL),
        }
    };

    let printed = reading_stdout
        .join()
        .expect("the thread reading opencode's output did not panic");
    let complained = reading_stderr
        .join()
        .expect("the thread reading opencode's errors did not panic");
    if !status.success() {
        // An `opencode` that cannot reach the model - no login for the provider, a model it
        // does not know - says so on stderr and ends on a status.
        let detail = if complained.trim().is_empty() {
            &printed
        } else {
            &complained
        };
        bail!(
            "opencode failed (status {}): {}",
            status.code().unwrap_or(-1),
            on_one_line(detail, ERROR_DETAIL_LIMIT)
        );
    }

    Ok(printed)
}

fn read_all(mut pipe: impl Read) -> String {
    let mut bytes = Vec::new();
    // A pipe that could not be read to the end still has what was read before that, which is
    // what the error the caller raises will be made of.
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// What a program printed, as one line short enough to read in the pane: no colours, no line
/// breaks, and cut where it stops fitting.
fn on_one_line(text: &str, limit: usize) -> String {
    let plain = without_ansi_escapes(text);
    let mut line = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > limit {
        line = line.chars().take(limit).collect::<String>() + "…";
    }
    line
}

/// What `opencode` prints is coloured, and a colour is a run of characters the parser would
/// otherwise read as part of a line. This takes them back out.
fn without_ansi_escapes(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut letters = line.chars();
    while let Some(letter) = letters.next() {
        if letter != '\u{1b}' {
            plain.push(letter);
            continue;
        }
        // An escape runs to the letter that ends it - `m` for a colour, and the same shape for
        // the rest of them.
        for letter in letters.by_ref() {
            if letter.is_ascii_alphabetic() {
                break;
            }
        }
    }
    plain
}

/// Read the two labelled lines back out of what the agent printed. Anything else it printed -
/// a banner, a blank line, a stray sentence - is not part of the message and is passed over.
fn parse_suggestion(raw: &str) -> Result<CommitSuggestion> {
    let mut subject = None;
    let mut paragraph = None;

    for line in raw.lines() {
        let line = without_ansi_escapes(line);
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(SUBJECT_LABEL) {
            subject = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix(PARAGRAPH_LABEL) {
            paragraph = Some(rest.trim().to_string());
        }
    }

    let (Some(subject), Some(paragraph)) = (subject, paragraph) else {
        // Which is also where an `opencode` that ended well without running the model lands -
        // one that printed its own help over an argument it did not know, say.
        bail!(
            "opencode did not answer with a `{SUBJECT_LABEL}` and a `{PARAGRAPH_LABEL}` line: {}",
            on_one_line(raw, ERROR_DETAIL_LIMIT)
        );
    };
    if subject.is_empty() {
        bail!("opencode answered with an empty commit subject");
    }
    Ok(CommitSuggestion { subject, paragraph })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_labelled_lines_are_read_out_of_the_answer() {
        // Arrange
        let raw = "\u{1b}[0m\n> build · gpt-5.6-luna\n\u{1b}[0mcommit: fix: stop the poll running twice\n\npr_paragraph: The pane asked the backend on every frame.\n";

        // Act
        let suggestion = parse_suggestion(raw).expect("expected a suggestion");

        // Assert
        assert_eq!(suggestion.subject, "fix: stop the poll running twice");
        assert_eq!(
            suggestion.paragraph,
            "The pane asked the backend on every frame."
        );
        assert_eq!(
            suggestion.as_message(),
            "fix: stop the poll running twice\n\nThe pane asked the backend on every frame."
        );
    }

    #[test]
    fn an_answer_missing_a_line_is_an_error_that_shows_what_came_back() {
        // Arrange
        let raw = "commit: feat: add a thing";

        // Act
        let error = parse_suggestion(raw).expect_err("expected the missing line to be refused");

        // Assert
        assert!(
            error.to_string().contains("feat: add a thing"),
            "the error should carry what opencode actually printed: {error}"
        );
    }

    /// An `opencode` that printed a help page over an argument it did not know ends well and
    /// says nothing about a commit. What the pane is told fits on the line it has for it.
    #[test]
    fn an_answer_that_is_not_a_message_is_cut_down_to_something_readable() {
        // Arrange
        let raw = format!("Options:\n{}", "  --flag  what the flag does\n".repeat(200));

        // Act
        let error = parse_suggestion(&raw).expect_err("expected a help page to be refused");

        // Assert
        let said = error.to_string();
        assert!(
            said.chars().count() < ERROR_DETAIL_LIMIT * 2,
            "the error should be readable on one line, not a help page: {said}"
        );
        assert!(!said.contains('\n'), "the error should be one line: {said}");
        assert!(
            said.ends_with('…'),
            "the error should say it was cut: {said}"
        );
    }

    #[test]
    fn colours_and_line_breaks_are_left_out_of_what_an_error_carries() {
        // Arrange
        let printed = "\u{1b}[91m\u{1b}[1mError: \u{1b}[0m{\n  \"name\": \"UnknownError\"\n}";

        // Act
        let line = on_one_line(printed, ERROR_DETAIL_LIMIT);

        // Assert
        assert_eq!(line, "Error: { \"name\": \"UnknownError\" }");
    }

    #[test]
    fn a_diff_past_the_limit_is_cut_on_a_line() {
        // Arrange
        let diff = "+a line of the diff\n".repeat(DIFF_LIMIT);

        // Act
        let (carried, was_cut) = diff_for_prompt(&diff);

        // Assert
        assert!(was_cut, "a diff this size does not fit in the prompt");
        assert!(carried.len() <= DIFF_LIMIT);
        assert!(
            carried.ends_with("+a line of the diff"),
            "the cut should leave whole lines behind"
        );
    }
}
