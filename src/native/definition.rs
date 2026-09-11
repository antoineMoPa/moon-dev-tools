//! ⌘-click a name in a file and land on where it is defined.
//!
//! The language server answers, and only the language server. It has parsed the project and
//! knows which `new` of the forty in the repo this one is, which is a thing no amount of
//! reading lines can work out - so where there is no server behind the file the click says so
//! plainly rather than guessing, and where there is one its answer is taken at its word.
//!
//! A server that has not finished reading the project is asked anyway. rust-analyzer takes the
//! better part of a minute over this repo from cold, and a click that did nothing at all for
//! that minute is a feature nobody would believe in; asking early is safe, because the one
//! refusal such a server gives is retried inside [`moon_lsp`]. What the wait costs is only how
//! an empty answer is allowed to read: from a server that has read the project, nothing means
//! the name is defined nowhere, and from one that has not it means "not yet". Those two arrive
//! as the same empty list and must never read the same - see
//! [`egui_moon_code_ide::AsksAbout`], which is where that distinction is kept.
//!
//! A server's answer is often not in the repo at all. A definition in a Rust project lands in
//! `~/.cargo/registry` or in the standard library about as often as it lands in the work being
//! reviewed, and landing there is the whole point of asking - so the pane opens what the jump
//! landed on, by the absolute path the server named. That is the one kind of file this window
//! may read from outside the repo, and it is readable only because the server named it: see
//! [`crate::lsp::FilesNamedOutsideTheRepo`] for what that costs and what it refuses. Such a
//! file opens read-only, and the pane says so rather than looking like a file of the repo.
//!
//! Several places is a real answer - a trait method and the impls of it - and the first of
//! them is what opens, with a line saying how many there were. The alternative was a list to
//! pick from, and a list is not worth its weight here: a server hands back one place almost
//! every time, orders what it hands back, and names files and lines with no text to read a row
//! by. Landing and reading the tab is faster than reading a list of two.
//!
//! Two places take the click: a file pane, and a row of a review's diff - which is where most
//! of the reading in this window actually happens, so a jump that only worked in the file pane
//! was a feature nobody ever met. Both end in the same question and the same landing. What
//! differs is the document: a file pane's buffer is already open on the server, kept up by
//! [`crate::native::lsp_document`], while a review has never told the server anything, so its
//! click opens the file with what the working tree holds, asks, and hands the document back -
//! through the same reference counting the panes use, so a document a pane is showing is never
//! closed underneath it.

use egui_frames::PaneId;
use egui_moon_code_ide::{AsksAbout, LanguageSource, asks_about};
use egui_moon_editor::Word;

use crate::{
    api::{LspLocation, LspPosition},
    backend::Backend,
    native::{
        app::App,
        language_source::SessionLanguages,
        palette::CommandAction,
        panes::{OpenAt, OpenPaneRequest},
    },
};

/// What the lookup came back with, before a frame that can act on it has read it.
enum Answer {
    /// Nothing serves this file, so there was nobody to ask. Not a fault - it is the standing
    /// state of markdown, shell, SQL and every language nobody installed a server for - but it
    /// is said out loud, because a click is a direct request and silence reads as a miss.
    NoServer,
    /// A server answered. Everywhere it says the name is defined, which can be nowhere.
    Places(Vec<LspLocation>),
    /// It answered with nothing while it was still reading the project, which is the wait
    /// showing through rather than an answer about the name.
    StillStarting,
    /// The line the click was made on does not hold the name in the file as it stands now. Only
    /// a review can see this: its rows are of the version being reviewed, and a review of an
    /// older commit is of a file the working tree has moved on from.
    NotThatLineAnyMore,
}

/// A lookup that has come back, waiting for a frame that can act on it.
///
/// It sits on whatever the ⌘-click was made in, because that is what it belongs to: two file
/// panes can each have a lookup out, and each one's answer is its own, and so can a review
/// being read beside them.
pub(crate) struct LookedUp {
    /// The name that was clicked, kept to mark in the file that is landed on and to say what
    /// was being looked for when nothing turned up.
    word: String,
    /// The file the click was made in. It names the server that is busy and the file nothing
    /// serves, and it is the document to hand back when this lookup is the only thing that
    /// opened one.
    file_path: String,
    /// Whether this lookup is why the server has the file open at all, which is true of a
    /// review's click and never of a file pane's - see [`crate::native::lsp_document`].
    holds_the_document: bool,
    /// Where the answer sends the window, worked out where it arrived: the reading of it
    /// needs nothing from the frame, and the frame that opens the pane should only open it.
    landing: Landing,
}

/// Where a finished lookup sends the window.
enum Landing {
    /// Nowhere, and why not: the three ways a lookup ends without a place to go.
    Nowhere(WhyNot),
    /// The place to open, and how many others the server named beside it.
    Place { place: LspLocation, others: usize },
}

