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

/// How another window is started: on its launch screen, asking which repo to open.
///
/// A new window is a new place to work rather than a second view of this one, so it opens
/// where the recent projects and the folder picker are rather than on this window's repo.
///
/// A window reading another machine is told which server to ask, and nothing else: a remote
/// window with no `--repo` is already the launch screen, asking for a path over there.
/// `launcher` is the frame's `.app` bundle, when one is installed. Going through it is what
/// makes the new window arrive in front, under its own icon: `open` hands the request to
/// LaunchServices, which starts and activates the application, while a plain executable
/// started from another window is left wherever the window server puts it — which is behind
/// the window it was asked for from. Without a bundle, the executable is all there is.
pub(crate) fn new_window_command(
    executable: &std::path::Path,
    launcher: Option<&std::path::Path>,
    connect_target: Option<&str>,
) -> Command {
    let arguments: Vec<&str> = match connect_target {
        Some(target) => vec!["--remote", target],
        None => vec!["--pick"],
    };

    match launcher {
        Some(bundle) => {
            let mut command = Command::new("open");
            // `-n` because a second window is a second instance: without it LaunchServices
            // brings the running one forward and the arguments go nowhere.
            command.arg("-n").arg("-a").arg(bundle).arg("--args");
            command.args(&arguments);
            command
        }
        None => {
            let mut command = Command::new(executable);
            command.args(&arguments);
            command
        }
    }
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

    /// A new window opens on the launch screen rather than on this window's repo, and takes
    /// no directory with it: where it was started from is not what it opens on.
    #[test]
    fn a_new_window_opens_on_the_launch_screen() {
        let command = new_window_command(std::path::Path::new("/bin/moontasks"), None, None);

        assert_eq!(command.get_program(), "/bin/moontasks");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["--pick"]);
        assert_eq!(command.get_current_dir(), None);
    }

    /// A window reading another machine is given that server, and no repo — which is the
    /// launch screen over there, asking which of its repos to open.
    #[test]
    fn a_remote_window_is_given_the_server_and_no_repo() {
        let command = new_window_command(
            std::path::Path::new("/bin/moontasks"),
            None,
            Some("https://dev-box:42000"),
        );

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--remote", "https://dev-box:42000"]
        );
    }

    /// With a launcher installed the request goes through the OS, which is what brings the
    /// new window to the front — a second instance of it, carrying the same arguments.
    #[test]
    fn a_window_with_a_launcher_is_opened_through_it() {
        let command = new_window_command(
            std::path::Path::new("/bin/moontasks"),
            Some(std::path::Path::new("/Applications/Moontasks.app")),
            None,
        );

        assert_eq!(command.get_program(), "open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "-n",
                "-a",
                "/Applications/Moontasks.app",
                "--args",
                "--pick"
            ]
        );
    }
}
