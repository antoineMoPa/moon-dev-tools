//! Code actions: what the language server offers to do to the code at a file tab's caret - the
//! fixes for what it found wrong there, and the rewrites it has for the code - picked from the
//! palette and put in the way a rename is.
//!
//! ⌘., the palette's "code actions" and the tab's context menu ask. The question waits until
//! every tab's text has reached its server - an action's edits are places in those texts, and
//! it can reach past the tab it was asked in - and goes out with what the server found wrong at
//! the caret, which is what its fixes are offered for. What comes back is a list in the
//! palette; the one picked goes in through [`crate::native::workspace_edits`].

use std::{collections::HashMap, time::Instant};

use egui_frames::PaneId;
use egui_moon_code_ide::{CanAnswer, LanguageSource, LspCodeAction, still_starting};

use crate::native::{
    app::App,
    language_source::SessionLanguages,
    model::Model,
    palette::PaletteMode,
    panes::Pane,
    workspace_edits::{
        HEARD_WITHIN, LOOKS_AGAIN_IN, counted, not_heard, put_in, tab_behind_its_server,
        what_the_tabs_hold,
    },
};

/// Where asking for code actions is. One at a time for the window: the palette has one list.
pub(crate) enum CodeActing {
    /// Waiting for the tabs' text to reach their servers before asking.
    WaitingToAsk {
        session_id: String,
        pane_id: PaneId,
        since: Instant,
    },
    /// The question is out.
    Asking,
    /// The palette is listing what came back, waiting for one to be picked.
    Choosing {
        session_id: String,
        /// What every tab held when the question went out - the texts the edits are places in.
        told: HashMap<String, String>,
        actions: Vec<LspCodeAction>,
    },
}

/// What the palette is offering to pick from, while it is.
pub(crate) fn offered(model: &Model) -> Option<&[LspCodeAction]> {
    match &model.code_acting {
        Some(CodeActing::Choosing { actions, .. }) => Some(actions),
        _ => None,
    }
}

/// Whether the tab in front is a file its server may edit, which is when the palette offers
/// "code actions".
pub(crate) fn front_tab_has_actions(app: &App) -> bool {
    app.active_pane().is_some_and(|(pane_id, pane)| {
        matches!(pane, Pane::File { .. })
            && app
                .model
                .file_editors
                .get(&pane_id)
                .is_some_and(|editor| editor.offers_edits())
    })
}

/// Ask for the code actions at the caret of the tab in front - ⌘., and the palette.
pub(crate) fn start_in_front(app: &mut App) {
    let Some((pane_id, Pane::File { session_id, .. })) = app.active_pane() else {
        app.model
            .error("code actions work on the caret of a file tab");
        return;
    };
    let session_id = session_id.clone();
    start(app, pane_id, &session_id);
}

/// Ask for the code actions at the caret of one file tab. Says why not, for every way it
/// cannot: a key that did nothing reads as a broken key.
pub(crate) fn start(app: &mut App, pane_id: PaneId, session_id: &str) {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    let file_path = &editor.file_path;
    let refused = if app.model.code_acting.is_some() {
        Some("code actions are already being asked for".to_string())
    } else if editor.is_outside_the_repo() {
        Some(format!(
            "{file_path} is outside the repo, and can only be read"
        ))
    } else if !editor.offers_edits() {
        Some(format!(
            "no language server serves {file_path}, so it has no code actions"
        ))
    } else if editor.caret().is_none() {
        Some("put the caret on the code first".to_string())
    } else {
        None
    };
    if let Some(said) = refused {
        app.model.error(said);
        return;
    }
    app.model.code_acting = Some(CodeActing::WaitingToAsk {
        session_id: session_id.to_string(),
        pane_id,
        since: Instant::now(),
    });
}

