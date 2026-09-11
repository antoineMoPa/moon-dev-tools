//! The signature of the call being typed in a file tab, above the caret, with the parameter the
//! caret is at picked out.
//!
//! When to ask, and which answer is still the one to show, is
//! [`egui_moon_code_ide::Signing`]'s. What is here is this window's: the question goes through
//! [`crate::backend::Backend`] on the window's own tasks, and the popup is drawn over the tab.

use std::time::Instant;

use egui::{Align2, Color32, FontId, Key, text::LayoutJob, vec2};
use egui_frames::PaneId;
use egui_moon_code_ide::{LanguageSource, SigningNext};
use egui_moon_editor::EditorOutput;

use crate::native::{app::App, language_source::SessionLanguages, theme::Palette};

/// Widest the popup grows: a long signature wraps rather than running off the tab.
const POPUP_WIDTH: f32 = 620.0;

/// Follow the caret, and ask about the call it is in when that is worth a question.
pub(crate) fn follow_the_caret(
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
    // Escape puts the signature away; the completion list takes Escape first while it is up.
    if output.response.has_focus() && ctx.input(|input| input.key_pressed(Key::Escape)) {
        editor.signing_mut().dismiss();
    }
    let caret = editor.caret();
    let can_answer = editor.server_heard().can_answer_about(editor.text());
    let (signing, text) = editor.signing_and_text();
    let asked = match signing.follow(caret, text, can_answer, Instant::now()) {
        SigningNext::Nothing => return,
        SigningNext::Wait(after) => {
            ctx.request_repaint_after(after);
            return;
        }
        SigningNext::Ask(asked) => asked,
    };

    let file_path = editor.file_path.clone();
    let for_call = session_id.to_string();
    app.tasks.spawn_keyed(
        Some(format!("signature:{pane_id}")),
        move |backend| {
            SessionLanguages::new(backend, &for_call).signature_help(&file_path, asked.at)
        },
        move |model, result| {
            if let Some(editor) = model.file_editors.get_mut(&pane_id) {
                // Nothing is said about a question that could not be answered: nobody asked it.
                editor.signing_mut().answered(asked, result.ok().flatten());
            }
        },
    );
}

/// Draw the signature above the caret, while the tab has the keyboard and there is one to
/// show.
pub(crate) fn draw(
    app: &App,
    ctx: &egui::Context,
    pane_id: PaneId,
    output: &EditorOutput,
    palette: &Palette,
) {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    let (Some(signature), Some(caret)) = (editor.signing().showing(), output.caret_rect) else {
        return;
    };
    if !output.response.has_focus() {
        return;
    }
    let font = FontId::monospace(crate::native::theme::SMALL_SIZE);
    let mut job = LayoutJob::default();
    job.wrap.max_width = POPUP_WIDTH;
    let active = signature
        .active_parameter
        .clone()
        .filter(|range| range.end <= signature.label.len());
    let mut append = |text: &str, color: Color32| {
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
    };
    match active {
        Some(range) => {
            append(&signature.label[..range.start], palette.muted);
            append(&signature.label[range.clone()], palette.accent);
            append(&signature.label[range.end..], palette.muted);
        }
        None => append(&signature.label, palette.muted),
    }
    // The first paragraph of what the server says about the function: a popup over the line
    // being typed has room for a line of prose, not the page.
    let documentation = signature
        .documentation
        .as_deref()
        .and_then(|text| text.split("\n\n").next())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);

    egui::Area::new(egui::Id::new(("signature", pane_id)))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(caret.left_top() - vec2(0.0, 4.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(POPUP_WIDTH);
                ui.label(job);
                if let Some(documentation) = documentation {
                    ui.label(
                        egui::RichText::new(documentation)
                            .size(crate::native::theme::SMALL_SIZE - 1.0)
                            .color(palette.muted),
                    );
                }
            });
        });
}
