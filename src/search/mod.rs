//! Searching the repo with `ag`, by file name and by what the files hold.
//!
//! `ag` leaves out whatever the repo ignores, which is the whole reason it is here rather
//! than a directory walk of our own: honouring `.gitignore` is its job and it already does
//! it - and so is leaving the ignored files in when asked, for the repos that gitignore
//! their submodules. The searches run where the repo is - beside the server on a `--remote`
//! connection - so this sits with the rest of the service.
//!
//! A search reports as it goes rather than once at the end: a large tree takes `ag` seconds
//! to walk, and the first match is worth showing the moment it is in. And a search nobody
//! is waiting for any more - its query typed over, its palette put away - is stopped where
//! it is, `ag` and all, rather than walked to the end for nothing.

pub(crate) mod file_contents;
pub(crate) mod file_names;

use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

use crate::api::{SearchProgress, SearchScope};

/// The searcher. Required to be installed - it is what makes the searches ignore-aware.
const SEARCHER: &str = "ag";

/// The arguments both searches are run with: dotfiles are files of the repo, and
/// `.github/workflows/ci.yml` is worth finding.
const REPO_ARGS: &[&str] = &["--hidden", "--nocolor"];

/// What no search reads, in either scope: the object store under `.git`, and `node_modules`,
/// which is never the code being looked for and dwarfs the repo it sits in - "include
/// gitignored" is for a submodule the repo ignores, not for a walk of every dependency.
const NEVER_SEARCHED: &[&str] = &[".git", "node_modules"];

/// How often a running search reports what came in, and asks whether it is still wanted.
const TICK: Duration = Duration::from_millis(50);

/// Who a search reports to while it runs. Told what it has found every time that changes,
/// and asked on every tick whether anyone still wants it - a search whose query has been
/// typed over is stopped where it is.
pub(crate) trait SearchListener<T> {
    fn wanted(&mut self) -> bool;
    fn found(&mut self, progress: SearchProgress<T>);
}

/// Whether a search goes on, as the layer reading its lines says after each batch.
enum Flow {
    Continue,
    Stop,
}

/// How a search ended: `ag` printed everything it had, or was told to stop first.
#[derive(PartialEq, Eq, Debug)]
enum Ended {
    Finished,
    Stopped,
}

/// What `ag` is told for each scope. `--skip-vcs-ignores` leaves the `.gitignore` files
/// unread and nothing else - `-u` is not it, as that reads the object store under `.git`
/// too, `--ignore` or not.
fn scope_args(scope: SearchScope) -> &'static [&'static str] {
    match scope {
        SearchScope::RepoFiles => &[],
        SearchScope::IncludingIgnored => &["--skip-vcs-ignores"],
    }
}

/// Run `ag` in the repo and hand over the lines it prints as they come, a tick's worth at a
/// time. The batch is empty on a tick nothing came in, so `on_lines` is asked whether to go
/// on at a steady rate however quiet the search; `ag` is killed the moment it says stop.
fn stream(
    repo_path: &Path,
    scope: SearchScope,
    args: &[&str],
    on_lines: &mut dyn FnMut(Vec<String>) -> Flow,
) -> Result<Ended> {
    let mut child = Command::new(SEARCHER)
        .current_dir(repo_path)
        // A window opened from a launcher has a bare PATH, without the `/opt/homebrew/bin`
        // the user installed `ag` into - see [`crate::shell_path`].
        .env("PATH", crate::shell_path::installed_tools_path())
        .args(REPO_ARGS)
        .args(NEVER_SEARCHED.iter().flat_map(|name| ["--ignore", name]))
        .args(scope_args(scope))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("{SEARCHER} has to be installed to search: could not run it"))?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    // Both pipes are read on threads of their own: `ag` blocks once a pipe it writes to
    // fills up, and the ticks below must not be what drains it.
    let (lines_tx, lines_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });
    let stderr_reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut text);
        text
    });

    let mut ended = Ended::Finished;
    loop {
        let deadline = Instant::now() + TICK;
        let mut batch = Vec::new();
        let mut printed_everything = false;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match lines_rx.recv_timeout(deadline - now) {
                Ok(line) => batch.push(line),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    printed_everything = true;
                    break;
                }
            }
        }
        if let Flow::Stop = on_lines(batch) {
            ended = Ended::Stopped;
            break;
        }
        if printed_everything {
            break;
        }
    }
    if ended == Ended::Stopped {
        // Whatever it was still walking is nobody's business now.
        let _ = child.kill();
    }

    let status = child
        .wait()
        .with_context(|| format!("could not wait for {SEARCHER}"))?;
    let complaint = stderr_reader.join().unwrap_or_default();
    if ended == Ended::Stopped {
        return Ok(ended);
    }
    match status.code() {
        // 0 is "matched", 1 is "matched nothing": both are answers.
        Some(0 | 1) => Ok(ended),
        _ => bail!("{}", complaint.trim()),
    }
}

