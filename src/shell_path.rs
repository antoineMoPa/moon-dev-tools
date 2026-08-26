//! The PATH the tools the user installed are looked for and started on — the coding agents,
//! and `ag` for finding files by name.
//!
//! A window opened from a desktop launcher is started by the OS, not by a shell, so it inherits
//! a bare PATH — on macOS `/usr/bin:/bin:/usr/sbin:/sbin`. What the user installed lives in
//! `~/.local/bin`, `/opt/homebrew/bin` and the like, which only their shell profile puts on
//! PATH, so from a launcher every agent reads as missing and the board offers none.
//!
//! Their login shell is what knows where those are, so it is asked once for its PATH and that
//! is the PATH both the availability checks and the processes use.

use std::{env, process::Command, sync::OnceLock};

/// The shell the user's account is set up with, which is the one that reads their profile.
pub(crate) fn login_shell() -> String {
    env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// The PATH a login shell of this user has, resolved once for the life of the process.
pub(crate) fn installed_tools_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        login_shell_path().unwrap_or_else(|| env::var("PATH").unwrap_or_default())
    })
}

/// Run the login shell for its PATH. `None` when it cannot be run or says nothing, which is
/// the case for a shell whose profile is broken — the process PATH is the answer then.
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

