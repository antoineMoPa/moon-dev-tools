//! A command typed into the status bar and started on the spot: `⌘⇧!` turns the strip along
//! the bottom of the window into a line to type in, and Enter starts what was typed.
//!
//! What is started has no terminal and a process group of its own, and is waited on by a worker
//! thread rather than the window, so the strip is free again the moment Enter is pressed. It
//! runs from the repo's root. When it ends, what it printed - both streams, cut to 100 lines -
//! is posted to the message buffer, with no toast. A command that wants a terminal belongs in
//! a shell tab.

use egui::{Key, RichText, Ui};

use crate::native::{
    app::App,
    model::ToastKind,
    theme::{Palette, SMALL_SIZE},
};

/// The line being typed in the strip.
pub(crate) struct CommandLauncher {
    pub(crate) text: String,
    /// Set until the box has been given the keyboard, which it takes the frame it opens.
    focus: bool,
}

impl CommandLauncher {
    pub(crate) fn open() -> Self {
        Self {
            text: String::new(),
            focus: true,
        }
    }
}

/// What the line asked for this frame.
pub(crate) enum Asked {
    Nothing,
    Start,
    Cancel,
}

/// Draw the line in place of the strip's own contents, and answer what it asked for.
pub(crate) fn draw_line(ui: &mut Ui, launcher: &mut CommandLauncher, palette: &Palette) -> Asked {
    let mut asked = Asked::Nothing;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("run")
                .monospace()
                .size(SMALL_SIZE)
                .color(palette.muted),
        );
        let response = ui.add(
            egui::TextEdit::singleline(&mut launcher.text)
                .font(egui::TextStyle::Monospace)
                .frame(egui::Frame::NONE)
                .hint_text("a command to start detached, from the repo's root - Enter starts it, Esc cancels")
                .desired_width(f32::INFINITY),
        );
        if launcher.focus {
            response.request_focus();
            launcher.focus = false;
        } else if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            asked = Asked::Start;
        } else if !response.has_focus() || ui.input(|input| input.key_pressed(Key::Escape)) {
            asked = Asked::Cancel;
        }
    });
    asked
}

impl App {
    /// Put the line up in the strip, or take the keyboard to it if it is up already.
    pub(crate) fn open_command_launcher(&mut self) {
        match &mut self.model.command_launcher {
            Some(launcher) => launcher.focus = true,
            None => self.model.command_launcher = Some(CommandLauncher::open()),
        }
    }

    /// Start what was typed, and put the line away. No toast: the command's output, once it
    /// has ended, is posted to the message buffer.
    pub(crate) fn start_launched_command(&mut self, command: &str) {
        let command = command.trim().to_string();
        if command.is_empty() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let root = self.model.root_repo_path();
        #[cfg(target_arch = "wasm32")]
        let root = None;
        let for_apply = command.clone();
        self.tasks.spawn(
            move |_backend| run_detached(root.clone(), &command),
            move |model, result| match result {
                Ok(finished) => model.log_message(
                    match finished.succeeded {
                        true => ToastKind::Info,
                        false => ToastKind::Error,
                    },
                    finished.message(&for_apply),
                ),
                Err(error) => model.log_message(
                    ToastKind::Error,
                    format!("could not start `{for_apply}`: {error}"),
                ),
            },
        );
    }
}

/// How many lines of a command's output are kept for the message buffer.
const OUTPUT_LINES_KEPT: usize = 100;

/// What a command printed, and whether it ended well.
struct Finished {
    output: String,
    succeeded: bool,
}

impl Finished {
    /// The command, then its output cut to [`OUTPUT_LINES_KEPT`] lines.
    fn message(&self, command: &str) -> String {
        let lines: Vec<&str> = self.output.lines().collect();
        let mut message = format!("$ {command}");
        for line in lines.iter().take(OUTPUT_LINES_KEPT) {
            message.push('\n');
            message.push_str(line);
        }
        if lines.len() > OUTPUT_LINES_KEPT {
            message.push_str(&format!(
                "\n… {} more lines",
                lines.len() - OUTPUT_LINES_KEPT
            ));
        }
        if !self.succeeded {
            message.push_str("\n(failed)");
        }
        message
    }
}

/// Run `command` through the shell in a session of its own - not a child of the window's
/// terminal, and not stopped by it - and wait for it here, on a worker thread, collecting what
/// it prints on either stream. The window is never waited on.
#[cfg(not(target_arch = "wasm32"))]
fn run_detached(root: Option<std::path::PathBuf>, command: &str) -> anyhow::Result<Finished> {
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };

    let root = root.ok_or_else(|| anyhow::anyhow!("the repo has not been read yet"))?;
    let output = Command::new("sh")
        // Both streams to the one pipe, so the lines come in the order they were printed.
        .args(["-c", &format!("exec 2>&1; {command}")])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .output()?;
    Ok(Finished {
        output: String::from_utf8_lossy(&output.stdout).into_owned(),
        succeeded: output.status.success(),
    })
}

#[cfg(target_arch = "wasm32")]
fn run_detached(_root: Option<std::path::PathBuf>, _command: &str) -> anyhow::Result<Finished> {
    anyhow::bail!("a window in a browser cannot start programs on the machine")
}
