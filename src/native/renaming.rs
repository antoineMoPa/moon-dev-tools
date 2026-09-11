//! Rename the name at a file tab's caret, everywhere the language server behind the file knows
//! it is used.
//!
//! Four steps, with a frame or more between each:
//!
//! 1. F2, the palette's "rename symbol" or the tab's context menu asks the server what is at
//!    the caret - once the server has heard what the tab is showing, since the caret is a place
//!    in *that* text and a question about a word typed a moment ago would be about another.
//! 2. The palette opens on the name the server gave, selected, so typing replaces it.
//! 3. Enter asks for the rename - once every tab's text has reached its server, since the
//!    places in the answer are places in the texts that server has.
//! 4. The answer lands. A file open in a tab takes its edits into the tab's buffer, unsaved,
//!    the way typing would: the tab is what the person is looking at, and its unsaved edits are
//!    theirs to keep or throw away. A file nobody has open is written.
//!
//! Step 4 refuses the whole rename rather than doing part of it when a tab's text is no longer
//! the text the edits were worked out against - typed into while the rename was out, or opened
//! since. Half a rename is a project that does not build, and an edit landing on whatever sits
//! at its line and column now is worse than none.

use std::{collections::HashMap, time::Instant};

use egui_frames::PaneId;
use egui_moon_code_ide::{CanAnswer, LanguageSource, LspFileEdit, LspPosition, still_starting};

use crate::native::{
    app::App,
    language_source::SessionLanguages,
    model::Model,
    panes::Pane,
    workspace_edits::{
        Counts, HEARD_WITHIN, LOOKS_AGAIN_IN, counted, not_heard, put_in, tab_behind_its_server,
        what_the_tabs_hold,
    },
};

/// The name being renamed: where it is, and what the server says it is called.
pub(crate) struct RenameTarget {
    pub(crate) session_id: String,
    pub(crate) file_path: String,
    pub(crate) at: LspPosition,
    /// The name as the server would have it renamed, which is what the palette offers to be
    /// typed over.
    pub(crate) name: String,
}

/// Where a rename is, from the key that asked for it to its edits landing. One at a time for
/// the window: the palette has one line to type a name into, and two renames out at once
/// would each be worked out against texts the other is about to change.
pub(crate) enum Renaming {
    /// Waiting for the server to hear what the tab is showing before asking what is at its
    /// caret.
    WaitingToAsk {
        session_id: String,
        pane_id: PaneId,
        since: Instant,
    },
    /// The server is being asked what is at the caret.
    Asking,
    /// The palette is open on the name, waiting for a new one to be typed.
    Naming {
        target: RenameTarget,
        pane_id: PaneId,
        /// The tab's text the server answered about, which the caret is a place in. A tab
        /// typed into since has a caret somewhere else, and the rename is refused.
        text: String,
    },
    /// Waiting for every tab's text to reach its server before the rename is asked for.
    WaitingToRename {
        target: RenameTarget,
        pane_id: PaneId,
        text: String,
        new_name: String,
        since: Instant,
    },
    /// The rename is out.
    Renaming,
    /// The answer is in, waiting for a frame that can put it into the tabs and write the rest.
    Landed {
        target: RenameTarget,
        new_name: String,
        /// What every tab held when the rename was asked for, by file - the texts the server
        /// worked the edits out against.
        told: HashMap<String, String>,
        files: Vec<LspFileEdit>,
    },
}

impl Renaming {
    /// Which step the rename is at, for a test that has to say where one stopped.
    #[cfg(test)]
    pub(crate) fn step_for_test(&self) -> &'static str {
        match self {
            Renaming::WaitingToAsk { .. } => "waiting to ask what is at the caret",
            Renaming::Asking => "asking what is at the caret",
            Renaming::Naming { .. } => "naming",
            Renaming::WaitingToRename { .. } => "waiting to rename",
            Renaming::Renaming => "renaming",
            Renaming::Landed { .. } => "landed",
        }
    }
}

/// The name the palette is asking a new name for, while it is.
pub(crate) fn naming(model: &Model) -> Option<&RenameTarget> {
    match &model.renaming {
        Some(Renaming::Naming { target, .. }) => Some(target),
        _ => None,
    }
}

