//! Who last touched each line of a file, read out of `git blame`.
//!
//! The blame is of a text the caller hands in - `git blame --contents` - or of the file as one
//! commit has it. The first is how a file tab asks about its own buffer and has the lines
//! typed into it a moment ago read as not committed, instead of as whatever the commit that
//! last touched that line number said; the second is how a tab on an old version of the file
//! asks about that version. Git answers line by line; what goes back is the lines gathered
//! into stretches, cut where the commit changes, which is the shape a column beside the lines
//! is drawn from. Each stretch also names the version of the file just before the change it
//! is put down to, which is what stepping back through the file's history opens.

use std::{
    collections::HashMap,
    io::Write,
    path::{Component, Path},
    process::Stdio,
};

use anyhow::{Context, Result, bail};

use crate::api::{BlameChunk, BlameOf, Blamed, BlamedCommit, FileVersion};

use super::{git_command, run_git_no_output};

/// The sha `git blame` gives a line no commit has.
const NOT_YET_COMMITTED_SHA: &str = "0000000000000000000000000000000000000000";

/// Who last touched each stretch of `file_path`, in the version `of` names.
///
/// A file HEAD does not have - new, or in a repo with no commits yet - has no history to
/// blame, and every line of the text handed in comes back as not committed rather than as an
/// error: that is the true answer, and it is the answer the fringe's bar beside a new line
/// already gives. A revision that does not have the file is an error, since there is no text
/// to answer about.
pub(crate) fn blame_file(
    repo_path: &Path,
    file_path: &str,
    of: &BlameOf,
) -> Result<Vec<BlameChunk>> {
    check_repo_path(file_path)?;
    let porcelain = match of {
        BlameOf::Text(text) => match run_blame_of_text(repo_path, file_path, text) {
            Ok(porcelain) => porcelain,
            Err(_) if !is_in_head(repo_path, file_path) => {
                let lines = text.lines().count();
                return Ok(match lines {
                    0 => Vec::new(),
                    lines => vec![BlameChunk {
                        lines: 0..lines,
                        blamed: Blamed::NotYetCommitted,
                        // The version before a file HEAD does not have: nothing.
                        before: None,
                        line_in_commit: 1,
                    }],
                });
            }
            Err(error) => return Err(error),
        },
        BlameOf::Revision(revision) => {
            check_revision(revision)?;
            run_blame_of_revision(repo_path, file_path, revision)?
        }
    };
    parse_porcelain(&porcelain)
}

/// A file of the repo as one commit has it.
pub(crate) fn read_file_at(repo_path: &Path, file_path: &str, revision: &str) -> Result<String> {
    check_repo_path(file_path)?;
    check_revision(revision)?;
    let spec = format!("{revision}:{file_path}");
    super::run_git(repo_path, &["show", &spec])
}

/// Refuse a path that leaves the repo.
///
/// Checked here rather than left to git, because a path git cannot see and a file HEAD does
/// not have fail the same way, and only the second is a file with no history. The text may be
/// a tab's and the file may not be on disk yet, so the path is read rather than resolved: a
/// path of the repo's own has no way out of it.
fn check_repo_path(file_path: &str) -> Result<()> {
    if file_path.trim().is_empty() {
        bail!("file path cannot be empty");
    }
    if Path::new(file_path).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("file path is outside the repository");
    }
    Ok(())
}

/// Refuse a revision that is not the name of one: what goes to git on the command line is a
/// sha or a ref, never an option.
fn check_revision(revision: &str) -> Result<()> {
    if revision.is_empty()
        || revision.starts_with('-')
        || revision.chars().any(|c| c.is_whitespace() || c == ':')
    {
        bail!("{revision:?} is not a revision");
    }
    Ok(())
}

