//! `request_for_review.txt`: the repos an agent says are ready to be looked at, in the order
//! they have to be deployed.
//!
//! An agent working in a task often finishes with work spread over several repos - a submodule,
//! its parent, another submodule - and the order they must be committed in is the part that
//! matters most and the part prose in `notes.md` cannot be acted on. So the agent writes the
//! list here instead, one repo per line, top to bottom, and the board draws a row for each.
//!
//! The file is the agent's. Nothing here writes it back: an entry goes when the agent takes it
//! out, and the board only reads.
//!
//! The window reads the files itself, off the board's folder, rather than asking the server for
//! them: this is a handful of short files in a folder the window already knows, and a route, a
//! trait method and two implementations of it would be four places to change every time a line
//! of the format does.

use anyhow::{Context, Result};

use std::path::{Path, PathBuf};

use crate::{commit_suggestion::CommitSuggestion, moontasks::ReviewRequestView, moontasks::store};

/// The file, in the task's folder beside `notes.md`.
pub(crate) const REVIEW_REQUEST_FILE_NAME: &str = "request_for_review.txt";

/// What the format is written down in, in the task's folder beside the file it describes.
pub(crate) const REVIEW_REQUEST_BRIEF_FILE_NAME: &str = "request_review.md";

/// The whole of the format, for an agent that has come to write the list.
///
/// It is a file rather than part of the brief because the brief is a system prompt on every run
/// of every task, and this is read once, by the one agent in ten that has work to hand over. The
/// brief says the file is here and says no more; this says the rest.
pub(crate) const REVIEW_REQUEST_BRIEF: &str = "\
# Asking for a review

Write the repos your work touched to `request_for_review.txt` in this task folder, one per line,
in the order they have to be committed and deployed. The board draws a row per line - `pending
turbocharger review` - and the commit pane of each repo offers the message you wrote for it.

One line is:

```
<path to the repo, from the board's repo>#<branch> // <conventional commit subject>
  <the commit's paragraph, on one or more indented lines>
```

The branch after `#`, the `//` and the subject after it, and the indented paragraph are each
optional: a line may name a repo and nothing else. Write `.` for the repo the board is in.

Write the line when the work is there to be looked at, committed or not - the person reviews it
and makes the commit, using the message you wrote. The branch after `#` is the branch the commit
belongs on: name the one you worked on, which you create if you are working on a branch at all.
The review opens wherever that branch is checked out, so a worktree you made is where it goes,
and nobody's checkout is moved to reach it. It is also how one task's commit is told from
another's: the message you wrote is offered on that branch and nowhere else, so a line naming a
branch that has been merged and left behind cannot hand its message to the next piece of work in
the same repo.

```
repos/retro_encabulator/#fix-the-races // fix(encabulator): put the bearing races back
  They were reversed, which is why the flux only ran one way.
repos/turbocharger/ // fix(turbo): widen the intake types
repos/flux_capacitor/
. // chore: take the three submodules forward
```

The order is the deploy order, top to bottom. A line whose repo has nothing left to commit reads
as done on the board, so the list ticks itself off as the person works down it. Take a line out
when it no longer needs saying.

A line starting `x ` has been crossed off by the person - work that is committed and wants no
more looking at. Leave those alone; they are their record, not yours. Moving the card to the
board's DONE column does the same to every line on it at once, and writes nothing here - so a
file that still reads as asking for everything may be asking for nothing.
";

/// What separates the repo a line names from the commit it suggests for it.
const SUGGESTION_MARK: &str = "//";
/// What separates a repo's path from the branch the commit belongs on.
const BRANCH_MARK: char = '#';
/// What an entry that has been dealt with carries at the front of its line, the way a checklist
/// crosses one off. Written by the person, from the row's menu - the board can tell that a repo
/// has nothing left to commit, but not that work already committed and pushed is finished with.
const DONE_MARK: &str = "x ";

