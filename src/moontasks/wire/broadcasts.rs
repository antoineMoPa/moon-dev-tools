//! The wire's broadcasts: the lines posted to every agent of a board, kept in
//! `.moontasks/messageboard.txt` - see [`crate::moontasks::WIRE_FILE_NAME`].
//!
//! One line a post, dated in this machine's time and signed with the handle of the task it
//! was posted from:
//!
//! ```text
//! 2026-10-06 09:14 @fix-the-races: rewriting src/terminal.rs and its tests
//! ```
//!
//! The file is the latest [`KEPT_LINES`] of them, oldest first. [`post`] is its only writer,
//! which is what keeps it in that order, at that length and in that shape - so it holds the
//! file to the shape as well: a line of any other, which is a line somebody wrote by hand,
//! stops the post rather than being dropped or rewritten.

use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::moontasks::{WIRE_FILE_NAME, store};

/// How many lines the file is cut back to as one is posted. An agent reads the whole of it
/// before it starts, so it is kept to what is worth reading: who is in what right now.
pub(crate) const KEPT_LINES: usize = 30;

/// How long the moment a line starts with is: `2026-10-06 09:14`.
const STAMP_LENGTH: usize = 16;

/// Where each character of the stamp that is not a digit sits, and what it is.
const STAMP_PUNCTUATION: [(usize, u8); 4] = [(4, b'-'), (7, b'-'), (10, b' '), (13, b':')];

/// The file the broadcasts of this repo's board are kept in.
pub(crate) fn file_path(repo_path: &Path) -> PathBuf {
    store::tasks_root(repo_path).join(WIRE_FILE_NAME)
}

/// Post a line to every agent of the board, signed with the handle of the task posting it.
pub(crate) fn post(repo_path: &Path, handle: &str, message: &str) -> Result<()> {
    post_at(repo_path, handle, message, store::now_unix())
}

/// [`post`], at a moment the caller names.
///
/// The file is locked from before it is read until it has been written back, so two agents
/// posting at once each read what the other wrote rather than both writing over one reading.
fn post_at(repo_path: &Path, handle: &str, message: &str, at_unix: u64) -> Result<()> {
    let text = super::one_line(message)?;
    let line = format!("{} @{handle}: {text}", stamp_of(at_unix));
    // Nothing is written that the next post would refuse to read back.
    if !is_a_line(&line) {
        bail!("@{handle} cannot sign a line of the wire: a handle is one word");
    }

    let path = file_path(repo_path);
    // Opened without truncating: the lock is taken on this file, and what is in it is read
    // under the lock. A board nobody has posted to may have no file yet, which is an empty one.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    lock_exclusively(&file).with_context(|| format!("failed to lock {}", path.display()))?;

    let mut kept = String::new();
    file.read_to_string(&mut kept)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut lines = lines_of(&kept, &path)?;
    lines.push(&line);
    let latest = &lines[lines.len().saturating_sub(KEPT_LINES)..];
    let written: String = latest.iter().map(|line| format!("{line}\n")).collect();

    // Written back through the handle the lock is on, so the lock covers the write. The file
    // is released as the handle is dropped.
    file.set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)))
        .and_then(|_| file.write_all(written.as_bytes()))
        .with_context(|| format!("failed to write {}", path.display()))
}

/// Wait for the file to be nobody else's and take it, until the handle is closed.
fn lock_exclusively(file: &File) -> std::io::Result<()> {
    loop {
        // SAFETY: `flock` reads nothing but the descriptor, which `file` keeps open.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        // A signal arriving during the wait is not the lock being refused.
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// The lines the file holds, or the first thing about it that is not as [`post`] writes it:
/// each line in the wire's shape, and each ended by a line break.
fn lines_of<'kept>(kept: &'kept str, path: &Path) -> Result<Vec<&'kept str>> {
    let mut lines = Vec::new();
    for (index, written) in kept.split_inclusive('\n').enumerate() {
        let number = index + 1;
        let Some(line) = written.strip_suffix('\n') else {
            bail!(
                "{} line {number} does not end in a line break, so it was written by hand: \
                 `{}` is the only writer of that file. Put the line right or take it out",
                path.display(),
                super::post_command()
            );
        };
        if !is_a_line(line) {
            bail!(
                "{} line {number} is not a line of the wire, so it was written by hand: \
                 `{}` is the only writer of that file. Put the line right or take it out",
                path.display(),
                super::post_command()
            );
        }
        lines.push(line);
    }
    Ok(lines)
}

