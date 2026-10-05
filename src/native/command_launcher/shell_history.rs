//! The login shell's own history, read from the file the shell keeps it in: what makes the
//! arrows and ctrl+r worth pressing in a line that has never started anything.
//!
//! Read and never written. What the line starts is written to the line's own file - see
//! [`super::history`] - and a shell's history file is the shell's.
//!
//! A command spread over several lines is left out: the line is one line, and has no way to
//! hold it.

use std::path::PathBuf;

use anyhow::{Context as _, Result};

/// How one shell keeps its history.
struct Kept {
    /// The shell's program, which is the last part of `$SHELL`.
    shell: &'static str,
    /// The file it writes when nothing tells it otherwise, from the home directory.
    #[cfg_attr(
        test,
        expect(dead_code, reason = "a test reads a scratch file instead")
    )]
    file: &'static str,
    /// The one-line commands in the bytes of such a file, oldest first.
    commands: fn(&[u8]) -> Vec<String>,
}

/// The shells whose history is read. Another shell's is not: the line has its own history
/// and nothing older under it.
const KEPT: &[Kept] = &[
    Kept {
        shell: "zsh",
        file: ".zsh_history",
        commands: zsh_commands,
    },
    Kept {
        shell: "bash",
        file: ".bash_history",
        commands: bash_commands,
    },
];

/// The login shell's program. Under test it is zsh whoever runs the tests, so what a test
/// seeds is read the same way on every machine.
fn login_shell_program() -> String {
    #[cfg(test)]
    {
        "zsh".to_string()
    }
    #[cfg(not(test))]
    {
        let shell = crate::shell_path::login_shell();
        std::path::Path::new(&shell)
            .file_name()
            .unwrap_or_else(|| panic!("{shell} names no shell program"))
            .to_string_lossy()
            .into_owned()
    }
}

/// Where the shell's history is: `$HISTFILE` when the window was handed one, and otherwise
/// where that shell keeps it by default. `None` for an account with no home directory.
///
/// Under test it is a scratch file of the test's own, beside the settings' - see
/// [`crate::settings::path`] - so a test never reads what the person running it has typed.
fn path(kept: &Kept) -> Option<PathBuf> {
    #[cfg(test)]
    {
        let _ = kept;
        Some(crate::settings::path()?.with_extension("shell_history"))
    }
    #[cfg(not(test))]
    {
        if let Some(named) = std::env::var_os("HISTFILE").filter(|named| !named.is_empty()) {
            return Some(PathBuf::from(named));
        }
        let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
        Some(PathBuf::from(home).join(kept.file))
    }
}

/// The login shell's one-line commands, oldest first. Nothing for a shell that is not one of
/// [`KEPT`], and nothing for one that has written no history yet.
pub(super) fn read() -> Result<Vec<String>> {
    let program = login_shell_program();
    let Some(kept) = KEPT.iter().find(|kept| kept.shell == program) else {
        return Ok(Vec::new());
    };
    let Some(path) = path(kept) else {
        return Ok(Vec::new());
    };
    match std::fs::read(&path) {
        Ok(bytes) => Ok((kept.commands)(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

/// Put a history file where [`read`] looks, for a test of what the line does with one.
#[cfg(test)]
pub(super) fn write_for_test(contents: &str) {
    let path = path(&KEPT[0]).expect("expected a scratch path");
    std::fs::write(&path, contents).expect("expected the history written");
}

/// The byte zsh writes before any byte it cannot keep as it is, which it then keeps with
/// [`ZSH_META_FLIP`] flipped - every byte of a letter outside ASCII among them.
const ZSH_META: u8 = 0x83;
const ZSH_META_FLIP: u8 = 0x20;

/// The bytes of a zsh history file as the text that was typed.
fn zsh_text(bytes: &[u8]) -> String {
    let mut typed = Vec::with_capacity(bytes.len());
    let mut bytes = bytes.iter();
    while let Some(byte) = bytes.next() {
        match (*byte, bytes.as_slice().first()) {
            (ZSH_META, Some(kept)) => {
                typed.push(kept ^ ZSH_META_FLIP);
                bytes.next();
            }
            _ => typed.push(*byte),
        }
    }
    String::from_utf8_lossy(&typed).into_owned()
}

/// The command on a line of zsh's history, which with `EXTENDED_HISTORY` set has when it was
/// run and for how long written before it: `: 1700000000:0;cargo build`.
fn zsh_command(line: &str) -> &str {
    let Some((stamp, command)) = line
        .strip_prefix(": ")
        .and_then(|stamped| stamped.split_once(';'))
    else {
        return line;
    };
    let is_a_stamp = stamp.split_once(':').is_some_and(|(began, took)| {
        !began.is_empty()
            && began.bytes().all(|byte| byte.is_ascii_digit())
            && took.bytes().all(|byte| byte.is_ascii_digit())
    });
    if is_a_stamp { command } else { line }
}

/// zsh's history: a command to a line, and a command of several lines with a backslash at
/// the end of every line but its last.
fn zsh_commands(bytes: &[u8]) -> Vec<String> {
    let text = zsh_text(bytes);
    let mut commands = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.ends_with('\\') {
            // The rest of a command of several lines, which is left out whole.
            for continued in lines.by_ref() {
                if !continued.ends_with('\\') {
                    break;
                }
            }
            continue;
        }
        let command = zsh_command(line);
        if !command.is_empty() {
            commands.push(command.to_string());
        }
    }
    commands
}

/// bash's history: a command to a line, and with `HISTTIMEFORMAT` set a line before each
/// saying when it was run, `#1700000000`.
fn bash_commands(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let is_a_stamp = line.strip_prefix('#').is_some_and(|stamp| {
                !stamp.is_empty() && stamp.bytes().all(|byte| byte.is_ascii_digit())
            });
            !is_a_stamp
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_history_is_read_with_or_without_its_stamps() {
        let commands = zsh_commands(b"cargo build\n: 1700000000:0;make deploy\n: > emptied\n\n");

        assert_eq!(commands, ["cargo build", "make deploy", ": > emptied"]);
    }

    #[test]
    fn a_zsh_command_of_several_lines_is_left_out() {
        let commands = zsh_commands(b"ls\n: 1700000000:0;for f in *; do\\\necho $f\\\ndone\npwd\n");

        assert_eq!(commands, ["ls", "pwd"]);
    }

    /// `é` is the bytes c3 a9, and zsh keeps each of them as its marker and the byte flipped.
    #[test]
    fn zsh_history_reads_letters_outside_ascii() {
        let commands = zsh_commands(b"echo caf\x83\xe3\x83\x89\n");

        assert_eq!(commands, ["echo café"]);
    }

    #[test]
    fn bash_history_is_read_without_its_stamps() {
        let commands = bash_commands(
            b"#1700000000\ncargo build\n#1700000050\nmake deploy\n# a comment typed\n",
        );

        assert_eq!(
            commands,
            ["cargo build", "make deploy", "# a comment typed"]
        );
    }
}
