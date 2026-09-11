//! The places a language server names for the name at a file tab's caret: where it is
//! defined, where its type is, where it is implemented, and everywhere it is used.
//!
//! Asked from the tab's context menu, the palette and the function keys - the keyboard's and a
//! menu's way to what ⌘-click is for a definition. A definition is still
//! [`crate::native::definition`]'s, which already knows how to land on one and what to say
//! when there is none; the other three are here.
//!
//! One place is gone to, the way a definition is - except for the uses of a name. A list of
//! one use is still the answer to "where is this used", and opening it would read as a jump to
//! the declaration made by mistake. Anything else is a list in the palette, one row per place,
//! reading what its line says, filtered by what is typed.

use egui_frames::PaneId;
use egui_moon_code_ide::{
    LanguageSource, LspLocation, LspPlaces, LspPosition, asks_about, still_starting,
};

use crate::native::{
    app::App,
    language_source::SessionLanguages,
    palette::CommandAction,
    panes::{OpenAt, OpenPaneRequest, Pane},
};

/// One kind of place, as the window offers and reads it.
pub(crate) struct PlaceKind {
    pub(crate) which: LspPlaces,
    /// The command, in the palette and in the tab's context menu.
    pub(crate) command: &'static str,
    /// What the palette says the command does.
    pub(crate) about: &'static str,
    /// What a list of these is of: "uses of" greet.
    pub(crate) listed: &'static str,
    /// Whether a single place of this kind is gone to rather than listed.
    pub(crate) one_is_gone_to: bool,
}

/// Every kind of place the window asks for, in the order the menu and the palette offer them.
pub(crate) const KINDS: &[PlaceKind] = &[
    PlaceKind {
        which: LspPlaces::Definition,
        command: "go to definition",
        about: "Open where the name at the caret is defined",
        listed: "definitions of",
        one_is_gone_to: true,
    },
    PlaceKind {
        which: LspPlaces::TypeDefinition,
        command: "go to type definition",
        about: "Open where the type of the name at the caret is defined",
        listed: "type definitions of",
        one_is_gone_to: true,
    },
    PlaceKind {
        which: LspPlaces::Implementation,
        command: "go to implementation",
        about: "Open where the name at the caret is implemented",
        listed: "implementations of",
        one_is_gone_to: true,
    },
    PlaceKind {
        which: LspPlaces::References,
        command: "find references",
        about: "List everywhere the name at the caret is used",
        listed: "uses of",
        one_is_gone_to: false,
    },
];

/// The row for one kind of place.
pub(crate) fn kind_of(which: LspPlaces) -> &'static PlaceKind {
    KINDS
        .iter()
        .find(|kind| kind.which == which)
        .expect("every kind of place has a row in the table")
}

/// What a question came back with, waiting for a frame that can open a pane or the palette.
pub(crate) struct FoundPlaces {
    pub(crate) which: LspPlaces,
    /// The name that was asked about, marked in the file a row opens.
    pub(crate) word: String,
    pub(crate) session_id: String,
    /// Never empty: a question with no places to show says so instead of coming here.
    pub(crate) places: Vec<LspLocation>,
}

/// Whether the tab in front is a file with a language server behind it, which is when the
/// palette offers these.
pub(crate) fn front_tab_finds_places(app: &App) -> bool {
    app.active_pane().is_some_and(|(pane_id, pane)| {
        matches!(pane, Pane::File { .. })
            && app
                .model
                .file_editors
                .get(&pane_id)
                .is_some_and(|editor| editor.offers_places())
    })
}

/// Ask about the name at the caret of the tab in front - the function keys, and the palette.
pub(crate) fn ask_in_front(app: &mut App, which: LspPlaces) {
    let Some((pane_id, Pane::File { session_id, .. })) = app.active_pane() else {
        app.model.error(format!(
            "{} works on the name at the caret of a file tab",
            kind_of(which).command
        ));
        return;
    };
    let session_id = session_id.clone();
    ask(app, pane_id, &session_id, which);
}

/// Ask the server behind a file tab for the places of one kind of the name at its caret.
pub(crate) fn ask(app: &mut App, pane_id: PaneId, session_id: &str, which: LspPlaces) {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    let kind = kind_of(which);
    let file_path = editor.file_path.clone();
    if !editor.offers_places() {
        app.model.error(format!(
            "no language server serves {file_path}, so the {} a name cannot be found",
            kind.listed
        ));
        return;
    }
    let Some(word) = editor.caret_word() else {
        app.model.error("put the caret on a name first");
        return;
    };
    if which == LspPlaces::Definition {
        crate::native::definition::look_up(app, pane_id, session_id, word);
        return;
    }

    let at = LspPosition {
        line: word.at.line,
        column: word.at.column,
    };
    let name = word.text;
    let session_id = session_id.to_string();
    let for_call = session_id.clone();
    let for_ask = file_path.clone();
    app.tasks.spawn_keyed(
        Some(format!("places:{pane_id}")),
        move |backend| {
            let languages = SessionLanguages::new(backend, &for_call);
            // A status that could not be had is a file with no server - see
            // [`SessionLanguages::status`].
            let asks = asks_about(languages.status(&for_ask));
            if !asks.asks() {
                return Ok(None);
            }
            let places = languages.places(&for_ask, at, which)?;
            Ok(Some((places, asks.an_empty_answer_is_only_the_wait())))
        },
        move |model, result| match result {
            Err(error) => model.error(format!(
                "could not find the {} {name}: {error}",
                kind.listed
            )),
            Ok(None) => model.error(format!(
                "no language server serves {file_path}, so the {} {name} cannot be found",
                kind.listed
            )),
            Ok(Some((places, _))) if !places.is_empty() => {
                model.places_found = Some(FoundPlaces {
                    which,
                    word: name,
                    session_id,
                    places,
                });
            }
            // Nothing from a server that has not read the project is the wait, not an answer.
            Ok(Some((_, true))) => model.error(still_starting(&file_path)),
            Ok(Some((_, false))) => model.error(format!(
                "the language server found no {} {name}",
                kind.listed
            )),
        },
    );
}

/// Go to, or list, what a question found - on a frame where a pane can be opened, which is
/// after the tree holding the panes has been drawn.
pub(crate) fn follow(app: &mut App) {
    // Something else is opening this frame; the answer keeps until the next one.
    if app.pending_action.is_some() {
        return;
    }
    let Some(found) = app.model.places_found.take() else {
        return;
    };
    match &found.places[..] {
        [one] if kind_of(found.which).one_is_gone_to => {
            app.pending_action = Some(open_at(&found.session_id, one, &found.word));
        }
        _ => app.model.palette.show_places(found),
    }
}

/// Open one place, with the name that was asked about marked on its line.
pub(crate) fn open_at(session_id: &str, place: &LspLocation, word: &str) -> CommandAction {
    CommandAction::OpenPane(OpenPaneRequest::File {
        session_id: session_id.to_string(),
        file_path: place.file_path.clone(),
        at: Some(OpenAt {
            line: place.line_number,
            query: word.to_string(),
        }),
    })
}
