//! Format a file tab's text with the language server behind it: rustfmt through rust-analyzer,
//! the TypeScript server's own formatter for TypeScript and JavaScript.
//!
//! The server formats the text it was sent, so the request waits until the tab's text has
//! reached it - the typing of the last moment included - and the edits it answers with go
//! into the tab the way typing would: unsaved, and taken back by undo. A tab typed into while
//! the question was out has moved from under the answer, which is then dropped rather than put
//! in at places that no longer mean what they meant.

use std::time::{Duration, Instant};

use egui_frames::PaneId;
use egui_moon_code_ide::{CanAnswer, LanguageSource, LspFormatting};
use egui_moon_editor::Indent;

use crate::native::{app::App, language_source::SessionLanguages, panes::Pane};

/// How long a format waits for the tab's text to reach the server - see
/// `crate::native::renaming`, which waits the same way for the same reason.
const HEARD_WITHIN: Duration = Duration::from_secs(5);

/// How soon a format waiting on the document sync looks again.
const LOOKS_AGAIN_IN: Duration = Duration::from_millis(50);

/// Whether the tab in front is a file its server may edit, which is when the palette offers to
/// format it.
pub(crate) fn front_tab_formats(app: &App) -> bool {
    app.active_pane().is_some_and(|(pane_id, pane)| {
        matches!(pane, Pane::File { .. })
            && app
                .model
                .file_editors
                .get(&pane_id)
                .is_some_and(|editor| editor.offers_edits())
    })
}

/// Format the file tab in front - ⌥⇧F, and the palette's "format file".
pub(crate) fn start_in_front(app: &mut App) {
    let Some((pane_id, Pane::File { .. })) = app.active_pane() else {
        app.model.error("format file works on a file tab");
        return;
    };
    start(app, pane_id);
}

/// Ask for one file tab to be formatted, once the server has heard what it is showing.
pub(crate) fn start(app: &mut App, pane_id: PaneId) {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    if !editor.offers_edits() {
        let said = match editor.is_outside_the_repo() {
            true => format!(
                "{} is outside the repo, and can only be read",
                editor.file_path
            ),
            false => format!(
                "no language server serves {}, so it cannot be formatted",
                editor.file_path
            ),
        };
        app.model.error(said);
        return;
    }
    if let Some(editor) = app.model.file_editors.get_mut(&pane_id) {
        editor.ask_to_format(Instant::now());
    }
}

/// Send a format a tab asked for, once its text has reached the server. Called as the tab
/// draws, which is where its text is.
pub(crate) fn follow(app: &mut App, ctx: &egui::Context, pane_id: PaneId, session_id: &str) {
    let indent = app.model.project.indent();
    let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
        return;
    };
    let Some(since) = editor.asked_to_format() else {
        return;
    };
    let file_path = editor.file_path.clone();
    match editor.server_heard().can_answer_about(editor.text()) {
        CanAnswer::NotThisText if since.elapsed() < HEARD_WITHIN => {
            ctx.request_repaint_after(LOOKS_AGAIN_IN);
            return;
        }
        CanAnswer::NotThisText => {
            editor.stop_asking_to_format();
            app.model.error(format!(
                "the language server has not heard what is in {file_path} yet - try formatting again in a moment"
            ));
            return;
        }
        // Formatting reads the one file rather than the project, so a server still reading
        // the project formats it all the same.
        CanAnswer::StillReadingTheProject | CanAnswer::Yes => {}
    }
    editor.stop_asking_to_format();
    let told = editor.text().to_string();

    let for_call = session_id.to_string();
    let for_ask = file_path.clone();
    app.tasks.spawn_keyed(
        Some(format!("format:{pane_id}")),
        move |backend| {
            SessionLanguages::new(backend, &for_call).format(&for_ask, formatting_for(indent))
        },
        move |model, result| {
            let edits = match result {
                Ok(edits) => edits,
                Err(error) => {
                    model.error(format!("could not format {file_path}: {error}"));
                    return;
                }
            };
            if edits.is_empty() {
                model.info(format!("{file_path} is already formatted"));
                return;
            }
            let outcome = match model.file_editors.get_mut(&pane_id) {
                // The tab was closed while the question was out.
                None => return,
                Some(editor) if editor.text() != told => Err(format!(
                    "{file_path} was edited while it was being formatted, so it was left as it was"
                )),
                Some(editor) => moon_lsp::edits::byte_ranges(&told, &edits)
                    .map(|ranges| editor.take_edits(ranges))
                    .map_err(|error| format!("could not format {file_path}: {error}")),
            };
            if let Err(said) = outcome {
                model.error(said);
            }
        },
    );
}

/// What the repo's indentation says to a server formatting one of its files. A tab is told as
/// four columns wide, which is what every formatter here takes one to be anyway; rustfmt reads
/// the project's own `rustfmt.toml` over all of it.
fn formatting_for(indent: Indent) -> LspFormatting {
    match indent {
        Indent::Tab => LspFormatting {
            tab_size: 4,
            insert_spaces: false,
        },
        Indent::Spaces(width) => LspFormatting {
            tab_size: width as u32,
            insert_spaces: true,
        },
    }
}