/// Why a lookup has nowhere to send the window. Each of these is said, and each says something
/// different: what a person does next about them is not the same.
enum WhyNot {
    /// No language server serves the file, so nothing was asked.
    NoServer,
    /// A server was asked and had not read the project yet.
    StillStarting,
    /// A server that has read the project says the name is defined nowhere it can see.
    NoDefinition,
    /// The row clicked is of a version of the file the working tree has moved on from.
    NotThatLineAnyMore,
}

/// Look the clicked name up with the server behind the file the click was made in.
///
/// A blocking question to a server, on a repo that may be on another machine, so it goes to a
/// worker thread the way reading a file does. Keyed by pane, so ⌘-clicking twice in a row looks
/// up the second name rather than racing the first.
pub(crate) fn look_up(app: &mut App, pane_id: PaneId, session_id: &str, word: Word) {
    // The file the click was in, which is the file the server is asked about. A pane that
    // has gone has no name to look up.
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    let file_path = editor.file_path.clone();
    let asks_a_server = editor.asks_language_servers();

    let for_call = session_id.to_string();
    let for_ask = file_path.clone();
    // Straight from the editor: the line counts from zero and the column is bytes into that
    // line, which is what [`LspPosition`] is. What the server counts in is settled inside
    // `src/lsp`, and there is nothing to convert here.
    let at = LspPosition {
        line: word.at.line,
        column: word.at.column,
    };
    let word = word.text;

    app.tasks.spawn_keyed(
        Some(format!("definition:{pane_id}")),
        move |backend| answer_for(backend, &for_call, &for_ask, asks_a_server, at),
        move |model, result| {
            let answer = match result {
                Ok(answer) => answer,
                Err(error) => {
                    model.error(format!("could not look up {word}: {error}"));
                    return;
                }
            };
            // The pane may have been closed while the lookup was out, and the answer belongs
            // to nobody else.
            let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                return;
            };
            editor.looking_up = Some(LookedUp {
                word,
                file_path,
                // The pane's buffer is open on the server and stays open: it is the pane's
                // document, kept up as it is typed into, and nothing about a click owns it.
                holds_the_document: false,
                landing: landing_of(answer),
            });
        },
    );
}

/// Look up a name ⌘-clicked on a row of a review's diff.
///
/// A review has never told a language server about the file whose rows it is showing - opening
/// a document belongs to a pane that holds text - so this opens it, with what the working tree
/// holds, which is what the rows of a review of the working tree are of. The document is handed
/// back once the answer has landed, unless a file pane has it open: a pane's copy is its
/// buffer, edits and all, and this must neither talk over it nor close it underneath it. Both
/// halves of that are [`crate::native::lsp_document`]'s, which is where the panes are counted.
///
/// `at` is the place in the file as it stands now - a line of the diff that exists in it, and
/// bytes into that line. A removed row is text the file does not contain any more, so there is
/// no such place for it and the click never gets this far: see
/// `crate::native::review::hunks::actions::jump_to_definition`.
///
/// Keyed by review, so ⌘-clicking twice in a row looks up the second name rather than racing
/// the first - the same as a file pane, and for the same reason.
pub(crate) fn look_up_in_review(
    app: &mut App,
    session_id: &str,
    file_path: String,
    at: LspPosition,
    word: String,
) {
    let asks_a_server = app.asks_language_servers;
    // Worked out here rather than on the worker, because it is the model that knows: a pane
    // showing this file has already told the server what is in it.
    let a_pane_has_it = app.a_pane_has_the_document_open(&file_path);

    let for_call = session_id.to_string();
    let for_park = session_id.to_string();
    let for_ask = file_path.clone();
    let name = word.clone();

    app.tasks.spawn_keyed(
        Some(format!("definition:review:{session_id}")),
        move |backend| {
            answer_for_a_review(
                backend,
                &for_call,
                &for_ask,
                at,
                &name,
                asks_a_server,
                a_pane_has_it,
            )
        },
        move |model, result| {
            let (answer, holds_the_document) = match result {
                Ok(answered) => answered,
                Err(error) => {
                    model.error(format!("could not look up {word}: {error}"));
                    return;
                }
            };
            model.review(&for_park).looking_up = Some(LookedUp {
                word,
                file_path,
                holds_the_document,
                landing: landing_of(answer),
            });
        },
    );
}

