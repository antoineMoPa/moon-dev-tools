//! Rest the pointer on a name in a file tab and see what the language server says about it -
//! its type, its signature, its docs - with anything the server found wrong under the pointer
//! said above it.
//!
//! When the pointer has rested long enough to be worth a question, and whether an answer is
//! still about the word under it, is [`egui_moon_code_ide::Hovering`]'s. What is here is this
//! window's: the question goes through [`crate::backend::Backend`] on the window's own tasks,
//! and the tooltip renders the server's markdown the way the markdown preview does.

use std::time::Instant;

use egui::{Color32, RichText};
use egui_frames::PaneId;
use egui_moon_code_ide::{HoveringNext, LanguageSource, LspPosition};
use egui_moon_editor::{EditorOutput, Word};

use crate::native::{app::App, language_source::SessionLanguages, theme::Palette};

/// Widest a tooltip grows: a signature wider than this wraps rather than covering the file.
const TOOLTIP_WIDTH: f32 = 560.0;

/// Follow the pointer over a tab's text, and ask about the word it rests on when that is worth
/// a question.
pub(crate) fn follow_the_pointer(
    app: &mut App,
    ctx: &egui::Context,
    pane_id: PaneId,
    session_id: &str,
    output: &EditorOutput,
) {
    let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
        return;
    };
    if !editor.offers_places() {
        return;
    }
    let can_answer = editor.server_heard().can_answer_about(editor.text());
    let asked =
        match editor
            .hovering_mut()
            .follow(output.pointed_word.as_ref(), can_answer, Instant::now())
        {
            HoveringNext::Nothing => return,
            HoveringNext::Wait(after) => {
                ctx.request_repaint_after(after);
                return;
            }
            HoveringNext::Ask(word) => word,
        };
    ask_about(app, pane_id, session_id, asked);
}

fn ask_about(app: &mut App, pane_id: PaneId, session_id: &str, word: Word) {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    let file_path = editor.file_path.clone();
    let for_call = session_id.to_string();
    let at = LspPosition {
        line: word.at.line,
        column: word.at.column,
    };
    app.tasks.spawn_keyed(
        Some(format!("hover:{pane_id}")),
        move |backend| SessionLanguages::new(backend, &for_call).hover(&file_path, at),
        move |model, result| {
            let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                return;
            };
            // Nothing is said about a question that could not be answered: a hover is not a
            // request anybody made, and a message for a pointer passing over a name would be
            // worse than the missing tooltip.
            editor.hovering_mut().answered(&word, result.ok().flatten());
        },
    );
}

/// What the tooltip over the pointer says: the diagnostics under it, each in its grade's
/// colour, and what the server says about the name.
pub(crate) struct Told {
    pub(crate) diagnostics: Vec<(Color32, String)>,
    pub(crate) markdown: Option<String>,
}

/// What a tab's tooltip says this frame, if anything.
pub(crate) fn told(
    app: &App,
    pane_id: PaneId,
    output: &EditorOutput,
    palette: &Palette,
) -> Option<Told> {
    let editor = app.model.file_editors.get(&pane_id)?;
    let diagnostics: Vec<(Color32, String)> = output
        .pointed_at
        .as_ref()
        .map(|at| {
            crate::native::diagnostics::at(editor.text(), editor.diagnosed().found(), at.offset)
                .into_iter()
                .map(|diagnostic| {
                    let said = match &diagnostic.source {
                        Some(source) => format!("{} ({source})", diagnostic.message),
                        None => diagnostic.message.clone(),
                    };
                    (
                        crate::native::diagnostics::colour_of(diagnostic.severity, palette),
                        said,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let markdown = editor
        .hovering()
        .showing(output.pointed_word.as_ref())
        .map(str::to_string);
    (!diagnostics.is_empty() || markdown.is_some()).then_some(Told {
        diagnostics,
        markdown,
    })
}

/// Draw the tooltip at the pointer, while it is over the text.
pub(crate) fn draw_tooltip(app: &mut App, response: &egui::Response, told: Told) {
    let cache = &mut app.model.markdown_cache;
    response.clone().on_hover_ui_at_pointer(|ui| {
        ui.set_max_width(TOOLTIP_WIDTH);
        for (colour, said) in &told.diagnostics {
            ui.label(RichText::new(said).color(*colour));
        }
        if let Some(markdown) = &told.markdown {
            if !told.diagnostics.is_empty() {
                ui.separator();
            }
            egui_commonmark::CommonMarkViewer::new().show(ui, cache, markdown);
        }
    });
}
