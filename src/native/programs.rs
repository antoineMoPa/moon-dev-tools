//! The three executables, as things a window can find and start another of.
//!
//! `moonreview`, `moontasks` and `moonshell` ship together and are installed side by side, so
//! a window that wants to open one of the others looks beside itself rather than on `PATH` —
//! the one next to it is the one it was installed with.

use std::{env, path::PathBuf, process::Command};

use anyhow::{Context, Result};

use crate::cli::Frame;

/// Where the running executable is installed, which is where its siblings are.
pub(crate) fn install_dir() -> Result<PathBuf> {
    Ok(env::current_exe()
        .context("failed to locate the running executable")?
        .parent()
        .context("the running executable has no directory")?
        .to_path_buf())
}

/// The executable that opens on this frame, as installed beside the running one.
///
/// The three ship together but only some of them may be installed — an archive from before
/// the split holds one — so a missing sibling is an answer of `None` rather than an error.
pub(crate) fn executable_for(frame: Frame) -> Option<PathBuf> {
    beside(&install_dir().ok()?, frame)
}

fn beside(dir: &std::path::Path, frame: Frame) -> Option<PathBuf> {
    let executable = dir.join(frame.program());
    executable.is_file().then_some(executable)
}

/// How another window is started: the executable, and enough for it to open the same repo.
///
/// A local repo is given as the directory the program is started in, which is what a shell
/// would have given it. A repo on another machine cannot be, so that one is passed the
/// address of the server it is being read through instead.
pub(crate) fn new_window_command(
    executable: &std::path::Path,
    repo_path: &str,
    connect_target: Option<&str>,
) -> Command {
    let mut command = Command::new(executable);
    match connect_target {
        Some(target) => {
            command.args(["--remote", target, "--repo", repo_path]);
        }
        None => {
            command.current_dir(repo_path);
        }
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the executables that are actually there are offered, so a menu item never points
    /// at a program this machine has not got.
    #[test]
    fn a_program_is_found_only_where_one_is_installed() {
        let dir = std::env::temp_dir().join(format!("moonreview-programs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("expected a directory to look in");
        std::fs::write(dir.join(Frame::Tasks.program()), "").expect("expected an executable");

        assert_eq!(
            beside(&dir, Frame::Tasks),
            Some(dir.join(Frame::Tasks.program()))
        );
        assert_eq!(beside(&dir, Frame::Review), None);
        assert_eq!(beside(&dir, Frame::Shell), None);

        std::fs::remove_dir_all(&dir).expect("expected the directory to be removed");
    }

    /// A repo on this machine is handed over as the directory to start in.
    #[test]
    fn a_local_window_opens_in_the_repo() {
        let command = new_window_command(std::path::Path::new("/bin/moontasks"), "/repo", None);

        assert_eq!(command.get_current_dir(), Some(std::path::Path::new("/repo")));
        assert_eq!(command.get_args().count(), 0, "a local window needs no flags");
    }

    /// A repo on another machine is not a directory here, so the new window is told where the
    /// server is and which repo on it to open.
    #[test]
    fn a_remote_window_is_given_the_server_and_the_repo() {
        let command = new_window_command(
            std::path::Path::new("/bin/moontasks"),
            "/home/you/project",
            Some("https://dev-box:42000"),
        );

        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            ["--remote", "https://dev-box:42000", "--repo", "/home/you/project"]
        );
        assert_eq!(
            command.get_current_dir(),
            None,
            "a remote window has no directory here to start in"
        );
    }
}
