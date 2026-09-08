//! Starting another window, which is another run of this same executable.
//!
//! A window is a process: each one carries its own review server and its own shells, so a
//! second window is a second run of `moon`, told which frame to open on.

use std::{env, path::PathBuf, process::Command};

use anyhow::{Context, Result};

use crate::cli::Frame;

/// The executable this window is running, which is the one another window is started from -
/// the installed one, whichever copy that is.
pub(crate) fn this_executable() -> Result<PathBuf> {
    env::current_exe().context("failed to locate the running executable")
}

/// What a window that is about to be started opens on.
#[derive(Clone, Copy)]
pub(crate) enum Opens<'a> {
    /// The launch screen, asking which repo to work in. A new window is a new place to work
    /// rather than a second view of this one, so it opens where the recent projects and the
    /// folder picker are rather than on this window's repo.
    LaunchScreen,
    /// The repo at this path, which is how a restarted window comes back where it was.
    Repo(&'a str),
}

/// How another window is started.
///
/// A window reading another machine is told which server to ask: a remote window with no
/// `--repo` is already the launch screen, asking for a path over there.
/// `launcher` is the frame's `.app` bundle, when one is installed. Going through it is what
/// makes the new window arrive in front, under its own icon: `open` hands the request to
/// LaunchServices, which starts and activates the application, while a plain executable
/// started from another window is left wherever the window server puts it - which is behind
/// the window it was asked for from. Without a bundle, the executable is all there is.
pub(crate) fn window_command(
    executable: &std::path::Path,
    launcher: Option<&std::path::Path>,
    frame: Frame,
    connect_target: Option<&str>,
    opens: Opens<'_>,
) -> Command {
    let mut arguments: Vec<&str> = Vec::new();
    if let Some(target) = connect_target {
        arguments.extend(["--remote", target]);
    }
    match opens {
        Opens::Repo(path) => arguments.extend(["--repo", path]),
        // A remote window is already asking which repo to open, so `--pick` - which takes
        // nothing else - is only for a window of this machine.
        Opens::LaunchScreen if connect_target.is_none() => arguments.push("--pick"),
        Opens::LaunchScreen => {}
    }

    match launcher {
        Some(bundle) => {
            let mut command = Command::new("open");
            // `-n` because a second window is a second instance: without it LaunchServices
            // brings the running one forward and the arguments go nowhere.
            command.arg("-n").arg("-a").arg(bundle).arg("--args");
            // No frame is named: the bundle's executable is the frame's own name, which is
            // what a window started from a launcher reads its frame out of - see
            // `crate::cli::parse_command`. Naming it here as well would leave the window
            // reading `shell` as a path to review.
            command.args(&arguments);
            command
        }
        None => {
            let mut command = Command::new(executable);
            command.arg(frame.subcommand());
            command.args(&arguments);
            command
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new window opens on the launch screen rather than on this window's repo, and takes
    /// no directory with it: where it was started from is not what it opens on.
    #[test]
    fn a_new_window_opens_on_the_launch_screen() {
        let command = window_command(
            std::path::Path::new("/bin/moon"),
            None,
            Frame::Tasks,
            None,
            Opens::LaunchScreen,
        );

        assert_eq!(command.get_program(), "/bin/moon");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["tasks", "--pick"]);
        assert_eq!(command.get_current_dir(), None);
    }

    /// A window reading another machine is given that server, and no repo - which is the
    /// launch screen over there, asking which of its repos to open.
    #[test]
    fn a_remote_window_is_given_the_server_and_no_repo() {
        let command = window_command(
            std::path::Path::new("/bin/moon"),
            None,
            Frame::Tasks,
            Some("https://dev-box:42000"),
            Opens::LaunchScreen,
        );

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["tasks", "--remote", "https://dev-box:42000"]
        );
    }

    /// With a launcher installed the request goes through the OS, which is what brings the
    /// new window to the front - a second instance of it, carrying the same arguments.
    ///
    /// The frame is not among them: the bundle is the frame, and what it starts reads which
    /// one out of the name it was started under.
    #[test]
    fn a_window_with_a_launcher_is_opened_through_it_and_told_no_frame() {
        let command = window_command(
            std::path::Path::new("/bin/moon"),
            Some(std::path::Path::new("/Applications/Moontasks.app")),
            Frame::Tasks,
            None,
            Opens::LaunchScreen,
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

    /// A restarted window is handed the repo it was on, so it comes back there rather than
    /// on the launch screen.
    #[test]
    fn a_restarted_window_is_given_the_repo_it_was_on() {
        let command = window_command(
            std::path::Path::new("/bin/moon"),
            None,
            Frame::Shell,
            None,
            Opens::Repo("/home/you/project"),
        );

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["shell", "--repo", "/home/you/project"]
        );
    }

    /// Restarting a window that reads another machine keeps both the server it asks and the
    /// repo it was on over there.
    #[test]
    fn a_restarted_remote_window_keeps_its_server_and_repo() {
        let command = window_command(
            std::path::Path::new("/bin/moon"),
            None,
            Frame::Shell,
            Some("https://dev-box:42000"),
            Opens::Repo("/home/you/project"),
        );

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "shell",
                "--remote",
                "https://dev-box:42000",
                "--repo",
                "/home/you/project"
            ]
        );
    }
}
