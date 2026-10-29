//! Shell - a line of shell typed into the palette after `!`, run at the root of the repo in
//! front with its output in a box under the palette, which stays open.
//!
//! `! git status` and `!git status` are the same line. It runs with `sh -c`, on the PATH the
//! person's login shell has - see [`crate::shell_path`] - rather than in their login shell,
//! whose profile would print its own lines into the box and slow every run down. Enter runs it
//! again; ⌘Enter opens it in a shell of its own, for anything that asks to be typed into.
//!
//! The output is thrown away, and the line stopped if it still runs, when the box's × is
//! clicked, the line is edited away from what ran, or the palette is put away.

use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, Command as Process, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use egui_command_palette::{OutputBox, OutputLine, Row};

use crate::{
    native::{app::App, theme::Palette},
    palette_history::HistoryThing,
};

use super::{Command, CommandAction, sections::Listed};

/// What it says with nothing after the `!`.
const TYPE_A_LINE: &str = "type a line of shell";

/// The line of shell a query is, if it starts with `!`: what follows it, without the spaces
/// round it.
pub(super) fn line_of(query: &str) -> Option<&str> {
    query.trim_start().strip_prefix('!').map(str::trim)
}

/// The line of shell typed into the palette, in a project that runs them - see
/// [`crate::project::ProjectConfig::palette_shell`]. Elsewhere `!` is typed like any letter.
pub(super) fn typed_line(app: &App) -> Option<&str> {
    line_of(app.model.palette.query()).filter(|_| app.model.project.palette_shell())
}

/// A line of shell as a row shows it: `$ line`, in the monospace font, the `$` in green.
pub(super) fn shell_row(line: &str, palette: &Palette) -> Row {
    Row {
        title: format!("$ {line}"),
        title_mono: true,
        inked: vec![(0..1, palette.added)],
        ..Row::default()
    }
}

/// The list while a line of shell is typed: the line alone, which Enter runs under the
/// palette and ⌘Enter in a shell of its own.
pub(super) fn listed(app: &App, line: &str) -> Listed {
    let row = Row {
        description: if line.is_empty() { TYPE_A_LINE } else { "" }.to_string(),
        stays_open: true,
        ..shell_row(line, &app.palette_of())
    };
    let command = Command {
        title: row.title.clone(),
        description: row.description.clone(),
        action: CommandAction::RunInShell(line.to_string()),
        shortcut: None,
    };
    Listed {
        kind: None,
        header: None,
        commands: vec![command],
        rows: vec![row],
        placeholders: 0,
    }
}

/// Whether a line of output came from the line's standard output or its standard error,
/// which the box writes in red.
#[derive(Clone, Copy)]
enum Stream {
    Out,
    Err,
}

/// A line of shell running, or run, under the palette.
pub(crate) struct ShellRun {
    /// What ran, as typed after the `!`.
    line: String,
    child: Child,
    /// What it has printed so far, both streams in the order the lines came.
    printed: Arc<Mutex<Vec<(String, Stream)>>>,
    /// The streams still being read: the line is over once both are done and it has exited.
    streams_open: Arc<AtomicUsize>,
}

impl ShellRun {
    /// Whether it is still running, or still has output on its way.
    fn busy(&mut self) -> bool {
        let exited = self
            .child
            .try_wait()
            .expect("a line of shell the palette started can be waited for")
            .is_some();
        !exited || self.streams_open.load(Ordering::SeqCst) > 0
    }
}

impl ShellRun {
    /// What it has printed so far, line by line.
    #[cfg(test)]
    pub(crate) fn printed(&self) -> Vec<String> {
        self.printed
            .lock()
            .expect("the line's output lock")
            .iter()
            .map(|(text, _)| text.clone())
            .collect()
    }
}

impl Drop for ShellRun {
    /// Thrown away: stopped, with whatever it started, if it still runs.
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = crate::agent::stop_process_group(&mut self.child);
        }
        let _ = self.child.wait();
    }
}

/// The root of the repo the palette's searches read - the review in front's.
fn repo_root(app: &App) -> Option<std::path::PathBuf> {
    let session_id = &app.model.palette.search_session_id;
    let payload = app.model.review_ref(session_id)?.payload.as_ref()?;
    Some(std::path::PathBuf::from(&payload.repo_path))
}

/// Run `line` under the palette, in place of whatever ran there before, and keep it in the
/// history. Only where the repo is on this machine: a window on another machine's repos runs
/// it in a shell there with ⌘Enter.
pub(super) fn run(app: &mut App, ctx: &egui::Context, line: &str) {
    if line.is_empty() {
        return;
    }
    if !app.backend().reads_this_machine() {
        app.model.error(
            "this window's repo is on another machine: ⌘Enter runs the line in a shell there",
        );
        return;
    }
    let Some(repo_root) = repo_root(app) else {
        app.model.error("no repo is open in this window yet");
        return;
    };
    // The one before goes first, stopped if it still runs.
    app.model.palette.shell = None;
    match start(ctx, line, &repo_root) {
        Ok(run) => app.model.palette.shell = Some(run),
        Err(error) => {
            app.model.error(format!("could not run {line}: {error}"));
            return;
        }
    }
    app.record_in_history(HistoryThing::Shell {
        line: line.to_string(),
    });
}