/// Whether the tab in front is a file whose server can rename things, which is when the
/// palette offers "rename symbol".
pub(crate) fn front_tab_renames(app: &App) -> bool {
    app.active_pane().is_some_and(|(pane_id, pane)| {
        matches!(pane, Pane::File { .. })
            && app
                .model
                .file_editors
                .get(&pane_id)
                .is_some_and(|editor| editor.offers_edits())
    })
}

/// Rename the name at the caret of the tab in front - F2, and the palette's "rename symbol".
pub(crate) fn start_in_front(app: &mut App) {
    let Some((pane_id, Pane::File { session_id, .. })) = app.active_pane() else {
        app.model
            .error("rename works on the name at the caret of a file tab");
        return;
    };
    let session_id = session_id.clone();
    start(app, pane_id, &session_id);
}

/// Rename the name at the caret of one file tab.
///
/// Says why not, out loud, for every way this cannot go ahead: a rename is a direct request,
/// and a key that did nothing reads as a broken key.
pub(crate) fn start(app: &mut App, pane_id: PaneId, session_id: &str) {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    let file_path = &editor.file_path;
    let refused = if app.model.renaming.is_some() {
        Some("a rename is already under way".to_string())
    } else if editor.is_outside_the_repo() {
        Some(format!(
            "{file_path} is outside the repo, and can only be read"
        ))
    } else if !editor.offers_edits() {
        Some(format!(
            "no language server serves {file_path}, so nothing in it can be renamed"
        ))
    } else if editor.caret().is_none() {
        Some("put the caret on the name to rename first".to_string())
    } else {
        None
    };
    if let Some(said) = refused {
        app.model.error(said);
        return;
    }
    app.model.renaming = Some(Renaming::WaitingToAsk {
        session_id: session_id.to_string(),
        pane_id,
        since: Instant::now(),
    });
}

/// Rename to the name typed into the palette - what its one row runs.
pub(crate) fn rename_to(app: &mut App, new_name: String) {
    app.model.renaming = match app.model.renaming.take() {
        Some(Renaming::Naming {
            target,
            pane_id,
            text,
        }) => Some(Renaming::WaitingToRename {
            target,
            pane_id,
            text,
            new_name,
            since: Instant::now(),
        }),
        other => other,
    };
}

/// Move a rename along, once a frame.
///
/// Called after the frame's deferred action has run, so a name picked in the palette this
/// frame is already waiting to be renamed rather than read as a palette put away.
pub(crate) fn follow(app: &mut App, ctx: &egui::Context) {
    let Some(renaming) = app.model.renaming.take() else {
        return;
    };
    app.model.renaming = match renaming {
        Renaming::WaitingToAsk {
            session_id,
            pane_id,
            since,
        } => ask_what_is_at_the_caret(app, ctx, session_id, pane_id, since),
        // The palette was put away without a name being picked, which is the rename called
        // off.
        Renaming::Naming { .. }
            if !(app.model.palette.open
                && app.model.palette.mode == crate::native::palette::PaletteMode::Rename) =>
        {
            None
        }
        Renaming::WaitingToRename {
            target,
            pane_id,
            text,
            new_name,
            since,
        } => ask_for_the_rename(app, ctx, target, pane_id, text, new_name, since),
        Renaming::Landed {
            target,
            new_name,
            told,
            files,
        } => {
            land(app, target, new_name, &told, files);
            None
        }
        waiting => Some(waiting),
    };
}

/// Ask the server what is at the caret, once it has heard the text the caret is in.
fn ask_what_is_at_the_caret(
    app: &mut App,
    ctx: &egui::Context,
    session_id: String,
    pane_id: PaneId,
    since: Instant,
) -> Option<Renaming> {
    // The tab was closed while the rename waited, and took the rename with it.
    let editor = app.model.file_editors.get(&pane_id)?;
    let file_path = editor.file_path.clone();
    match editor.server_heard().can_answer_about(editor.text()) {
        CanAnswer::StillReadingTheProject => {
            app.model.error(still_starting(&file_path));
            return None;
        }
        CanAnswer::NotThisText if since.elapsed() < HEARD_WITHIN => {
            ctx.request_repaint_after(LOOKS_AGAIN_IN);
            return Some(Renaming::WaitingToAsk {
                session_id,
                pane_id,
                since,
            });
        }
        CanAnswer::NotThisText => {
            app.model.error(not_heard(&file_path));
            return None;
        }
        CanAnswer::Yes => {}
    }
    let Some(at) = editor.caret() else {
        app.model.error("put the caret on the name to rename first");
        return None;
    };
    let text = editor.text().to_string();

    let for_call = session_id.clone();
    let for_ask = file_path.clone();
    app.tasks.spawn_keyed(
        Some("rename".to_string()),
        move |backend| SessionLanguages::new(backend, &for_call).prepare_rename(&for_ask, at),
        move |model, result| {
            model.renaming = None;
            match result {
                Ok(Some(name)) => {
                    model.palette.show_rename(&name);
                    model.renaming = Some(Renaming::Naming {
                        target: RenameTarget {
                            session_id,
                            file_path,
                            at,
                            name,
                        },
                        pane_id,
                        text,
                    });
                }
                Ok(None) => model.error(format!(
                    "nothing at the caret in {file_path} can be renamed"
                )),
                Err(error) => model.error(format!("could not rename in {file_path}: {error}")),
            }
        },
    );
    Some(Renaming::Asking)
}