/// One line of the file: a repo to review, and what to commit there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct ReviewRequest {
    /// Whether the line is crossed off - see [`DONE_MARK`].
    pub(crate) done: bool,
    /// The repo as the line named it - `repos/turbocharger/`. Empty for the board's own repo,
    /// which is how `.` and `/` are written down as well.
    pub(crate) path_under_repo: String,
    /// The branch the line asked for after its `#`, if it named one.
    pub(crate) branch: Option<String>,
    /// The commit the agent wrote for that repo, for its commit pane to offer. `None` for a
    /// line that named a repo and nothing else.
    pub(crate) suggestion: Option<CommitSuggestion>,
}

/// Every repo the board's tasks ask to have looked at, in the order they are to be deployed:
/// task by task, and within a task the order its file lists them.
///
/// Answers an empty list for a repo with no board, which is most repos. Nothing here is an
/// error: a task with no file has asked for nothing, and one whose file cannot be read is the
/// same to the board as one that has not written it yet.
pub(crate) fn list_for_repo(repo_path: &Path) -> Vec<ReviewRequestView> {
    let Ok(task_ids) = store::list_task_ids(repo_path) else {
        return Vec::new();
    };
    // Read once for the whole board rather than per task: the column that finishes a task is a
    // property of the board, and the rule is off on a board that has no such column.
    let finished_column = store::read_board(repo_path).role(store::CLOSES_REVIEWS_IN);
    let mut requests = Vec::new();

    for task_id in task_ids {
        let Ok(dir) = store::task_dir(repo_path, &task_id) else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(dir.join(REVIEW_REQUEST_FILE_NAME)) else {
            continue;
        };
        // Only for a task that asked for something: most tasks have no file, and reading their
        // metadata to find out where a list they do not have sits would be a read each per tick.
        // A task whose metadata cannot be read is somewhere unknown, which is not the finished
        // column - the same way an unreadable line reads as still wanting a look.
        let finished = finished_column.as_ref().is_some_and(|column| {
            store::read_task(repo_path, &task_id).is_ok_and(|metadata| &metadata.status == column)
        });
        requests.extend(
            parse(&contents)
                .into_iter()
                .enumerate()
                .map(|(index, request)| view_of(repo_path, &task_id, index, finished, request)),
        );
    }
    requests
}

/// What is being done to one entry of a task's file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Amend {
    /// Take the line out - a review that turned out not to be wanted.
    Dismiss,
    /// Cross it off, or put it back. The line stays, because it is still true that the repo was
    /// part of this work; it just no longer wants looking at.
    Done(bool),
}

/// Change one entry of a task's file, by where it sits in it.
///
/// Both of the things that can be done to a request are written to the file, because the file is
/// the list and there is nowhere else for a row to have gone. Dismissing is the same act the
/// brief asks agents to do - take a line out when it no longer needs saying. An agent that writes
/// a line again means it again, and the row comes back, which is right.
///
/// The file is read again here rather than worked from what the board last saw: it may have been
/// written since, and the entry to change is the one at this place in it now.
pub(crate) fn amend(repo_path: &Path, task_id: &str, index: usize, amend: Amend) -> Result<()> {
    let dir = store::task_dir(repo_path, task_id)?;
    let path = dir.join(REVIEW_REQUEST_FILE_NAME);
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let mut kept = String::new();
    let mut entry = 0usize;
    // Everything that is not part of the entry being changed is written back exactly as it was -
    // the file is the agent's, and changing one line is not a licence to reformat the rest of it.
    let mut dropping = false;
    for line in contents.lines() {
        let opens_entry = !line.trim().is_empty()
            && !line.starts_with([' ', '\t'])
            && parse_entry(line).is_some();
        if opens_entry {
            let is_the_one = entry == index;
            entry += 1;
            dropping = is_the_one && amend == Amend::Dismiss;
            if is_the_one && let Amend::Done(done) = amend {
                let bare = strip_done_mark(line).unwrap_or(line);
                kept.push_str(&match done {
                    true => format!("{DONE_MARK}{bare}"),
                    false => bare.to_string(),
                });
                kept.push('\n');
                continue;
            }
        }
        if !dropping {
            kept.push_str(line);
            kept.push('\n');
        }
    }

    // A file with nothing left in it is taken away rather than left empty: there is no list any
    // more, and an empty file reads as one that has not been written yet, which it has not.
    if kept.trim().is_empty() {
        return std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()));
    }
    std::fs::write(&path, kept).with_context(|| format!("failed to write {}", path.display()))
}