/// Move the question along, once a frame. Called after the frame's deferred action has run, so
/// a pick made in the palette this frame has already been applied rather than read as the
/// palette put away.
pub(crate) fn follow(app: &mut App, ctx: &egui::Context) {
    let Some(acting) = app.model.code_acting.take() else {
        return;
    };
    app.model.code_acting = match acting {
        CodeActing::WaitingToAsk {
            session_id,
            pane_id,
            since,
        } => ask(app, ctx, session_id, pane_id, since),
        // The palette was put away without anything being picked.
        CodeActing::Choosing { .. }
            if !(app.model.palette.open && app.model.palette.mode == PaletteMode::CodeActions) =>
        {
            None
        }
        waiting => Some(waiting),
    };
}

/// Ask, once every tab's text has reached its server.
fn ask(
    app: &mut App,
    ctx: &egui::Context,
    session_id: String,
    pane_id: PaneId,
    since: Instant,
) -> Option<CodeActing> {
    // The tab was closed while the question waited, and took it with it.
    let editor = app.model.file_editors.get(&pane_id)?;
    let file_path = editor.file_path.clone();
    let waiting = CodeActing::WaitingToAsk {
        session_id: session_id.clone(),
        pane_id,
        since,
    };
    match editor.server_heard().can_answer_about(editor.text()) {
        CanAnswer::StillReadingTheProject => {
            app.model.error(still_starting(&file_path));
            return None;
        }
        CanAnswer::NotThisText if since.elapsed() < HEARD_WITHIN => {
            ctx.request_repaint_after(LOOKS_AGAIN_IN);
            return Some(waiting);
        }
        CanAnswer::NotThisText => {
            app.model.error(not_heard(&file_path));
            return None;
        }
        CanAnswer::Yes => {}
    }
    let Some(at) = editor.caret() else {
        app.model.error("put the caret on the code first");
        return None;
    };
    if let Some(behind) = tab_behind_its_server(&app.model) {
        if since.elapsed() < HEARD_WITHIN {
            ctx.request_repaint_after(LOOKS_AGAIN_IN);
            return Some(waiting);
        }
        app.model.error(not_heard(&behind));
        return None;
    }

    let told = what_the_tabs_hold(&app.model);
    let for_call = session_id.clone();
    let for_ask = file_path.clone();
    app.tasks.spawn_keyed(
        Some("code-actions".to_string()),
        move |backend| SessionLanguages::new(backend, &for_call).code_actions(&for_ask, at),
        move |model, result| {
            model.code_acting = None;
            match result {
                Ok(actions) if actions.is_empty() => {
                    model.info(format!("nothing to do at the caret in {file_path}"));
                }
                Ok(actions) => {
                    model.palette.show_code_actions();
                    model.code_acting = Some(CodeActing::Choosing {
                        session_id,
                        told,
                        actions,
                    });
                }
                Err(error) => model.error(format!(
                    "could not ask for the code actions in {file_path}: {error}"
                )),
            }
        },
    );
    Some(CodeActing::Asking)
}

/// Carry out the action picked from the palette, by its place in the list.
pub(crate) fn apply(app: &mut App, index: usize) {
    let Some(CodeActing::Choosing {
        session_id,
        told,
        actions,
    }) = app.model.code_acting.take()
    else {
        return;
    };
    let Some(action) = actions.into_iter().nth(index) else {
        return;
    };
    let title = action.title;
    let said_title = title.clone();
    put_in(
        app,
        session_id,
        &told,
        action.files,
        format!("\"{title}\""),
        move |counts| {
            format!(
                "{said_title}: {} in {}{}",
                counted(counts.places, "change"),
                counted(counts.files, "file"),
                counts.unsaved_note()
            )
        },
    );
}

/// What a row of the palette says about an action under its title: what kind of action it is,
/// and where it reaches.
pub(crate) fn about(action: &LspCodeAction) -> String {
    let reaches = match &action.files[..] {
        [one] => one.file_path.clone(),
        many => counted(many.len(), "file"),
    };
    match (&action.kind, action.preferred) {
        (Some(kind), true) => format!("{kind} · preferred · {reaches}"),
        (Some(kind), false) => format!("{kind} · {reaches}"),
        (None, true) => format!("preferred · {reaches}"),
        (None, false) => reaches,
    }
}
