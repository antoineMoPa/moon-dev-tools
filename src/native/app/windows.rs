//! The other processes of this program a window starts: another window, itself again, and
//! the launchers the OS starts them from. None of it is compiled for the browser, whose window
//! is a page and has no program on this machine to start.

use web_time::Instant;

use crate::native::programs::Opens;

use super::{App, QUIT_CONFIRM_WINDOW};

impl App {
    /// Open another window - of this program or of one of its siblings - on its launch
    /// screen, where it asks which repo to open.
    ///
    /// A window is a process here: each one carries its own review server and its own shells,
    /// so there is nothing to open a second window out of but a second run of the executable.
    /// It is left to run on its own; closing this one does not take it with it.
    pub(super) fn open_new_window(&mut self, frame: crate::cli::Frame) {
        let executable = match crate::native::programs::this_executable() {
            Ok(executable) => executable,
            Err(error) => {
                self.model
                    .error(format!("could not find this window's own program: {error}"));
                return;
            }
        };

        if let Err(error) = self.start_window(frame, &executable, Opens::LaunchScreen) {
            self.model.error(format!(
                "could not open a {} window: {error:#}",
                frame.display_name()
            ));
        }
    }

    /// Start this program again on the repo this window is on, and close this window once the
    /// new one is on its way.
    ///
    /// A window runs the executable it was started with, so a rebuilt one only reaches the
    /// screen through a second process. The new instance is started first: a window that
    /// closed on a failed spawn would leave the user with nothing.
    pub(crate) fn restart_window(&mut self, ctx: &egui::Context) {
        let frame = self.frame;
        let executable = match crate::native::programs::this_executable() {
            Ok(executable) => executable,
            Err(error) => {
                self.model
                    .error(format!("could not find this window's own program: {error}"));
                return;
            }
        };

        // Without a project the window is on its launch screen, and that is where it comes
        // back to.
        let project_path = self.model.project_path.clone();
        let opens = match &project_path {
            Some(path) => Opens::Repo(path),
            None => Opens::LaunchScreen,
        };
        match self.start_window(frame, &executable, opens) {
            Ok(()) => self.close_window(ctx),
            Err(error) => self
                .model
                .error(format!("could not restart this window: {error:#}")),
        }
    }

    /// Close this window because the window itself was told to go, rather than because someone
    /// pressed ⌘Q.
    ///
    /// Asking for a restart is already the answer to "a shell is still running": the window
    /// arms the confirmation on its way out, so the close it sends itself is not questioned
    /// back and answered with a toast instead of a new window.
    pub(crate) fn close_window(&mut self, ctx: &egui::Context) {
        self.quit_armed_until = Some(Instant::now() + QUIT_CONFIRM_WINDOW);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Start another process of `frame`, against the same machine this window reads.
    fn start_window(
        &self,
        frame: crate::cli::Frame,
        executable: &std::path::Path,
        opens: Opens<'_>,
    ) -> anyhow::Result<()> {
        let target = self.backend().connect_target();
        let launcher = crate::native::launchers::installed_launcher(frame);
        crate::native::programs::window_command(
            executable,
            launcher.as_deref(),
            frame,
            target.as_ref(),
            opens,
        )?
        .spawn()?;
        Ok(())
    }

    /// Write the launchers the OS lists, and say what landed where.
    pub(super) fn install_launchers(&mut self) {
        match crate::native::launchers::install() {
            Ok(installed) => {
                let names: Vec<&str> = installed
                    .iter()
                    .map(|launcher| launcher.frame.display_name())
                    .collect();
                self.model.info(format!(
                    "{} in {}",
                    names.join(", "),
                    crate::native::launchers::destination_hint()
                ));
            }
            Err(error) => self
                .model
                .error(format!("could not write the launchers: {error}")),
        }
    }
}