/// One line of a task's file, against the repo the board belongs to.
///
/// The path is resolved the way the submodule hub resolves a submodule's, so a request and a hub
/// row for the same repo carry the same string and can be told to be the same repo. A path that
/// resolves to no repo is kept as it was written: the row is still worth drawing, and opening it
/// is what says what is wrong with it.
fn view_of(
    repo_path: &Path,
    task_id: &str,
    index: usize,
    task_finished: bool,
    request: ReviewRequest,
) -> ReviewRequestView {
    let joined = repo_path.join(&request.path_under_repo);
    let repo = crate::git::canonicalize_repo(&joined).unwrap_or(joined);
    // The row is named after the repo, not after wherever its branch is checked out: a worktree
    // is a directory named after a task, and `pending moon-dev-tools review` is what the person
    // is looking for on the card.
    let name = repo
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| repo.display().to_string());
    let reviewed = working_copy_of(&repo, request.branch.as_deref());

    ReviewRequestView {
        task_id: task_id.to_string(),
        index,
        path_under_repo: request.path_under_repo,
        // A path that is not a repo at all cannot be counted, and reads as changed: the row is
        // what the agent asked for, and it should not read as dealt with because nothing could
        // be read there.
        changed_files: crate::git::changed_file_count(&reviewed).unwrap_or(1),
        done: request.done,
        task_finished,
        repo_path: reviewed.display().to_string(),
        name,
        branch: request.branch,
        suggestion: request.suggestion,
    }
}

/// Where the work a line names actually is - which is what its review, its commit and its push
/// are of.
///
/// A line naming a branch means the commit belongs on that branch, not that anyone should be
/// moved onto it. An agent working on a branch usually made a worktree to do it in, precisely so
/// that nobody's HEAD had to move; so that worktree is where the review goes, and committing and
/// pushing there are already on the right branch with nothing checked out and nothing switched.
///
/// The repo itself, when it is the thing on that branch or the line named none. And the repo
/// again when the branch is checked out nowhere - there is nothing better to offer, and the
/// commit pane is what says the branch is not the one that was asked for.
fn working_copy_of(repo: &Path, branch: Option<&str>) -> PathBuf {
    let Some(branch) = branch else {
        return repo.to_path_buf();
    };
    // Asked first, and not only as a shortcut: `git worktree list` inside a submodule names the
    // main worktree by its gitdir under `.git/modules`, which is not where its files are. Every
    // path the listing is trusted for below belongs to a linked worktree, which is a real one.
    if crate::git::current_branch_name(repo)
        .ok()
        .flatten()
        .as_deref()
        == Some(branch)
    {
        return repo.to_path_buf();
    }
    crate::git::worktree_on_branch(repo, branch)
        .ok()
        .flatten()
        .unwrap_or_else(|| repo.to_path_buf())
}

/// Read the file.
///
/// Never fails on content. The file is written by an agent, a line at a time, and half of it is
/// still worth drawing - so a line that does not read as an entry is passed over rather than
/// taking the rest of the list with it.
///
/// | | |
/// | --- | --- |
/// | a line at the left margin | opens an entry: the repo, then `//`, then the commit subject |
/// | a line starting with a space or a tab | continues the entry above it - its paragraph |
/// | a blank line | nothing, and it does not end an entry |
/// | `#` in the repo | the branch the commit belongs on |
pub(crate) fn parse(contents: &str) -> Vec<ReviewRequest> {
    let mut requests: Vec<ReviewRequest> = Vec::new();
    let mut paragraph_of_last: Vec<String> = Vec::new();

    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // Indented: more of the entry above, which is the commit's paragraph. Before the first
        // entry it continues nothing, so it is passed over.
        if line.starts_with([' ', '\t']) {
            if !requests.is_empty() {
                paragraph_of_last.push(line.trim().to_string());
            }
            continue;
        }
        if let Some(request) = parse_entry(line) {
            close_paragraph(&mut requests, &mut paragraph_of_last);
            requests.push(request);
        }
    }
    close_paragraph(&mut requests, &mut paragraph_of_last);
    requests
}