/// `git blame --porcelain` over `text` in place of the working-tree file.
fn run_blame_of_text(repo_path: &Path, file_path: &str, text: &str) -> Result<String> {
    let mut child = git_command(repo_path)
        .args(["blame", "--porcelain", "--contents", "-", "--", file_path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git blame")?;
    {
        let mut stdin = child.stdin.take().expect("stdin was asked for");
        // Git stops reading once it has what it needs for a text it cannot blame, and the
        // pipe closing under the write is then the error already on its way from stderr.
        let _ = stdin.write_all(text.as_bytes());
    }
    let output = child
        .wait_with_output()
        .context("failed to wait for git blame")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `git blame --porcelain` of the file as `revision` has it.
fn run_blame_of_revision(repo_path: &Path, file_path: &str, revision: &str) -> Result<String> {
    super::run_git(
        repo_path,
        &["blame", "--porcelain", revision, "--", file_path],
    )
}

/// Whether HEAD has the file at all - which a repo with no commits does not, for any file.
fn is_in_head(repo_path: &Path, file_path: &str) -> bool {
    let spec = format!("HEAD:{file_path}");
    run_git_no_output(repo_path, &["cat-file", "-e", &spec]).is_ok()
}

/// What `--porcelain` says about a commit, gathered off the lines that follow the first
/// header naming it - later headers with the same sha carry none of this again.
#[derive(Default)]
struct CommitFacts {
    author: Option<String>,
    author_time: Option<i64>,
    author_tz: Option<String>,
    summary: Option<String>,
    /// The `previous` line: the commit's parent and the path the file had there. Absent on
    /// the commit that brought the file in.
    previous: Option<FileVersion>,
}

/// One line of the text as the porcelain puts it down: to which commit, and where it was in
/// that commit's version.
struct LineBlamed {
    final_line: usize,
    line_in_commit: usize,
    sha: String,
}

/// The stretches of a `--porcelain` blame, in order, adjacent lines of one commit gathered
/// into one.
///
/// Each line of the text comes as a header - the sha, the line it was in the commit, the
/// line it is now, and on the first header of a run how many lines the run has - then, the
/// first time a sha is seen, the facts about its commit as `key value` lines, and last the
/// line's own text behind a tab. The runs git cuts are kept where the commit changes and
/// joined where it does not: git also cuts a run where the lines were not adjacent in the
/// commit, which is a fact about the commit and not about the text being read.
fn parse_porcelain(porcelain: &str) -> Result<Vec<BlameChunk>> {
    let mut facts: HashMap<String, CommitFacts> = HashMap::new();
    // Every line of the text with the sha it is put down to, in order of the text.
    let mut lines_blamed: Vec<LineBlamed> = Vec::new();
    let mut lines = porcelain.lines();
    while let Some(header) = lines.next() {
        let (sha, line_in_commit, final_line) = parse_header(header)
            .with_context(|| format!("unexpected line in git blame output: {header:?}"))?;
        let known = facts.entry(sha.to_string()).or_default();
        // The facts about the commit, up to the line's own text.
        loop {
            let Some(line) = lines.next() else {
                bail!("git blame output ended inside a line's header");
            };
            if line.starts_with('\t') {
                break;
            }
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "author" => known.author = Some(value.to_string()),
                "author-time" => {
                    known.author_time = Some(
                        value
                            .parse()
                            .with_context(|| format!("unreadable author-time {value:?}"))?,
                    );
                }
                "author-tz" => known.author_tz = Some(value.to_string()),
                "summary" => known.summary = Some(value.to_string()),
                "previous" => {
                    let (sha, file_path) = value
                        .split_once(' ')
                        .with_context(|| format!("unreadable previous {value:?}"))?;
                    known.previous = Some(FileVersion {
                        sha: sha.to_string(),
                        file_path: file_path.to_string(),
                    });
                }
                _ => {}
            }
        }
        lines_blamed.push(LineBlamed {
            final_line,
            line_in_commit,
            sha: sha.to_string(),
        });
    }

    let mut chunks: Vec<BlameChunk> = Vec::new();
    for blamed_line in lines_blamed {
        // Git counts lines from one; the chunks count from zero, the way the editor does.
        let line = blamed_line
            .final_line
            .checked_sub(1)
            .context("git blame numbered a line zero")?;
        let blamed = blamed_of(&blamed_line.sha, &facts)?;
        match chunks.last_mut() {
            Some(last) if last.lines.end == line && last.blamed == blamed => last.lines.end += 1,
            _ => chunks.push(BlameChunk {
                lines: line..line + 1,
                blamed,
                before: facts
                    .get(&blamed_line.sha)
                    .and_then(|known| known.previous.clone()),
                line_in_commit: blamed_line.line_in_commit,
            }),
        }
    }
    Ok(chunks)
}

/// The sha, the line in the commit and the line number in the text off a header line, or
/// nothing for a line that is not one.
fn parse_header(line: &str) -> Option<(&str, usize, usize)> {
    let mut fields = line.split(' ');
    let sha = fields.next()?;
    if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let line_in_commit = fields.next()?.parse().ok()?;
    let final_line = fields.next()?.parse().ok()?;
    Some((sha, line_in_commit, final_line))
}

/// What a sha and the facts gathered about it come to.
fn blamed_of(sha: &str, facts: &HashMap<String, CommitFacts>) -> Result<Blamed> {
    if sha == NOT_YET_COMMITTED_SHA {
        return Ok(Blamed::NotYetCommitted);
    }
    let known = facts
        .get(sha)
        .with_context(|| format!("git blame named {sha} without saying anything about it"))?;
    let author_time = known
        .author_time
        .with_context(|| format!("git blame gave no author-time for {sha}"))?;
    let author_tz = known
        .author_tz
        .as_deref()
        .with_context(|| format!("git blame gave no author-tz for {sha}"))?;
    Ok(Blamed::Committed(BlamedCommit {
        sha: sha.to_string(),
        author: known
            .author
            .clone()
            .with_context(|| format!("git blame gave no author for {sha}"))?,
        authored_on: day_of(author_time, author_tz)?,
        summary: known
            .summary
            .clone()
            .with_context(|| format!("git blame gave no summary for {sha}"))?,
    }))
}

/// The calendar day of a unix time as seen from a zone written the way git writes one -
/// `+0200`, `-0530` - as `YYYY-MM-DD`.
fn day_of(unix: i64, tz: &str) -> Result<String> {
    let offset_seconds = tz_offset_seconds(tz)
        .with_context(|| format!("unreadable time zone {tz:?} in git blame output"))?;
    let local = unix + offset_seconds;
    let days = local.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

/// `+hhmm` or `-hhmm` as seconds east of UTC.
fn tz_offset_seconds(tz: &str) -> Option<i64> {
    let (sign, digits) = match tz.chars().next()? {
        '+' => (1, &tz[1..]),
        '-' => (-1, &tz[1..]),
        _ => return None,
    };
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let hours: i64 = digits[..2].parse().ok()?;
    let minutes: i64 = digits[2..].parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

/// The proleptic Gregorian date of a day counted from 1970-01-01 - Howard Hinnant's
/// `civil_from_days`, which needs no table of month lengths.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, process::Command};

    use super::*;

    /// Two lines of one commit, then one line of another, then two more of the first again
    /// - which git reports as three runs, since the first commit's lines were not adjacent
    /// in the commit.
    const PORCELAIN: &str = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1 1 2
author Ada Lovelace
author-mail <ada@example.com>
author-time 1704164645
author-tz +0100
committer Ada Lovelace
committer-mail <ada@example.com>
committer-time 1704164645
committer-tz +0100
summary Add the library
filename src/lib.rs
\tfn one() {}
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 2 2
\tfn two() {}
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 3 3 1
author Grace Hopper
author-mail <grace@example.com>
author-time 1717200000
author-tz -0500
committer Grace Hopper
committer-mail <grace@example.com>
committer-time 1717200000
committer-tz -0500
summary Add three
previous aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa src/old_lib.rs
filename src/lib.rs
\tfn three() {}
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 3 4 2
\tfn four() {}
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 4 5
\tfn five() {}
";

    fn ada() -> Blamed {
        Blamed::Committed(BlamedCommit {
            sha: "a".repeat(40),
            author: "Ada Lovelace".to_string(),
            authored_on: "2024-01-02".to_string(),
            summary: "Add the library".to_string(),
        })
    }

    #[test]
    fn porcelain_lines_are_gathered_into_stretches_cut_where_the_commit_changes() {
        let chunks = parse_porcelain(PORCELAIN).expect("the porcelain is well formed");
        assert_eq!(
            chunks,
            vec![
                BlameChunk {
                    lines: 0..2,
                    blamed: ada(),
                    // The commit that brought the file in has nothing before it.
                    before: None,
                    line_in_commit: 1,
                },
                BlameChunk {
                    lines: 2..3,
                    blamed: Blamed::Committed(BlamedCommit {
                        sha: "b".repeat(40),
                        author: "Grace Hopper".to_string(),
                        // 2024-06-01T00:00Z, which is still the 31st of May in New York.
                        authored_on: "2024-05-31".to_string(),
                        summary: "Add three".to_string(),
                    }),
                    // Its parent, at the path the file had there.
                    before: Some(FileVersion {
                        sha: "a".repeat(40),
                        file_path: "src/old_lib.rs".to_string(),
                    }),
                    line_in_commit: 3,
                },
                BlameChunk {
                    lines: 3..5,
                    blamed: ada(),
                    before: None,
                    // Line 3 of Ada's version, now line 4.
                    line_in_commit: 3,
                },
            ]
        );
    }

    #[test]
    fn a_line_no_commit_has_is_not_yet_committed_with_head_before_it() {
        let porcelain = "\
0000000000000000000000000000000000000000 1 1 1
author Not Committed Yet
author-mail <not.committed.yet>
author-time 1717200000
author-tz +0000
committer Not Committed Yet
committer-mail <not.committed.yet>
committer-time 1717200000
committer-tz +0000
summary Version of src/lib.rs from src/lib.rs
previous aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa src/lib.rs
filename src/lib.rs
\tfn typed() {}
";
        assert_eq!(
            parse_porcelain(porcelain).expect("well formed"),
            vec![BlameChunk {
                lines: 0..1,
                blamed: Blamed::NotYetCommitted,
                before: Some(FileVersion {
                    sha: "a".repeat(40),
                    file_path: "src/lib.rs".to_string(),
                }),
                line_in_commit: 1,
            }]
        );
        assert!(
            parse_porcelain("")
                .expect("nothing is well formed")
                .is_empty()
        );
    }

    #[test]
    fn output_that_is_not_a_blame_is_refused() {
        assert!(parse_porcelain("fatal: something\n").is_err());
        assert!(parse_porcelain(&format!("{} 1 1 1\nauthor Ada\n", "a".repeat(40))).is_err());
    }

    #[test]
    fn the_day_is_the_authors_own() {
        // 2024-01-01T23:30Z: already the 2nd in Paris, still the 1st in UTC and New York.
        let unix = 1_704_151_800;
        assert_eq!(day_of(unix, "+0100").expect("readable"), "2024-01-02");
        assert_eq!(day_of(unix, "+0000").expect("readable"), "2024-01-01");
        assert_eq!(day_of(unix, "-0500").expect("readable"), "2024-01-01");
        // Half-hour zones and the epoch itself.
        assert_eq!(day_of(0, "+0530").expect("readable"), "1970-01-01");
        assert_eq!(day_of(-1, "+0000").expect("readable"), "1969-12-31");
        // A leap day.
        assert_eq!(
            day_of(1_709_164_800, "+0000").expect("readable"),
            "2024-02-29"
        );
        assert!(day_of(0, "0100").is_err());
        assert!(day_of(0, "+1").is_err());
    }

    #[test]
    fn only_the_name_of_a_revision_goes_to_git() {
        assert!(check_revision("HEAD").is_ok());
        assert!(check_revision(&"a".repeat(40)).is_ok());
        assert!(check_revision("main~2").is_ok());
        assert!(check_revision("").is_err());
        assert!(check_revision("--output=/tmp/x").is_err());
        assert!(check_revision("HEAD:src/lib.rs").is_err());
        assert!(check_revision("two words").is_err());
    }

    /// A throwaway repo for the tests that run git itself.
    struct TestRepo {
        path: PathBuf,
    }

    impl TestRepo {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("moonreview-blame-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("failed to create the test repo directory");
            run_git_no_output(&path, &["init"]).expect("failed to init");
            for (key, value) in [
                ("user.email", "test@example.com"),
                ("user.name", "Test User"),
                ("commit.gpgsign", "false"),
            ] {
                run_git_no_output(&path, &["config", key, value]).expect("failed to configure");
            }
            Self { path }
        }

        fn commit(&self, message: &str) -> String {
            run_git_no_output(&self.path, &["add", "-A"]).expect("failed to add");
            let status = Command::new("git")
                .args(["commit", "-m", message])
                .current_dir(&self.path)
                .env("GIT_AUTHOR_DATE", "2024-01-02T03:04:05+00:00")
                .env("GIT_COMMITTER_DATE", "2024-01-02T03:04:05+00:00")
                .output()
                .expect("failed to run git commit");
            assert!(status.status.success(), "failed to commit");
            super::super::run_git(&self.path, &["rev-parse", "HEAD"])
                .expect("failed to read HEAD")
                .trim()
                .to_string()
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn of_text(text: &str) -> BlameOf {
        BlameOf::Text(text.to_string())
    }

    #[test]
    fn a_blame_is_of_the_text_handed_in_with_typed_lines_not_committed() {
        let repo = TestRepo::new("typed");
        fs::write(repo.path.join("lib.rs"), "fn one() {}\nfn two() {}\n").expect("write");
        let head = repo.commit("Add the library");

        // A line typed between the two committed ones, and never saved.
        let chunks = blame_file(
            &repo.path,
            "lib.rs",
            &of_text("fn one() {}\nfn typed() {}\nfn two() {}\n"),
        )
        .expect("blame runs");
        assert_eq!(chunks.len(), 3, "{chunks:?}");
        assert_eq!(chunks[0].lines, 0..1);
        assert_eq!(chunks[1].lines, 1..2);
        assert_eq!(chunks[1].blamed, Blamed::NotYetCommitted);
        // Before the typing: the file as HEAD has it.
        assert_eq!(
            chunks[1].before,
            Some(FileVersion {
                sha: head.clone(),
                file_path: "lib.rs".to_string(),
            })
        );
        assert_eq!(chunks[2].lines, 2..3);
        let Blamed::Committed(commit) = &chunks[0].blamed else {
            panic!("the first line was committed: {chunks:?}");
        };
        assert_eq!(commit.author, "Test User");
        assert_eq!(commit.authored_on, "2024-01-02");
        assert_eq!(commit.summary, "Add the library");
        assert_eq!(commit.sha, head);
        assert_eq!(
            chunks[0].before, None,
            "the first commit has nothing before it"
        );
        assert_eq!(chunks[2].blamed, chunks[0].blamed);
        // `two` was line 2 of the commit, and is line 3 now.
        assert_eq!(chunks[2].line_in_commit, 2);
    }

    /// The blame of an old version has no uncommitted lines in it, and its stretches name
    /// the versions before them - which is how stepping back through the history goes.
    #[test]
    fn a_blame_of_a_revision_steps_back_through_the_versions_before() {
        let repo = TestRepo::new("revision");
        fs::write(repo.path.join("lib.rs"), "fn one() {}\n").expect("write");
        let first = repo.commit("Add the library");
        fs::write(repo.path.join("lib.rs"), "fn one() {}\nfn two() {}\n").expect("write");
        let second = repo.commit("Add two");
        fs::write(
            repo.path.join("lib.rs"),
            "fn one() {}\nfn two() {}\nfn three() {}\n",
        )
        .expect("write");
        let third = repo.commit("Add three");
        // Typing since, which a blame of a revision knows nothing of.
        fs::write(repo.path.join("lib.rs"), "typed\n").expect("write");

        let chunks =
            blame_file(&repo.path, "lib.rs", &BlameOf::Revision(third.clone())).expect("blames");
        assert_eq!(chunks.len(), 3, "{chunks:?}");
        let sha_of = |chunk: &BlameChunk| match &chunk.blamed {
            Blamed::Committed(commit) => commit.sha.clone(),
            Blamed::NotYetCommitted => panic!("a revision has no uncommitted lines"),
        };
        assert_eq!(sha_of(&chunks[0]), first);
        assert_eq!(sha_of(&chunks[1]), second);
        assert_eq!(sha_of(&chunks[2]), third);
        assert_eq!(
            chunks[2].before.as_ref().map(|before| &before.sha),
            Some(&second)
        );
        assert_eq!(
            chunks[1].before.as_ref().map(|before| &before.sha),
            Some(&first)
        );
        assert_eq!(chunks[0].before, None);

        // And the file as the second commit has it, to open that step on.
        assert_eq!(
            read_file_at(&repo.path, "lib.rs", &second).expect("reads"),
            "fn one() {}\nfn two() {}\n"
        );
        assert!(read_file_at(&repo.path, "nowhere.rs", &second).is_err());
        assert!(blame_file(&repo.path, "nowhere.rs", &BlameOf::Revision(second)).is_err());
    }

    #[test]
    fn a_file_head_does_not_have_is_not_committed_from_top_to_bottom() {
        let repo = TestRepo::new("untracked");
        fs::write(repo.path.join("lib.rs"), "fn one() {}\n").expect("write");
        repo.commit("Add the library");

        // Not in HEAD: a file only just created beside the committed one.
        let chunks =
            blame_file(&repo.path, "new.rs", &of_text("a\nb\n")).expect("a new file blames");
        assert_eq!(
            chunks,
            vec![BlameChunk {
                lines: 0..2,
                blamed: Blamed::NotYetCommitted,
                before: None,
                line_in_commit: 1,
            }]
        );
        assert!(
            blame_file(&repo.path, "new.rs", &of_text(""))
                .expect("empty")
                .is_empty()
        );

        // A repo with no commits at all has no HEAD to have anything.
        let empty = TestRepo::new("no-commits");
        let chunks = blame_file(&empty.path, "lib.rs", &of_text("one\n")).expect("blames");
        assert_eq!(chunks[0].blamed, Blamed::NotYetCommitted);
    }

    #[test]
    fn a_blame_outside_the_repository_is_refused() {
        let repo = TestRepo::new("outside");
        fs::write(repo.path.join("lib.rs"), "fn one() {}\n").expect("write");
        repo.commit("Add the library");
        assert!(blame_file(&repo.path, "", &of_text("x\n")).is_err());
        assert!(blame_file(&repo.path, "../elsewhere.rs", &of_text("x\n")).is_err());
        assert!(read_file_at(&repo.path, "../elsewhere.rs", "HEAD").is_err());
    }
}