/// The characters a regex reads as something other than themselves.
const SPECIAL: &[char] = &[
    '\\', '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$',
];

/// One character, written so that the searcher looks for that very character.
fn escape_char_into(character: char, pattern: &mut String) {
    if SPECIAL.contains(&character) {
        pattern.push('\\');
    }
    pattern.push(character);
}

/// What was typed, as a regex that matches it literally: a query with a `.` or a `+` in it
/// finds the text that has that character in it rather than whatever the regex would mean.
fn escape_regex(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len());
    for character in term.chars() {
        escape_char_into(character, &mut escaped);
    }
    escaped
}

/// A repo for the searches' tests: a file of the repo, a file its `.gitignore` leaves out,
/// and a file under `node_modules`, each holding `needle`. Left behind under the temp dir,
/// the way the other test repos are.
#[cfg(test)]
fn repo_with_an_ignored_file(name: &str) -> std::path::PathBuf {
    let repo = std::env::temp_dir().join(format!(
        "moonreview-search-{name}-{}-{}",
        std::process::id(),
        crate::moontasks::store::new_uuid()
    ));
    std::fs::create_dir_all(repo.join("src")).expect("failed to make the test repo");
    std::fs::create_dir_all(repo.join("build")).expect("failed to make the test repo");
    crate::git::run_git_no_output(&repo, &["init", "-q"]).expect("git init failed");
    std::fs::write(repo.join(".gitignore"), "build/\n").expect("failed to write");
    std::fs::write(repo.join("src/kept.rs"), "needle in the repo\n").expect("failed to write");
    std::fs::write(repo.join("build/left.rs"), "needle in what it ignores\n")
        .expect("failed to write");
    std::fs::create_dir_all(repo.join("node_modules/dep")).expect("failed to make the test repo");
    std::fs::write(
        repo.join("node_modules/dep/index.rs"),
        "needle in a dependency\n",
    )
    .expect("failed to write");
    repo
}

/// A listener for the tests: keeps the last report, and wants the search for as long as
/// it was told to.
#[cfg(test)]
pub(crate) struct LastReport<T> {
    pub(crate) last: Option<SearchProgress<T>>,
    pub(crate) reports: usize,
    pub(crate) wanted: bool,
}

#[cfg(test)]
impl<T> LastReport<T> {
    pub(crate) fn wanting() -> Self {
        Self {
            last: None,
            reports: 0,
            wanted: true,
        }
    }

    pub(crate) fn done(&self) -> &SearchProgress<T> {
        let last = self.last.as_ref().expect("the search never reported");
        assert!(last.done, "the last report should say the search is done");
        last
    }
}

#[cfg(test)]
impl<T> SearchListener<T> for LastReport<T> {
    fn wanted(&mut self) -> bool {
        self.wanted
    }

    fn found(&mut self, progress: SearchProgress<T>) {
        self.reports += 1;
        self.last = Some(progress);
    }
}