/// Put the lines gathered under an entry into its suggestion, and start gathering again.
///
/// A paragraph under a line that suggested no commit is dropped: there is no message for it to
/// be the body of, and inventing a subject to hang it on would be putting words in the commit.
fn close_paragraph(requests: &mut [ReviewRequest], gathered: &mut Vec<String>) {
    let lines = std::mem::take(gathered);
    if lines.is_empty() {
        return;
    }
    let Some(last) = requests.last_mut() else {
        return;
    };
    let Some(suggestion) = &mut last.suggestion else {
        return;
    };
    suggestion.paragraph = lines.join(" ");
}

/// One line at the left margin, as an entry - or `None` for a line that names no repo at all.
fn parse_entry(line: &str) -> Option<ReviewRequest> {
    let (done, line) = match strip_done_mark(line) {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    let (repo, subject) = match line.split_once(SUGGESTION_MARK) {
        Some((repo, subject)) => (repo, Some(subject.trim())),
        None => (line, None),
    };

    let (path, branch) = match repo.split_once(BRANCH_MARK) {
        Some((path, branch)) => (path, Some(branch.trim())),
        None => (repo, None),
    };

    let path_under_repo = path_of(path)?;
    let suggestion = subject
        .filter(|subject| !subject.is_empty())
        .map(|subject| CommitSuggestion {
            subject: subject.to_string(),
            paragraph: String::new(),
        });

    Some(ReviewRequest {
        done,
        path_under_repo,
        branch: branch
            .filter(|branch| !branch.is_empty())
            .map(str::to_string),
        suggestion,
    })
}

/// The line with its crossed-off mark taken off, or `None` for one that has none. Either case
/// of the letter, because it is typed by hand as often as it is written from the menu.
fn strip_done_mark(line: &str) -> Option<&str> {
    line.strip_prefix(DONE_MARK)
        .or_else(|| line.strip_prefix(&DONE_MARK.to_uppercase()))
}

/// The repo part of a line as a path under the board's repo: no leading or trailing slash, and
/// empty for the board's own repo. `None` when the line named nothing readable as a path.
///
/// The board's own repo has to be written down as something, so `.` and `/` both say it. A line
/// that leaves the repo part empty - one opening with `//`, or with `#` - names no repo at all,
/// and is a different thing from one naming the root.
fn path_of(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let under = path.trim_matches('/').trim();
    if under.is_empty() || under == "." {
        return Some(String::new());
    }
    Some(under.trim_start_matches("./").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject_of(request: &ReviewRequest) -> Option<&str> {
        request
            .suggestion
            .as_ref()
            .map(|suggestion| suggestion.subject.as_str())
    }

    /// The example the board was designed around: three repos in deploy order, two of them the
    /// same repo on different branches, one of them carrying a paragraph.
    #[test]
    fn a_deploy_list_reads_back_in_order() {
        let requests = parse(
            "repos/retro_encabulator/#branch_name // fix(encabulator): Fix the retro encabulator\n\
             \x20 The bearing races were reversed.\n\
             \x20 This puts them back.\n\
             repos/retro_encabulator/#main // fix(turbocharger): Prepare turbocharger types\n\
             \n\
             repos/turbocharger/ // fix(turbo): Fix types\n",
        );

        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path_under_repo, "repos/retro_encabulator");
        assert_eq!(requests[0].branch.as_deref(), Some("branch_name"));
        assert_eq!(
            subject_of(&requests[0]),
            Some("fix(encabulator): Fix the retro encabulator")
        );
        assert_eq!(
            requests[0]
                .suggestion
                .as_ref()
                .map(|one| one.paragraph.as_str()),
            Some("The bearing races were reversed. This puts them back.")
        );

        assert_eq!(requests[1].branch.as_deref(), Some("main"));
        assert_eq!(
            requests[1]
                .suggestion
                .as_ref()
                .expect("a suggestion")
                .paragraph,
            ""
        );

        assert_eq!(requests[2].path_under_repo, "repos/turbocharger");
        assert_eq!(requests[2].branch, None);
    }

    /// A repo and nothing else: somewhere to look, with no commit written for it.
    #[test]
    fn a_line_may_name_a_repo_alone() {
        let requests = parse("repos/turbocharger/\n");

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path_under_repo, "repos/turbocharger");
        assert_eq!(requests[0].suggestion, None);
    }

    /// The board's own repo, which is what a list of submodules is deployed alongside.
    #[test]
    fn the_root_repo_is_named_by_a_dot_or_a_slash() {
        for line in [
            ". // chore: bump the submodules",
            "/ // chore: bump the submodules",
        ] {
            let requests = parse(line);
            assert_eq!(requests.len(), 1, "expected {line} to name the root repo");
            assert_eq!(requests[0].path_under_repo, "");
        }
    }

    /// A paragraph belongs to the entry above it, so an indented line after the second entry
    /// does not land on the first.
    #[test]
    fn a_paragraph_stays_with_its_own_entry() {
        let requests = parse(
            "repos/one/ // fix: one\n\
             repos/two/ // fix: two\n\
             \x20 about two\n",
        );

        assert_eq!(requests[0].suggestion.as_ref().expect("one").paragraph, "");
        assert_eq!(
            requests[1].suggestion.as_ref().expect("two").paragraph,
            "about two"
        );
    }

    /// Half a file is still a list. A line that reads as nothing is passed over and the rest
    /// is drawn.
    #[test]
    fn an_unreadable_line_does_not_take_the_list_with_it() {
        let requests = parse("// not a repo\nrepos/turbocharger/ // fix: types\n");

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path_under_repo, "repos/turbocharger");
    }

    #[test]
    fn an_empty_file_asks_for_nothing() {
        assert_eq!(parse(""), []);
        assert_eq!(parse("\n\n   \n"), []);
    }

    /// The whole read, off a board's folder: the tasks' files, in order, with each line's repo
    /// resolved against the repo the board is in.
    #[test]
    fn a_boards_files_are_read_in_deploy_order() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-list");
        let repo = fixture.root.clone();
        let task_dir = repo.join(".moontasks/deploy-the-thing-1111");
        std::fs::create_dir_all(task_dir.join("src")).expect("failed to make the fixture task");
        std::fs::write(task_dir.join("metadata.json"), "{}\n")
            .expect("failed to write the fixture task");
        std::fs::create_dir_all(repo.join("src")).expect("failed to make the fixture folder");
        std::fs::write(
            task_dir.join(REVIEW_REQUEST_FILE_NAME),
            "src/#the-branch // fix(parser): read the trailing newline\n\
             \x20 It was eating the last line of every file.\n\
             . // chore: take the submodule forward\n",
        )
        .expect("failed to write the fixture request");

        let requests = list_for_repo(&repo);

        assert_eq!(requests.len(), 2, "one per line, in order");
        assert_eq!(requests[0].task_id, "deploy-the-thing-1111");
        assert_eq!(requests[0].path_under_repo, "src");
        assert_eq!(requests[0].branch.as_deref(), Some("the-branch"));
        assert_eq!(
            requests[0]
                .suggestion
                .as_ref()
                .expect("a suggestion")
                .paragraph,
            "It was eating the last line of every file."
        );
        // `src` is no repo of its own, so it resolves to the repo it sits in - which is the repo
        // the second line names outright, and the same string for both.
        assert_eq!(requests[0].repo_path, requests[1].repo_path);
        assert_eq!(requests[1].path_under_repo, "");
        assert_eq!(requests[1].branch, None);
    }

    /// Finishing the card finishes what it was asking for, without touching a line of the file.
    ///
    /// Moving a card to the column that finishes a task is the person saying the work is behind
    /// them - so every repo it still names reads as reviewed at once, rather than being crossed
    /// off one at a time. Dragging it back out is them saying it is not, and the file it never
    /// wrote to is still there to say what it was asking for.
    #[test]
    fn a_task_in_the_finished_column_asks_for_nothing_until_it_is_moved_back() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-finished");
        let repo = fixture.root.clone();
        let task_dir = repo.join(".moontasks/deploy-the-thing-1111");
        std::fs::create_dir_all(&task_dir).expect("failed to make the fixture task");
        let request_file = task_dir.join(REVIEW_REQUEST_FILE_NAME);
        let line = ". // chore: take the submodule forward
