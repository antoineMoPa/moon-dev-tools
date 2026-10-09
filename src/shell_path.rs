//! Shell PATH - resolves the PATH used to find and start the tools the user installed: the
//! coding agents, and `ag` for finding files by name.
//!
//! A window opened from a desktop launcher is started by the OS, not by a shell, so it inherits
//! a bare PATH - on macOS `/usr/bin:/bin:/usr/sbin:/sbin`. What the user installed lives in
//! `~/.local/bin`, `/opt/homebrew/bin` and the like, which only their shell profile puts on
//! PATH, so from a launcher every agent reads as missing and the board offers none.
//!
//! Their login shell is what knows where those are, so it is asked once for its PATH and that
//! is the PATH both the availability checks and the processes use.
//!
//! On a server that gives each person a Unix user - see [`crate::unix_users`] - "the user" of
//! a tool is the person it is run for, and [`installed_tool`] is what runs it as them, on
//! their PATH rather than the server's.

use std::{env, process::Command, sync::OnceLock};

use anyhow::Result;

/// The shell the user's account is set up with, which is the one that reads their profile.
pub(crate) fn login_shell() -> String {
    env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// The PATH a login shell of this user has, resolved once for the life of the process.
pub(crate) fn installed_tools_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| login_shell_path().unwrap_or_else(|| env::var("PATH").unwrap_or_default()))
}

/// The PATH those tools are looked for on, for this thread's current user: the person's
/// own login PATH on a server that gives each a Unix user, and this process's user's anywhere
/// else.
pub(crate) fn tools_path_of_the_current_user() -> Result<String> {
    match crate::unix_users::current_user() {
        Some(person) => person.unix_user.login_path(),
        None => Ok(installed_tools_path().to_string()),
    }
}

/// `program`, one of those tools, to be run as this thread's current user - see
/// [`crate::unix_users::command`] - and looked for on the login PATH of that user.
///
/// A person's command comes with their own login PATH and keeps it: the server's is root's
/// there, and what a person installed in their home is not on it. Anywhere else the command
/// is this process's own user's, and is given [`installed_tools_path`].
pub(crate) fn installed_tool(program: &str) -> Result<Command> {
    let mut command = crate::unix_users::command(program)?;
    if crate::unix_users::current_user().is_none() {
        command.env("PATH", installed_tools_path());
    }
    Ok(command)
}

/// Run the login shell for its PATH. `None` when it cannot be run or says nothing, which is
/// the case for a shell whose profile is broken - the process PATH is the answer then.
fn login_shell_path() -> Option<String> {
    let output = Command::new(login_shell())
        .args(["-lc", "printf %s \"$PATH\""])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim().to_string();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where nobody has a Unix user of their own, which is everywhere but a shared server, a
    /// tool is the program itself, found where this user's login shell finds it.
    #[test]
    fn a_tool_run_for_nobody_is_the_program_on_this_users_login_path() {
        let command = installed_tool("ag").expect("expected a command");

        assert_eq!(command.get_program(), "ag");
        assert_eq!(command.get_args().count(), 0);
        let path = command
            .get_envs()
            .find(|(name, _)| *name == "PATH")
            .and_then(|(_, path)| path);
        assert_eq!(path, Some(std::ffi::OsStr::new(installed_tools_path())));
    }
}
