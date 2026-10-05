//! A command typed into the status bar and started on the spot: `⌘⇧!` turns the strip along
//! the bottom of the window into a line to type in, and Enter starts what was typed.
//!
//! What is started has no terminal and a process group of its own, and is waited on by a worker
//! thread rather than the window, so the strip is free again the moment Enter is pressed. It
//! runs from the repo's root, through the login shell with the PATH, aliases and functions a
//! terminal has. When it ends, what it printed - both streams, cut to 100 lines -
//! is posted to the message buffer, with no toast. A command that wants a terminal belongs in
//! a shell tab.
//!
//! The line has a history, the way a shell does: the arrows walk back through it and ctrl+r
//! searches it. It is what the line started before and, under that, the login shell's own
//! history - see [`history`] for where each is kept and [`recall`] for the rules.
//!
//! The keys, since the line says none of this itself: Enter starts what is typed and Esc
//! puts the line away. In a search, ctrl+r again looks further back, Enter starts what was
//! found, an arrow along the line takes it into the line to be changed first, and Esc goes
//! back to the line.

mod history;
mod recall;
#[cfg(not(target_arch = "wasm32"))]
mod shell_history;

use egui::{Key, Modifiers, RichText, Ui};

use self::{
    history::History,
    recall::{Search, Walk},
};
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
    /// Set when the box was filled by something other than typing, and the caret belongs
    /// after what it was filled with.
    caret_to_end: bool,
    /// What was started from the strip before, as it stood when the line was put up.
    history: History,
    walk: Walk,
    /// The search ctrl+r opened. While there is one the box holds what is being looked for,
    /// and `text` waits behind it untouched.
    search: Option<Search>,
}

impl CommandLauncher {
    pub(crate) fn open(history: History) -> Self {
        Self {
            text: String::new(),
            focus: true,
            caret_to_end: false,
            history,
            walk: Walk::default(),
            search: None,
        }
    }

    /// Put a command in the box in place of what it held.
    fn fill(&mut self, command: String) {
        self.text = command;
        self.caret_to_end = true;
    }

    /// Answer this frame's keys: the ones a shell recalls a command with. They are taken out
    /// of the input here, before the box is drawn, so an arrow does not also move its caret.
    fn recall(&mut self, ui: &Ui) {
        let searching = self.search.is_some();
        let (search, older, newer, leave_search, take_found) = ui.input_mut(|input| {
            (
                input.consume_key(Modifiers::CTRL, Key::R),
                input.consume_key(Modifiers::NONE, Key::ArrowUp),
                input.consume_key(Modifiers::NONE, Key::ArrowDown),
                searching && input.consume_key(Modifiers::NONE, Key::Escape),
                // Moving along the line is how a shell is told the command it found is to be
                // changed before it is started.
                searching
                    && (input.consume_key(Modifiers::NONE, Key::ArrowLeft)
                        | input.consume_key(Modifiers::NONE, Key::ArrowRight)),
            )
        });
        match &mut self.search {
            Some(found) if search => found.look_further_back(&self.history),
            Some(found) if take_found => {
                if let Some(command) = found.found(&self.history) {
                    self.text = command.to_string();
                }
                self.search = None;
                self.caret_to_end = true;
            }
            Some(_) if leave_search => {
                self.search = None;
                self.caret_to_end = true;
                // egui takes the keyboard off whatever has it on Escape, before anything is
                // drawn. The line is still up, so it takes it back.
                self.focus = true;
            }
            Some(_) => {}
            None if search => {
                self.search = Some(Search::default());
                self.caret_to_end = true;
            }
            None if older => {
                if let Some(command) = self.walk.older(&self.history, &self.text) {
                    self.fill(command);
                }
            }
            None if newer => {
                if let Some(command) = self.walk.newer(&self.history) {
                    self.fill(command);
                }
            }
            None => {}
        }
    }
}

/// Put a history where the login shell's is read from, for a test of the line with one.
#[cfg(test)]
pub(crate) fn write_shell_history_for_test(contents: &str) {
    shell_history::write_for_test(contents);
}

/// What the line asked for this frame.
pub(crate) enum Asked {
    Nothing,
    Start,
    Cancel,
}

/// The box is the same box whether it holds the command or what ctrl+r is looking for, so the
/// keyboard stays in it when one is swapped for the other.
fn line_id() -> egui::Id {
    egui::Id::new("moonreview-command-launcher-line")
}

