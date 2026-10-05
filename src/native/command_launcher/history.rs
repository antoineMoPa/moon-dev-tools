//! What the arrows walk back through and ctrl+r searches: the commands started from the strip
//! before, and under them the login shell's own history.
//!
//! The way a shell has it: what this session ran comes back first, and what is in the history
//! file after it. So the line is worth an arrow up the first time it is ever opened, and what
//! it started itself is still the nearest thing.
//!
//! What the strip started is kept in `~/.moonreview/launched_commands`, a command to a line
//! with the oldest first - the shape of a shell's own history file, and for the same reason: it
//! can be read, and it can be pruned by hand. The shell's file is only ever read - see
//! [`super::shell_history`].
//!
//! No command is in the history twice. One started again moves to the newest end, so the list
//! is what was started in the order it was last wanted, and the arrows do not walk the same
//! line over and over.

/// How many commands are kept. The oldest go first.
#[cfg(not(target_arch = "wasm32"))]
const COMMANDS_KEPT: usize = 1000;

#[cfg(not(any(test, target_arch = "wasm32")))]
const FILE_NAME: &str = "launched_commands";

/// The commands that can be brought back, oldest first, each of them once.
#[derive(Default)]
pub(crate) struct History {
    commands: Vec<String>,
}

impl History {
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    /// The command at a place in the history, counted from the oldest.
    pub(crate) fn command(&self, at: usize) -> &str {
        &self.commands[at]
    }

    /// The newest command older than `before` that has `wanted` somewhere in it, which is the
    /// question ctrl+r asks. Nothing has an empty string "in it": a search for nothing finds
    /// nothing, rather than whatever happened to be started last.
    pub(crate) fn newest_containing(&self, wanted: &str, before: usize) -> Option<usize> {
        if wanted.is_empty() {
            return None;
        }
        self.commands[..before]
            .iter()
            .rposition(|command| command.contains(wanted))
    }

    /// The history with one more command started: at the newest end, and nowhere else.
    #[cfg(not(target_arch = "wasm32"))]
    fn with_started(mut self, command: &str) -> Self {
        assert!(
            !command.contains('\n'),
            "a command is one line of the history file, got {command:?}"
        );
        self.commands.retain(|started| started != command);
        self.commands.push(command.to_string());
        let over = self.commands.len().saturating_sub(COMMANDS_KEPT);
        self.commands.drain(..over);
        self
    }

    /// This history with `older` commands under it, each command once and where it was last
    /// run: one in both is the one here.
    #[cfg(not(target_arch = "wasm32"))]
    fn over(self, older: Vec<String>) -> Self {
        let mut seen = std::collections::HashSet::new();
        let mut commands: Vec<String> = older
            .into_iter()
            .chain(self.commands)
            .rev()
            .filter(|command| seen.insert(command.clone()))
            .collect();
        commands.reverse();
        Self { commands }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn parsed(text: &str) -> Self {
        Self {
            commands: text
                .lines()
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod on_disk {
    use std::path::{Path, PathBuf};

    use anyhow::{Context as _, Result};

    use super::History;

    /// Where the history is kept: beside the settings. Under test that is a scratch file of
    /// the test's own - see [`crate::settings::path`] - so this is one too, and a test neither
    /// reads what the person running it has started nor adds to it.
    fn path() -> Result<PathBuf> {
        let settings = crate::settings::path().context("no home directory to keep history in")?;
        #[cfg(test)]
        {
            Ok(settings.with_extension("launched_commands"))
        }
        #[cfg(not(test))]
        {
            Ok(settings.with_file_name(super::FILE_NAME))
        }
    }

    /// No file yet is the ordinary case before anything has been started.
    fn read_at(path: &Path) -> Result<History> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(History::parsed(&text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(History::default()),
            Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    impl History {
        /// What the strip started, over what the login shell has in its history.
        pub(crate) fn read() -> Result<Self> {
            Ok(read_at(&path()?)?.over(crate::native::command_launcher::shell_history::read()?))
        }

        /// Write down that `command` was started. The file is replaced whole, by a rename, so
        /// a window reading it never finds half of one.
        pub(crate) fn record(command: &str) -> Result<()> {
            let path = path()?;
            let history = read_at(&path)?.with_started(command);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("failed to create {}", dir.display()))?;
            }
            let mut text = history.commands.join("\n");
            text.push('\n');
            let written = path.with_extension(format!("{}.part", std::process::id()));
            std::fs::write(&written, text)
                .with_context(|| format!("failed to write {}", written.display()))?;
            std::fs::rename(&written, &path)
                .with_context(|| format!("failed to replace {}", path.display()))
        }
    }
}

/// A window in a browser starts nothing, so it has nothing to remember.
#[cfg(target_arch = "wasm32")]
impl History {
    pub(crate) fn read() -> anyhow::Result<Self> {
        Ok(Self::default())
    }

    pub(crate) fn record(_command: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A history of these commands, oldest first.
    pub(crate) fn history_of(commands: &[&str]) -> History {
        commands
            .iter()
            .fold(History::default(), |history, command| {
                history.with_started(command)
            })
    }

    #[test]
    fn a_command_started_again_moves_to_the_newest_end() {
        let history = history_of(&["make", "cargo build", "make"]);

        assert_eq!(history.commands, ["cargo build", "make"]);
    }

    #[test]
    fn the_oldest_commands_go_once_there_are_too_many() {
        let mut history = History::default();
        for at in 0..COMMANDS_KEPT + 2 {
            history = history.with_started(&format!("echo {at}"));
        }

        assert_eq!(history.len(), COMMANDS_KEPT);
        assert_eq!(history.command(0), "echo 2");
    }

    #[test]
    fn a_search_finds_the_newest_command_holding_what_was_typed() {
        let history = history_of(&["cargo build", "make deploy", "cargo test", "ls"]);

        let newest = history.newest_containing("cargo", history.len());
        assert_eq!(newest, Some(2));
        assert_eq!(history.newest_containing("cargo", 2), Some(0));
        assert_eq!(history.newest_containing("cargo", 0), None);
        assert_eq!(history.newest_containing("", history.len()), None);
    }

    #[test]
    fn what_was_started_is_read_back_from_the_file() {
        History::record("cargo build").expect("expected the command written down");
        History::record("make deploy").expect("expected the command written down");
        History::record("cargo build").expect("expected the command written down");

        let history = History::read().expect("expected the history read");
        assert_eq!(history.commands, ["make deploy", "cargo build"]);
    }

    /// The shell's history is under what the strip started, and a command in both is where
    /// the strip last started it.
    #[test]
    fn the_shells_history_is_under_what_the_strip_started() {
        crate::native::command_launcher::shell_history::write_for_test(
            "git status\nmake deploy\ngit status\nls\n",
        );
        History::record("make deploy").expect("expected the command written down");

        let history = History::read().expect("expected the history read");
        assert_eq!(history.commands, ["git status", "ls", "make deploy"]);
    }

    /// Starting a command writes the strip's own file, and nothing of the shell's into it.
    #[test]
    fn the_shells_history_is_not_copied_into_the_strips_file() {
        crate::native::command_launcher::shell_history::write_for_test("git status\n");
        History::record("make deploy").expect("expected the command written down");
        History::record("cargo build").expect("expected the command written down");

        let history = History::read().expect("expected the history read");
        assert_eq!(
            history.commands,
            ["git status", "make deploy", "cargo build"]
        );
    }
}