/// Ask the server behind a file pane's buffer, which it already has open.
fn answer_for(
    backend: &dyn Backend,
    session_id: &str,
    file_path: &str,
    asks_a_server: bool,
    at: LspPosition,
) -> anyhow::Result<Answer> {
    // A window with its language servers switched off is every window but the one `run`
    // opens - see [`App::asks_language_servers`] - and it has nobody to ask.
    if !asks_a_server {
        return Ok(Answer::NoServer);
    }
    let languages = SessionLanguages::new(backend, session_id);
    // A status that could not be had is a file with no server, as far as a click is
    // concerned - see [`SessionLanguages::status`].
    let asks = asks_about(languages.status(file_path));
    if !asks.asks() {
        return Ok(Answer::NoServer);
    }
    ask_where(&languages, file_path, at, asks)
}

/// The same for a row of a review's diff, which has to open the document first.
///
/// Says whether it left a document open, so the frame that reads the answer knows there is one
/// to hand back. An error hands it back here instead: a question that failed must not leave a
/// document open behind the caller's back, and there is no answer for a frame to act on.
fn answer_for_a_review(
    backend: &dyn Backend,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
    word: &str,
    asks_a_server: bool,
    a_pane_has_it: bool,
) -> anyhow::Result<(Answer, bool)> {
    if !asks_a_server {
        return Ok((Answer::NoServer, false));
    }
    let languages = SessionLanguages::new(backend, session_id);
    let asks = asks_about(languages.status(file_path));
    if !asks.asks() {
        return Ok((Answer::NoServer, false));
    }
    if a_pane_has_it {
        // The pane's copy is the truer one - it is what is on screen in that pane, unsaved
        // edits and all - so it is left exactly as it is and the question goes as it stands.
        return Ok((ask_where(&languages, file_path, at, asks)?, false));
    }

    let text = backend.file_content(session_id, file_path)?.content;
    // The row is of the version being reviewed and this text is what the file holds now. On a
    // review of an older commit those can be different files, and asking anyway would land
    // the click on whatever happens to sit at that line number today.
    if !line_holds(&text, at.line, word) {
        return Ok((Answer::NotThatLineAnyMore, false));
    }
    languages.did_open(file_path, &text)?;
    match ask_where(&languages, file_path, at, asks) {
        Ok(answer) => Ok((answer, true)),
        Err(error) => {
            let _ = languages.did_close(file_path);
            Err(error)
        }
    }
}

/// Put the question, and read an empty answer the way the state of the server says to.
fn ask_where(
    languages: &SessionLanguages<'_>,
    file_path: &str,
    at: LspPosition,
    asks: AsksAbout,
) -> anyhow::Result<Answer> {
    let places = languages.definition(file_path, at)?;
    if places.is_empty() && asks.an_empty_answer_is_only_the_wait() {
        return Ok(Answer::StillStarting);
    }
    Ok(Answer::Places(places))
}

/// Whether the line of `text` counted from zero holds `word`.
fn line_holds(text: &str, line: usize, word: &str) -> bool {
    text.lines()
        .nth(line)
        .is_some_and(|holds| holds.contains(word))
}

/// Where an answer sends the window.
///
/// A server that answered at all is taken at its word - no ranking, no guessing. Several
/// places is the several the language really has, a trait method and the impls of it, and the
/// first is the one the server put first.
fn landing_of(answer: Answer) -> Landing {
    let places = match answer {
        Answer::NoServer => return Landing::Nowhere(WhyNot::NoServer),
        Answer::StillStarting => return Landing::Nowhere(WhyNot::StillStarting),
        Answer::NotThatLineAnyMore => return Landing::Nowhere(WhyNot::NotThatLineAnyMore),
        Answer::Places(places) => places,
    };
    let others = places.len().saturating_sub(1);
    match places.into_iter().next() {
        Some(place) => Landing::Place { place, others },
        None => Landing::Nowhere(WhyNot::NoDefinition),
    }
}

/// Act on a lookup that has come back, on a frame where a pane can be opened.
///
/// Called as the pane draws rather than from where the answer arrives, because opening a pane
/// is deferred to the end of the frame everywhere in this window - the tree holding the pane
/// being drawn must not be rebuilt underneath it.
pub(crate) fn follow(app: &mut App, pane_id: PaneId, session_id: &str) {
    // Something else is already opening this frame. The answer keeps until the next one
    // rather than being dropped on the floor.
    if app.pending_action.is_some() {
        return;
    }
    let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
        return;
    };
    let Some(looked_up) = editor.looking_up.take() else {
        return;
    };
    land(app, session_id, looked_up);
}

/// The same, for a name ⌘-clicked on a row of a review's diff.
///
/// Called as the review draws, for the same reason the file pane's is: the pane the jump opens
/// cannot be opened from underneath the tree that is drawing.
pub(crate) fn follow_in_review(app: &mut App, session_id: &str) {
    if app.pending_action.is_some() {
        return;
    }
    let Some(looked_up) = app.model.review(session_id).looking_up.take() else {
        return;
    };
    land(app, session_id, looked_up);
}