/// Draw the line in place of the strip's own contents, and answer what it asked for.
pub(crate) fn draw_line(ui: &mut Ui, launcher: &mut CommandLauncher, palette: &Palette) -> Asked {
    launcher.recall(ui);
    let mut asked = Asked::Nothing;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let small = |text: &str, color| {
            RichText::new(text)
                .monospace()
                .size(SMALL_SIZE)
                .color(color)
        };
        let found = launcher
            .search
            .as_ref()
            .map(|search| search.found(&launcher.history).map(str::to_string));
        let nothing_typed = launcher
            .search
            .as_ref()
            .is_some_and(|search| search.query.is_empty());
        // The way a shell words it, which is what says the box is no longer the command.
        match &found {
            None => ui.label(small("run ", palette.muted)),
            Some(None) if !nothing_typed => {
                ui.label(small("(failed reverse-i-search)`", palette.warn))
            }
            Some(_) => ui.label(small("(reverse-i-search)`", palette.muted)),
        };

        let held = match &mut launcher.search {
            Some(search) => &mut search.query,
            None => &mut launcher.text,
        };
        if std::mem::take(&mut launcher.caret_to_end) {
            let mut state =
                egui::text_edit::TextEditState::load(ui.ctx(), line_id()).unwrap_or_default();
            let end = egui::text::CCursor::new(held.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), line_id());
        }
        let line = egui::TextEdit::singleline(held)
            .id(line_id())
            .font(egui::TextStyle::Monospace)
            .frame(egui::Frame::NONE);
        let response = ui.add(match found {
            // As wide as what is being looked for, so what was found reads on after it.
            Some(_) => line.desired_width(0.0).clip_text(false),
            None => line.desired_width(f32::INFINITY),
        });
        if response.changed()
            && let Some(search) = &mut launcher.search
        {
            search.look_again(&launcher.history);
        }
        match &found {
            None => {}
            Some(Some(command)) => {
                ui.label(small("': ", palette.muted));
                ui.label(RichText::new(command).monospace().color(palette.ink));
            }
            Some(None) => {
                ui.label(small("': ", palette.muted));
            }
        }

        let entered = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        if launcher.focus {
            response.request_focus();
            launcher.focus = false;
        } else if entered {
            match (launcher.search.is_some(), found.flatten()) {
                (false, _) => asked = Asked::Start,
                (true, Some(command)) => {
                    launcher.search = None;
                    launcher.text = command;
                    asked = Asked::Start;
                }
                // Enter on a search that found nothing starts nothing: the line stays up,
                // still searching, and takes the keyboard back.
                (true, None) => launcher.focus = true,
            }
        } else if !response.has_focus() || ui.input(|input| input.key_pressed(Key::Escape)) {
            asked = Asked::Cancel;
        }
    });
    asked
}

impl App {
    /// Put the line up in the strip, or take the keyboard to it if it is up already.
    pub(crate) fn open_command_launcher(&mut self) {
        if let Some(launcher) = &mut self.model.command_launcher {
            launcher.focus = true;
            return;
        }
        // A history that cannot be read is said, and the line opens without one: a command
        // can still be typed.
        let history = History::read().unwrap_or_else(|error| {
            self.model.log_message(
                ToastKind::Error,
                format!("could not read the commands started before: {error:#}"),
            );
            History::default()
        });
        self.model.command_launcher = Some(CommandLauncher::open(history));
    }

    /// Start what was typed, and put the line away. No toast: the command's output, once it
    /// has ended, is posted to the message buffer.
    pub(crate) fn start_launched_command(&mut self, command: &str) {
        let command = command.trim().to_string();
        if command.is_empty() {
            return;
        }
        if let Err(error) = History::record(&command) {
            self.model.log_message(
                ToastKind::Error,
                format!("could not write `{command}` down as started: {error:#}"),
            );
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

/// Run `command` through the person's login shell, started the way a terminal starts it - a
/// login shell, and an interactive one - so the line means what it means at their prompt: their
/// PATH, their aliases, their functions. A plain `sh` knows none of the three, and a login shell
/// that is not interactive does not expand an alias.
///
/// In a process group of its own - not a child of the window's terminal, and not stopped by it -
/// and waited for here, on a worker thread, collecting what it prints on either stream. The
/// window is never waited on. What the shell itself says as it starts with no terminal to be
/// interactive on goes nowhere: the streams are joined by the line, after that.
#[cfg(not(target_arch = "wasm32"))]
fn run_detached(root: Option<std::path::PathBuf>, command: &str) -> anyhow::Result<Finished> {
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };

    let root = root.ok_or_else(|| anyhow::anyhow!("the repo has not been read yet"))?;
    let output = Command::new(crate::shell_path::login_shell())
        // Both streams to the one pipe, so the lines come in the order they were printed.
        .args(["-l", "-i", "-c", &format!("exec 2>&1; {command}")])
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// What is started is waited for, and both of its streams come back in the order they
    /// were printed. `contains` rather than equals: the shell is the login shell of whoever
    /// runs the test, and what its profile prints is theirs.
    #[test]
    fn a_started_command_answers_with_what_it_printed_on_either_stream() {
        let finished = run_detached(
            Some(std::env::temp_dir()),
            "echo to-stdout; echo to-stderr >&2",
        )
        .expect("expected the command to start");

        assert!(finished.succeeded);
        assert!(
            finished.output.contains("to-stdout\nto-stderr\n"),
            "got {:?}",
            finished.output
        );

        let failed = run_detached(Some(std::env::temp_dir()), "exit 3")
            .expect("expected the command to start");
        assert!(!failed.succeeded);
    }
}