";
        std::fs::write(&request_file, line).expect("failed to write the fixture request");
        let card_in = |column: &str| {
            std::fs::write(
                task_dir.join("metadata.json"),
                format!(
                    "{{\"title\": \"Deploy the thing\", \"status\": \"{column}\", \
                     \"created_at_unix\": 1700000000}}\n"
                ),
            )
            .expect("failed to write the fixture task");
            let requests = list_for_repo(&repo);
            assert_eq!(
                requests.len(),
                1,
                "the line is read whichever column it is in"
            );
            requests[0].task_finished
        };

        assert!(!card_in("in_progress"), "a card being worked on still asks");
        assert!(
            card_in(store::CLOSES_REVIEWS_IN),
            "a card finished does not"
        );
        assert_eq!(
            std::fs::read_to_string(&request_file).expect("the file should still be there"),
            line,
            "finishing the card writes nothing, so moving it back asks for the same thing again"
        );
        assert!(!card_in("todo"), "and moving it back does ask again");
    }

    /// A board with no such column has no such rule - the same way a board with no column to
    /// release shells in releases none.
    #[test]
    fn a_board_without_the_finished_column_finishes_nothing() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-no-column");
        let repo = fixture.root.clone();
        let task_dir = repo.join(".moontasks/deploy-the-thing-1111");
        std::fs::create_dir_all(&task_dir).expect("failed to make the fixture task");
        std::fs::write(
            task_dir.join("metadata.json"),
            format!(
                "{{\"title\": \"Deploy the thing\", \"status\": \"{}\", \
                 \"created_at_unix\": 1700000000}}\n",
                store::CLOSES_REVIEWS_IN
            ),
        )
        .expect("failed to write the fixture task");
        std::fs::write(
            task_dir.join(REVIEW_REQUEST_FILE_NAME),
            ". // chore: take the submodule forward
",
        )
        .expect("failed to write the fixture request");
        std::fs::write(
            repo.join(".moontasks/board.json"),
            "{\"columns\": [{\"id\": \"todo\", \"label\": \"TODO\"}]}\n",
        )
        .expect("failed to write the fixture board");

        let requests = list_for_repo(&repo);

        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0].task_finished,
            "with the column gone the rule is off, whatever a card's status still says"
        );
    }

    /// The example in the file the brief sends agents to read has to parse as it says it does.
    ///
    /// It is prose next to a parser, so it can drift from it - and the way it would drift is
    /// silent: reflowing the text and losing the one leading space on the paragraph line leaves
    /// an example that still reads as three repos and quietly stops carrying a paragraph.
    #[test]
    fn the_example_in_the_brief_parses_as_it_says_it_does() {
        let example = REVIEW_REQUEST_BRIEF
            .rsplit("```")
            .nth(1)
            .expect("the brief ends with a fenced example");
        let requests = parse(example);

        assert_eq!(requests.len(), 4, "four lines, four repos: {requests:?}");
        assert_eq!(requests[0].path_under_repo, "repos/retro_encabulator");
        assert_eq!(requests[0].branch.as_deref(), Some("fix-the-races"));
        assert_eq!(
            requests[0]
                .suggestion
                .as_ref()
                .expect("a suggestion")
                .paragraph,
            "They were reversed, which is why the flux only ran one way.",
            "the indented line under the first entry is that commit's paragraph"
        );
        // The line the text calls a repo and nothing else.
        assert_eq!(requests[2].path_under_repo, "repos/flux_capacitor");
        assert_eq!(requests[2].branch, None);
        assert_eq!(requests[2].suggestion, None);
        // And the one that names the repo the board is in.
        assert_eq!(requests[3].path_under_repo, "");
    }

    /// Dismissing takes out the one line it was asked about, its paragraph with it, and leaves
    /// everything else in the file exactly as it was.
    #[test]
    fn dismissing_takes_one_entry_out_of_the_file() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-dismiss");
        let repo = fixture.root.clone();
        let dir = repo.join(".moontasks/deploy-the-thing-1111");
        std::fs::create_dir_all(&dir).expect("failed to make the fixture task");
        std::fs::write(dir.join("metadata.json"), "{}\n").expect("failed to write the task");
        let path = dir.join(REVIEW_REQUEST_FILE_NAME);
        let written = |path: &std::path::Path| std::fs::read_to_string(path).expect("the file");

        std::fs::write(
            &path,
            "# deploy in this order\n\
             repos/one/ // fix: one\n\
             \x20 about one\n\
             repos/two/ // fix: two\n\
             \x20 about two\n\
             repos/three/ // fix: three\n",
        )
        .expect("failed to write the fixture request");

        amend(&repo, "deploy-the-thing-1111", 1, Amend::Dismiss).expect("failed to dismiss");

        assert_eq!(
            written(&path),
            "# deploy in this order\n\
             repos/one/ // fix: one\n\
             \x20 about one\n\
             repos/three/ // fix: three\n",
            "the entry and its paragraph go, and nothing else is touched"
        );

        // The last two go as well. What someone wrote around them stays: it is their file, and
        // a line that was never an entry is not one to take out.
        amend(&repo, "deploy-the-thing-1111", 1, Amend::Dismiss).expect("failed to dismiss");
        amend(&repo, "deploy-the-thing-1111", 0, Amend::Dismiss).expect("failed to dismiss");
        assert_eq!(written(&path), "# deploy in this order\n");
        assert!(list_for_repo(&repo).is_empty(), "and nothing is asked for");

        // A file left holding nothing at all is taken away instead: an empty file reads as one
        // nobody has written yet, which is not what happened.
        std::fs::write(&path, "repos/only/ // fix: only\n").expect("failed to rewrite");
        amend(&repo, "deploy-the-thing-1111", 0, Amend::Dismiss).expect("failed to dismiss");
        assert!(
            !path.exists(),
            "a list with nothing left in it is taken away"
        );
    }

    /// Crossing a line off keeps it and marks it; dismissing takes it away. Both leave the rest
    /// of the file exactly as it was, and crossing off goes back the way it came.
    #[test]
    fn a_line_can_be_crossed_off_and_put_back() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-done");
        let repo = fixture.root.clone();
        let dir = repo.join(".moontasks/deploy-the-thing-1111");
        std::fs::create_dir_all(&dir).expect("failed to make the fixture task");
        std::fs::write(dir.join("metadata.json"), "{}\n").expect("failed to write the task");
        let path = dir.join(REVIEW_REQUEST_FILE_NAME);
        let written = || std::fs::read_to_string(&path).expect("the file");
        std::fs::write(
            &path,
            "repos/one/ // fix: one\n\
             \x20 about one\n\
             repos/two/ // fix: two\n",
        )
        .expect("failed to write the fixture request");

        amend(&repo, "deploy-the-thing-1111", 0, Amend::Done(true)).expect("failed to cross off");
        assert_eq!(
            written(),
            "x repos/one/ // fix: one\n\
             \x20 about one\n\
             repos/two/ // fix: two\n",
            "the line is marked where it stands, paragraph and all left alone"
        );

        let requests = list_for_repo(&repo);
        assert!(requests[0].done, "and reads back as crossed off");
        assert_eq!(
            requests[0].path_under_repo, "repos/one",
            "the mark is not part of the path"
        );
        assert!(!requests[1].done);

        // Crossing off twice does not stack marks, and it goes back the way it came.
        amend(&repo, "deploy-the-thing-1111", 0, Amend::Done(true)).expect("failed to cross off");
        assert!(written().starts_with("x repos/one/"));
        amend(&repo, "deploy-the-thing-1111", 0, Amend::Done(false)).expect("failed to put back");
        assert!(written().starts_with("repos/one/"));
        assert!(!list_for_repo(&repo)[0].done);
    }

    /// A line naming a branch is reviewed where that branch is checked out.
    ///
    /// An agent working on a branch makes a worktree for it so that nobody's HEAD has to move.
    /// The line says which branch the commit belongs on; this is what turns that into the place
    /// the review, the commit and the push happen - with the repo left on whatever it was on.
    #[test]
    fn a_branch_is_reviewed_in_the_worktree_it_is_checked_out_in() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-worktree");
        let repo = fixture.root.clone();
        let beside = repo.join("../work-on-the-parser");
        crate::git::run_git_no_output(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "write-the-parser",
                &beside.display().to_string(),
            ],
        )
        .expect("failed to add the fixture worktree");
        let beside = beside.canonicalize().expect("the worktree is on disk");

        let on_the_branch = working_copy_of(&repo, Some("write-the-parser"));
        assert_eq!(
            on_the_branch, beside,
            "a branch checked out in a worktree is reviewed there"
        );
        assert_eq!(
            crate::git::current_branch_name(&repo)
                .expect("failed to read the branch")
                .as_deref(),
            Some("main"),
            "and the repo is left on the branch it was on"
        );

        // The branch the repo is itself on is the repo, not whatever the listing calls it - which
        // for a submodule is a path under `.git/modules` with no files in it.
        assert_eq!(working_copy_of(&repo, Some("main")), repo);
        // A branch checked out nowhere, and a line naming no branch, are both the repo.
        assert_eq!(working_copy_of(&repo, Some("never-made")), repo);
        assert_eq!(working_copy_of(&repo, None), repo);
    }

    /// A row stops being pending when the place it reviews has nothing left to commit - and that
    /// has to hold for a worktree too.
    ///
    /// This is why the count is taken here rather than off the submodule hub: the hub knows the
    /// repo and its submodules, and a worktree beside one is none of those. A row it could not
    /// be told about would read as pending however long ago the work was committed and pushed,
    /// leaving nothing to do but dismiss it by hand.
    #[test]
    fn a_committed_worktree_stops_reading_as_pending() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-pending");
        let repo = fixture.root.clone();
        let dir = repo.join(".moontasks/deploy-the-thing-1111");
        std::fs::create_dir_all(&dir).expect("failed to make the fixture task");
        std::fs::write(dir.join("metadata.json"), "{}\n").expect("failed to write the task");

        let beside = repo.join("../work-on-the-parser");
        crate::git::run_git_no_output(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "write-the-parser",
                &beside.display().to_string(),
            ],
        )
        .expect("failed to add the fixture worktree");
        std::fs::write(
            dir.join(REVIEW_REQUEST_FILE_NAME),
            ".#write-the-parser // feat: it\n",
        )
        .expect("failed to write the fixture request");

        // Work in the worktree, which is what the line is asking to have looked at.
        std::fs::write(beside.join("parser.rs"), "pub fn parse() {}\n").expect("failed to write");
        let pending = |repo: &std::path::Path| list_for_repo(repo)[0].changed_files > 0;
        assert!(pending(&repo), "work waiting in the worktree is pending");

        crate::git::run_git_no_output(&beside, &["add", "-A"]).expect("failed to stage");
        assert!(pending(&repo), "staged and not committed is still pending");

        crate::git::run_git_no_output(&beside, &["commit", "-m", "feat: it"])
            .expect("failed to commit");
        assert!(
            !pending(&repo),
            "committed in the worktree, so there is nothing left to review there"
        );
    }

    /// Most repos have no board at all, and every repo with one has tasks that have asked for
    /// nothing. Neither is a failure - they have asked for nothing, which is an empty list.
    #[test]
    fn a_repo_with_no_board_asks_for_nothing() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-none");

        assert!(list_for_repo(&fixture.root).is_empty());
    }
}