/// Whether this is a line as [`post`] writes one: `2026-10-06 09:14 @handle: text`.
fn is_a_line(line: &str) -> bool {
    let Some((stamp, signed)) = line.split_at_checked(STAMP_LENGTH) else {
        return false;
    };
    let Some((handle, text)) = signed
        .strip_prefix(" @")
        .and_then(|signed| signed.split_once(": "))
    else {
        return false;
    };
    is_a_stamp(stamp)
        && !handle.is_empty()
        && !handle.contains(char::is_whitespace)
        && super::one_line(text).is_ok()
}

fn is_a_stamp(stamp: &str) -> bool {
    stamp.len() == STAMP_LENGTH
        && stamp.bytes().enumerate().all(|(at, byte)| {
            match STAMP_PUNCTUATION.iter().find(|(place, _)| *place == at) {
                Some((_, punctuation)) => byte == *punctuation,
                None => byte.is_ascii_digit(),
            }
        })
}

/// A moment as a line is dated: `2026-10-06 09:14`, in this machine's zone. `localtime_r`
/// rather than a date crate, since the zone and its daylight-saving rules are the OS's to know.
fn stamp_of(unix: u64) -> String {
    let time = unix as libc::time_t;
    // SAFETY: `libc::tm` is plain data - integers, and on macOS a pointer to a zone name that
    // `localtime_r` sets or leaves null - so zeroes are a valid value of it.
    let mut placed: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to locals that outlive the call, and `localtime_r` writes to
    // no other memory.
    let answered = unsafe { libc::localtime_r(&time, &mut placed) };
    assert!(
        !answered.is_null(),
        "localtime_r could not place {unix} in this machine's zone"
    );
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        placed.tm_year + 1900,
        placed.tm_mon + 1,
        placed.tm_mday,
        placed.tm_hour,
        placed.tm_min
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// 2025-09-16, at some hour of this machine's day.
    const A_MOMENT: u64 = 1_758_000_000;

    /// A repo with a board in it and nothing on the board.
    fn temp_repo(name: &str) -> PathBuf {
        let repo = std::env::temp_dir().join(format!(
            "moonreview-wire-{}-{name}-{}",
            std::process::id(),
            store::new_uuid()
        ));
        fs::create_dir_all(store::tasks_root(&repo)).expect("failed to create the board");
        repo
    }

    fn kept(repo: &Path) -> String {
        fs::read_to_string(file_path(repo)).expect("failed to read the wire's file")
    }

    /// `localtime_r` against `date`, which reads the same zone.
    #[test]
    fn a_line_is_dated_to_the_minute_in_this_machines_time() {
        // BSD `date` reads a moment as `-r`, GNU `date` as `-d @`.
        let moment = if cfg!(target_os = "macos") {
            vec!["-r".to_string(), A_MOMENT.to_string()]
        } else {
            vec!["-d".to_string(), format!("@{A_MOMENT}")]
        };
        let output = std::process::Command::new("date")
            .args(moment)
            .arg("+%Y-%m-%d %H:%M")
            .output()
            .expect("expected `date` to run");
        let expected = String::from_utf8(output.stdout).expect("expected a date");

        assert_eq!(stamp_of(A_MOMENT), expected.trim());
        assert!(is_a_stamp(&stamp_of(A_MOMENT)));
    }

    #[test]
    fn a_post_is_a_dated_line_signed_with_the_handle() {
        let repo = temp_repo("signed");

        post_at(
            &repo,
            "fix-the-races",
            "rewriting src/terminal.rs",
            A_MOMENT,
        )
        .expect("expected the first post");
        post_at(
            &repo,
            "bing-bong-313",
            "in src/cli: all of it",
            A_MOMENT + 60,
        )
        .expect("expected the second post");

        assert_eq!(
            kept(&repo),
            format!(
                "{} @fix-the-races: rewriting src/terminal.rs\n{} @bing-bong-313: in src/cli: \
                 all of it\n",
                stamp_of(A_MOMENT),
                stamp_of(A_MOMENT + 60)
            )
        );
    }

    /// The file is made empty as an agent starts, and that is a file with no lines in it.
    #[test]
    fn the_first_post_goes_into_the_empty_file_an_agent_was_started_with() {
        let repo = temp_repo("empty");
        store::ensure_wire_file(&repo).expect("expected the file to be made");
        assert_eq!(kept(&repo), "");

        post_at(&repo, "fix-the-races", "starting", A_MOMENT).expect("expected the post");

        assert_eq!(
            kept(&repo),
            format!("{} @fix-the-races: starting\n", stamp_of(A_MOMENT))
        );
    }

    #[test]
    fn the_file_is_cut_back_to_its_latest_lines() {
        let repo = temp_repo("cut");

        for number in 0..KEPT_LINES + 5 {
            post_at(&repo, "fix-the-races", &format!("post {number}"), A_MOMENT)
                .expect("expected the post");
        }

        let kept = kept(&repo);
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines.len(), KEPT_LINES);
        assert!(lines[0].ends_with(": post 5"), "got {:?}", lines[0]);
        assert!(
            lines[KEPT_LINES - 1].ends_with(&format!(": post {}", KEPT_LINES + 4)),
            "got {:?}",
            lines[KEPT_LINES - 1]
        );
    }

    #[test]
    fn a_line_break_and_an_empty_line_are_refused_and_nothing_is_written() {
        let repo = temp_repo("refused");
        post_at(&repo, "fix-the-races", "starting", A_MOMENT).expect("expected the post");
        let before = kept(&repo);

        for refused in ["", "  ", "one\ntwo", "one\r\n"] {
            let error = post_at(&repo, "fix-the-races", refused, A_MOMENT)
                .expect_err("expected the post to be refused");
            assert!(error.to_string().contains("the wire carries"), "{error}");
        }

        assert_eq!(kept(&repo), before);
    }

    /// A line somebody wrote into the file by hand stops the post, and stays as it is.
    #[test]
    fn a_line_written_by_hand_stops_the_post_and_is_named_by_its_number() {
        let repo = temp_repo("by-hand");
        post_at(&repo, "fix-the-races", "starting", A_MOMENT).expect("expected the post");
        let by_hand = format!("{}note to self: do not touch main\n", kept(&repo));
        fs::write(file_path(&repo), &by_hand).expect("failed to write the file by hand");

        let error = post_at(&repo, "bing-bong", "in src/cli", A_MOMENT)
            .expect_err("expected the post to be refused");

        assert!(error.to_string().contains("line 2"), "{error}");
        assert_eq!(kept(&repo), by_hand, "the file is left as it was found");
    }

    #[test]
    fn a_last_line_with_no_line_break_stops_the_post() {
        let repo = temp_repo("unended");
        let unended = format!("{} @fix-the-races: starting", stamp_of(A_MOMENT));
        fs::write(file_path(&repo), &unended).expect("failed to write the file by hand");

        let error = post_at(&repo, "bing-bong", "in src/cli", A_MOMENT)
            .expect_err("expected the post to be refused");

        assert!(error.to_string().contains("line 1"), "{error}");
        assert_eq!(kept(&repo), unended);
    }

    #[test]
    fn only_a_line_in_the_wires_shape_is_one() {
        assert!(is_a_line("2026-10-06 09:14 @fix-the-races: in src/cli"));
        assert!(is_a_line("2026-10-06 09:14 @board-task: a: b"));
        for not_one in [
            "",
            "in src/cli",
            "2026-10-06 09:14",
            "2026-10-06 09:14 @fix-the-races:",
            "2026-10-06 09:14 @fix-the-races: ",
            "2026-10-06 09:14 fix-the-races: in src/cli",
            "2026-10-06 09:14 @: in src/cli",
            "2026-10-06 09:14 @fix the races: in src/cli",
            "2026/10/06 09:14 @fix-the-races: in src/cli",
            "2026-10-06  9:14 @fix-the-races: in src/cli",
            "2026-10-06 09:14 @fix-the-races: in src/cli\r",
            "ünïcode stamp!!! @fix-the-races: in src/cli",
        ] {
            assert!(!is_a_line(not_one), "{not_one:?} should not be a line");
        }
    }

    /// A handle that is not one word - a folder named by hand, with a space in it - would sign
    /// a line the next post refuses, so it signs none.
    #[test]
    fn a_handle_that_is_not_one_word_signs_nothing() {
        let repo = temp_repo("two-words");

        let error = post_at(&repo, "my task", "starting", A_MOMENT)
            .expect_err("expected the post to be refused");

        assert!(error.to_string().contains("@my task"), "{error}");
        assert!(!file_path(&repo).exists());
    }

    /// Each poster reads the file under the lock, so none of them writes over a reading that
    /// another has since added to.
    #[test]
    fn posts_made_at_the_same_moment_all_land() {
        let repo = temp_repo("at-once");
        let posters: Vec<_> = (0..KEPT_LINES)
            .map(|number| {
                let repo = repo.clone();
                std::thread::spawn(move || {
                    post_at(&repo, &format!("agent-{number}"), "starting", A_MOMENT)
                })
            })
            .collect();
        for poster in posters {
            poster
                .join()
                .expect("expected the poster to finish")
                .expect("expected the post");
        }

        let kept = kept(&repo);
        assert_eq!(kept.lines().count(), KEPT_LINES);
        for number in 0..KEPT_LINES {
            assert!(
                kept.contains(&format!(" @agent-{number}: starting\n")),
                "agent-{number} is missing from {kept}"
            );
        }
    }
}