/// Ask for the rename, once every tab's text has reached its server.
fn ask_for_the_rename(
    app: &mut App,
    ctx: &egui::Context,
    target: RenameTarget,
    pane_id: PaneId,
    text: String,
    new_name: String,
    since: Instant,
) -> Option<Renaming> {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        app.model.error(format!(
            "the tab on {} was closed, so {} was not renamed",
            target.file_path, target.name
        ));
        return None;
    };
    if editor.text() != text {
        app.model.error(format!(
            "{} changed after {} was picked, so nothing was renamed",
            target.file_path, target.name
        ));
        return None;
    }
    if let Some(behind) = tab_behind_its_server(&app.model) {
        if since.elapsed() < HEARD_WITHIN {
            ctx.request_repaint_after(LOOKS_AGAIN_IN);
            return Some(Renaming::WaitingToRename {
                target,
                pane_id,
                text,
                new_name,
                since,
            });
        }
        app.model.error(not_heard(&behind));
        return None;
    }

    let told = what_the_tabs_hold(&app.model);
    let for_call = target.session_id.clone();
    let for_ask = target.file_path.clone();
    let for_name = new_name.clone();
    let at = target.at;
    app.tasks.spawn_keyed(
        Some("rename".to_string()),
        move |backend| SessionLanguages::new(backend, &for_call).rename(&for_ask, at, &for_name),
        move |model, result| match result {
            Ok(files) if files.is_empty() => {
                model.renaming = None;
                model.error(format!(
                    "the language server found nothing to rename for {}",
                    target.name
                ));
            }
            Ok(files) => {
                model.renaming = Some(Renaming::Landed {
                    target,
                    new_name,
                    told,
                    files,
                });
            }
            Err(error) => {
                model.renaming = None;
                model.error(format!("could not rename {}: {error}", target.name));
            }
        },
    );
    Some(Renaming::Renaming)
}

/// Put the edits into the tabs showing their files, and write the files nobody has open - see
/// [`crate::native::workspace_edits`].
fn land(
    app: &mut App,
    target: RenameTarget,
    new_name: String,
    told: &HashMap<String, String>,
    files: Vec<LspFileEdit>,
) {
    let name = target.name.clone();
    let to = new_name.clone();
    put_in(
        app,
        target.session_id,
        told,
        files,
        format!("renaming {} to {new_name}", target.name),
        move |counts| said_about(&name, &to, counts),
    );
}

/// The line a finished rename leaves.
fn said_about(name: &str, new_name: &str, counts: &Counts) -> String {
    format!(
        "renamed {name} to {new_name}: {} in {}{}",
        counted(counts.places, "place"),
        counted(counts.files, "file"),
        counts.unsaved_note()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_rename_says_how_much_it_changed_and_what_is_left_unsaved() {
        assert_eq!(
            said_about(
                "greet",
                "hello",
                &Counts {
                    places: 1,
                    files: 1,
                    files_in_tabs: 0
                }
            ),
            "renamed greet to hello: 1 place in 1 file"
        );
        assert_eq!(
            said_about(
                "greet",
                "hello",
                &Counts {
                    places: 7,
                    files: 3,
                    files_in_tabs: 2
                }
            ),
            "renamed greet to hello: 7 places in 3 files - 2 files open in tabs, edited there and not saved yet"
        );
    }
}