/// Start `line` in `repo_root`, its two streams read into the run line by line as they come.
fn start(
    ctx: &egui::Context,
    line: &str,
    repo_root: &std::path::Path,
) -> std::io::Result<ShellRun> {
    let mut process = Process::new("sh");
    process
        .arg("-c")
        .arg(line)
        .current_dir(repo_root)
        .env("PATH", crate::shell_path::installed_tools_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A group of its own, so stopping it stops what it started too.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut process, 0);
    let mut child = process.spawn()?;
    let printed = Arc::new(Mutex::new(Vec::new()));
    let streams_open = Arc::new(AtomicUsize::new(2));
    let stdout = child.stdout.take().expect("the line's output was piped");
    let stderr = child.stderr.take().expect("the line's errors were piped");
    read_lines(stdout, Stream::Out, &printed, &streams_open, ctx);
    read_lines(stderr, Stream::Err, &printed, &streams_open, ctx);
    Ok(ShellRun {
        line: line.to_string(),
        child,
        printed,
        streams_open,
    })
}

/// Read one of the line's streams on a thread of its own, into `printed`, asking for a frame
/// for each line.
fn read_lines(
    stream: impl Read + Send + 'static,
    from: Stream,
    printed: &Arc<Mutex<Vec<(String, Stream)>>>,
    streams_open: &Arc<AtomicUsize>,
    ctx: &egui::Context,
) {
    let printed = Arc::clone(printed);
    let streams_open = Arc::clone(streams_open);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut bytes = Vec::new();
        // Up to a stream that ends, or one that fails once the line is stopped: either way
        // there is nothing more to read.
        while reader
            .read_until(b'\n', &mut bytes)
            .is_ok_and(|read| read > 0)
        {
            let text = String::from_utf8_lossy(&bytes)
                .trim_end_matches(['\n', '\r'])
                .to_string();
            printed
                .lock()
                .expect("the line's output lock")
                .push((text, from));
            bytes.clear();
            ctx.request_repaint();
        }
        streams_open.fetch_sub(1, Ordering::SeqCst);
        ctx.request_repaint();
    });
}

/// ⌘Enter: open `line` in a shell of its own at the repo's root, beside the others, and put
/// the palette away for it.
pub(super) fn open_in_terminal(app: &mut App, line: &str) {
    if line.is_empty() {
        return;
    }
    let Some(repo_root) = repo_root(app) else {
        app.model.error("no repo is open in this window yet");
        return;
    };
    app.record_in_history(HistoryThing::Shell {
        line: line.to_string(),
    });
    app.model.palette.dismiss();
    app.pending_action = Some(CommandAction::RunInShell(
        crate::native::workspace::shells::line_at_repo(&repo_root, line),
    ));
}

/// Throw the output away once the line typed is no longer the one that ran.
pub(super) fn follow_query(app: &mut App) {
    let typed = typed_line(app);
    if app
        .model
        .palette
        .shell
        .as_ref()
        .is_some_and(|run| Some(run.line.as_str()) != typed)
    {
        app.model.palette.shell = None;
    }
}

/// The box under the palette with what the line printed, and its × to throw it away.
pub(super) fn draw_output(app: &mut App, ctx: &egui::Context) {
    let palette = app.palette_of();
    let Some(run) = &mut app.model.palette.shell else {
        return;
    };
    let busy = run.busy();
    let lines: Vec<OutputLine> = run
        .printed
        .lock()
        .expect("the line's output lock")
        .iter()
        .map(|(text, from)| OutputLine {
            text: text.clone(),
            ink: match from {
                Stream::Out => None,
                Stream::Err => Some(palette.removed),
            },
        })
        .collect();
    let title = format!("$ {}", run.line);
    let output = OutputBox {
        title: &title,
        busy,
        lines: &lines,
        close_button: &|ui| crate::native::widgets::close_button(ui, &palette),
    };
    if app.model.palette.input.show_output(ctx, &output) {
        app.model.palette.shell = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_shell_is_what_follows_the_bang() {
        assert_eq!(line_of("! git status"), Some("git status"));
        assert_eq!(line_of("!git status"), Some("git status"));
        assert_eq!(line_of("  !  ls  "), Some("ls"));
        assert_eq!(line_of("!"), Some(""));
        assert_eq!(line_of("git !status"), None);
    }
}