/// Send the window where the answer says, whichever kind of click asked.
fn land(app: &mut App, session_id: &str, looked_up: LookedUp) {
    let LookedUp {
        word,
        file_path,
        holds_the_document,
        landing,
    } = looked_up;
    // The document this lookup opened only so the question could be put, given back now the
    // answer is in hand.
    if holds_the_document {
        app.close_document_asked_about(&file_path, session_id);
    }
    match landing {
        // Silence would read as the click having missed, so each of these says so - but for
        // the wait, which the status bar is already reading out.
        Landing::Nowhere(why) => {
            if let Some(said) = said_about(why, &word, &file_path) {
                app.model.error(said);
            }
        }
        Landing::Place { place, others } => {
            if others > 0 {
                app.model.info(format!(
                    "{} places define {word} - opened the first",
                    others + 1
                ));
            }
            app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::File {
                session_id: session_id.to_string(),
                file_path: place.file_path,
                at: Some(OpenAt {
                    line: place.line_number,
                    query: word,
                }),
            }));
        }
    }
}

/// What a lookup with nowhere to go says. Three sentences, because a person's next move about
/// each of them is different: install a server, believe it, or read the file itself.
///
/// A server still reading the project says nothing here. The status bar is already saying
/// what it is doing and how far through it is - see [`crate::native::status_bar`] - and a toast
/// repeating it on every ⌘-click only covers the code being read.
fn said_about(why: WhyNot, word: &str, file_path: &str) -> Option<String> {
    match why {
        WhyNot::NoServer => Some(format!(
            "no language server serves {file_path}, so {word} cannot be looked up"
        )),
        WhyNot::StillStarting => None,
        WhyNot::NoDefinition => Some(format!("the language server has no definition for {word}")),
        WhyNot::NotThatLineAnyMore => Some(format!(
            "{word} is not on that line of {file_path} any more - this review is of an older version"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(file_path: &str, line_number: usize) -> LspLocation {
        LspLocation {
            file_path: file_path.to_string(),
            line_number,
            line_text: None,
        }
    }

    /// One place is a jump. Several is a jump to the first with a count said out loud, because
    /// a server that names several has ordered them and a list of two is slower to read than
    /// the tab it would open.
    #[test]
    fn one_place_is_a_jump_and_several_is_a_jump_to_the_first_of_them() {
        let one = landing_of(Answer::Places(vec![place("src/lib.rs", 12)]));
        assert!(matches!(one, Landing::Place { place, others }
            if place.file_path == "src/lib.rs" && place.line_number == 12 && others == 0));

        let two = landing_of(Answer::Places(vec![
            place("src/one.rs", 3),
            place("src/two.rs", 4),
        ]));
        assert!(matches!(two, Landing::Place { place, others }
            if place.file_path == "src/one.rs" && others == 1));
    }

    /// The distinction the whole of this hangs on: an empty answer from a server that has read
    /// the project means the name is defined nowhere, and one from a server that has not means
    /// the wait - which is the status bar's to say, not a toast's.
    #[test]
    fn nothing_from_a_ready_server_is_said_and_the_wait_is_left_to_the_status_bar() {
        let Landing::Nowhere(nowhere) = landing_of(Answer::Places(Vec::new())) else {
            panic!("a ready server with nothing to say has nowhere to send the window");
        };
        assert_eq!(
            said_about(nowhere, "greet", "src/main.rs").as_deref(),
            Some("the language server has no definition for greet")
        );

        let Landing::Nowhere(waiting) = landing_of(Answer::StillStarting) else {
            panic!("a starting server with nothing to say has nowhere to send the window");
        };
        assert_eq!(
            said_about(waiting, "greet", "src/main.rs"),
            None,
            "the wait must not read as an answer, and the status bar is already saying it"
        );
    }

    /// A file nothing serves says so, and names the file: with no search behind the click any
    /// more, that is the whole of the answer and it has to be a legible one.
    #[test]
    fn a_file_no_language_server_serves_says_so_and_names_the_file() {
        let Landing::Nowhere(nowhere) = landing_of(Answer::NoServer) else {
            panic!("a file nothing serves has nowhere to send the window");
        };
        assert_eq!(
            said_about(nowhere, "greet", "notes/plan.txt").as_deref(),
            Some("no language server serves notes/plan.txt, so greet cannot be looked up")
        );
    }

    /// A row of a review of an older commit is not a line of the file as it stands, and the
    /// name it holds is what says so - a line number alone would happily point at anything.
    #[test]
    fn a_row_whose_line_no_longer_holds_the_name_is_not_asked_about() {
        let text = "fn one() {}\npub fn greet() {}\n";
        assert!(line_holds(text, 1, "greet"));
        assert!(!line_holds(text, 0, "greet"));
        // Past the end of a file that has been cut down since the review was made.
        assert!(!line_holds(text, 9, "greet"));
    }
}
