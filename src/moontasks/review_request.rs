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
<path under the repo>#<branch> // <conventional commit subject>
  <the commit's paragraph, on one or more indented lines>
```

The branch after `#`, the `//` and the subject after it, and the indented paragraph are each
optional: a line may name a repo and nothing else. Write `.` for the repo the board is in.

```
repos/retro_encabulator/#fix-the-races // fix(encabulator): put the bearing races back
  They were reversed, which is why the flux only ran one way.
repos/turbocharger/ // fix(turbo): widen the intake types
repos/flux_capacitor/
. // chore: take the three submodules forward
```

The order is the deploy order, top to bottom. A line whose repo has nothing left to commit reads
as done on the board, so the list ticks itself off as the person works down it - you do not mark
anything, and nothing but you writes the file. Take a line out when it no longer needs saying.
";

/// What separates the repo a line names from the commit it suggests for it.
const SUGGESTION_MARK: &str = "//";
/// What separates a repo's path from the branch the commit belongs on.
const BRANCH_MARK: char = '#';

/// One line of the file: a repo to review, and what to commit there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct ReviewRequest {
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
    let mut requests = Vec::new();

    for task_id in task_ids {
        let Ok(dir) = store::task_dir(repo_path, &task_id) else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(dir.join(REVIEW_REQUEST_FILE_NAME)) else {
            continue;
        };
        requests.extend(
            parse(&contents)
                .into_iter()
                .map(|request| view_of(repo_path, &task_id, request)),
        );
    }
    requests
}

/// When each of the board's request files was last written, which is what says whether the list
/// has to be read again.
///
/// A file per task, and one `stat` each: cheap enough to do on the board's clock, so a line an
/// agent appends is on the cards within a poll of it being written. A task with no file has no
/// entry, so a file appearing or going is a change like any other.
pub(crate) fn written_at(repo_path: &Path) -> Vec<(String, std::time::SystemTime)> {
    let Ok(task_ids) = store::list_task_ids(repo_path) else {
        return Vec::new();
    };
    task_ids
        .into_iter()
        .filter_map(|task_id| {
            let dir = store::task_dir(repo_path, &task_id).ok()?;
            let written = std::fs::metadata(dir.join(REVIEW_REQUEST_FILE_NAME))
                .ok()?
                .modified()
                .ok()?;
            Some((task_id, written))
        })
        .collect()
}

/// One line of a task's file, against the repo the board belongs to.
///
/// The path is resolved the way the submodule hub resolves a submodule's, so a request and a hub
/// row for the same repo carry the same string and can be told to be the same repo. A path that
/// resolves to no repo is kept as it was written: the row is still worth drawing, and opening it
/// is what says what is wrong with it.
fn view_of(repo_path: &Path, task_id: &str, request: ReviewRequest) -> ReviewRequestView {
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
        path_under_repo: request.path_under_repo,
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
    if crate::git::current_branch_name(repo).ok().flatten().as_deref() == Some(branch) {
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
        path_under_repo,
        branch: branch.filter(|branch| !branch.is_empty()).map(str::to_string),
        suggestion,
    })
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
            requests[0].suggestion.as_ref().map(|one| one.paragraph.as_str()),
            Some("The bearing races were reversed. This puts them back.")
        );

        assert_eq!(requests[1].branch.as_deref(), Some("main"));
        assert_eq!(requests[1].suggestion.as_ref().expect("a suggestion").paragraph, "");

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
        for line in [". // chore: bump the submodules", "/ // chore: bump the submodules"] {
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
            requests[0].suggestion.as_ref().expect("a suggestion").paragraph,
            "It was eating the last line of every file."
        );
        // `src` is no repo of its own, so it resolves to the repo it sits in - which is the repo
        // the second line names outright, and the same string for both.
        assert_eq!(requests[0].repo_path, requests[1].repo_path);
        assert_eq!(requests[1].path_under_repo, "");
        assert_eq!(requests[1].branch, None);

        // The write times are what the window watches, so a file that is there has one.
        let written = written_at(&repo);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].0, "deploy-the-thing-1111");
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
            requests[0].suggestion.as_ref().expect("a suggestion").paragraph,
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

    /// Most repos have no board at all, and every repo with one has tasks that have asked for
    /// nothing. Neither is a failure - they have asked for nothing, which is an empty list.
    #[test]
    fn a_repo_with_no_board_asks_for_nothing() {
        let fixture = crate::native::ui_tests::Fixture::new("review-request-none");

        assert!(list_for_repo(&fixture.root).is_empty());
        assert_eq!(written_at(&fixture.root), []);
    }
}
