//! ⌘-click a name in a file and land on where it is defined.
//!
//! Where a language server is installed for the file, it is asked first and its answer is the
//! answer: it has parsed the project and knows which `new` of the forty in the repo this one
//! is. For every other language - and that is most of a repo, and most machines - there is
//! `ag` over the repo, which finds every line holding the name, the definition among them,
//! along with every call of it. Which of those lines reads as the definition is
//! [`crate::native::definition_ranking`]'s reading; what is here is the choosing between the
//! two kinds of answer, and where the window is sent by the one it gets.
//!
//! That reading is a guess, and it is treated as one. When a single line reads as the
//! definition and nothing else comes near it, the click opens it and the guess never has to be
//! discussed. When several are plausible the palette offers them and the person picks, which
//! is a better answer than jumping somewhere wrong and quietly losing where they were. A
//! server's answer is not a guess and is not ranked at all: one place is a jump, several are
//! offered.
//!
//! The search is not a consolation prize for the server being missing. It is what markdown,
//! shell, SQL and every other language nobody wrote a server for use forever, so it keeps
//! working exactly as it did before there was a server to ask.
//!
//! Two places take the click: a file pane, and a row of a review's diff - which is where most
//! of the reading in this window actually happens, so a jump that only worked in the file pane
//! was a feature nobody ever met. Both end in the same lookup, the same ranking and the same
//! landing; what differs is only where the answer waits for a frame that can open a pane, and
//! whether a language server is in a position to be asked at all.

use egui_frames::PaneId;
use egui_moon_code_ide::{AsksAbout, LanguageSource, asks_about, still_starting};
use egui_moon_editor::Word;

use crate::{
    api::{ContentMatch, LspLocation, LspPosition},
    backend::Backend,
    native::{
        app::App,
        definition_ranking::{Ranked, rank},
        language_source::SessionLanguages,
        palette::CommandAction,
        panes::{OpenAt, OpenPaneRequest},
    },
};

/// What the lookup came back with, before a frame that can act on it has read it.
enum Answer {
    /// A server serves this file and has not finished indexing the project. It would answer
    /// with nothing, which is why it is not asked: nothing reads as "this name is defined
    /// nowhere", and the person would go looking for a bug that is really a wait.
    Indexing,
    /// A server answered. Everywhere it says the name is defined.
    Server(Vec<LspLocation>),
    /// No server was there to answer, or the one that was had nothing to say. Every line the
    /// repo holds the name on.
    Searched(Vec<ContentMatch>),
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
    /// Where the answer sends the window, worked out where it arrived: the reading of it
    /// needs nothing from the frame, and the frame that opens the pane should only open it.
    landing: Landing,
}

/// Where a finished lookup sends the window.
enum Landing {
    /// The repo does not hold the name anywhere, which can only mean the search never reached
    /// the file - the clicked name is in it by definition.
    Nowhere,
    /// One line reads as the definition and nothing else scored as well.
    Straight(ContentMatch),
    /// Several are plausible. The palette offers them all rather than picking one.
    Offer(Vec<ContentMatch>),
}

/// Look the clicked name up: the server behind the file if there is one, the repo if there
/// is not.
///
/// A blocking question to a server and a blocking `ag` over a repo that may be on another
/// machine, so the pair of them goes to a worker thread the way reading a file does. Keyed by
/// pane, so ⌘-clicking twice in a row looks up the second name rather than racing the first.
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
    let name = word.text.clone();
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
        move |backend| answer_for(backend, &for_call, &for_ask, asks_a_server, &name, at),
        move |model, result| {
            let answer = match result {
                Ok(answer) => answer,
                Err(error) => {
                    model.error(format!("could not look up {word}: {error}"));
                    return;
                }
            };
            let landing = match answer {
                // Said rather than jumped to: the click has to read as having been heard,
                // and landing somewhere the search guessed at while the server is coming up
                // is worse than being told to ask again.
                Answer::Indexing => {
                    model.info(still_starting(&file_path));
                    return;
                }
                Answer::Server(locations) => landing_of_locations(locations),
                Answer::Searched(matches) => landing_of(rank(&word, matches)),
            };
            // The pane may have been closed while the lookup was out, and the answer belongs
            // to nobody else.
            let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                return;
            };
            editor.looking_up = Some(LookedUp { word, landing });
        },
    );
}

/// Look up a name ⌘-clicked on a row of a review's diff.
///
/// The repo search answers, and only the repo search. A review has never told a language
/// server about the file whose rows it is showing - opening a document belongs to the file
/// pane, which holds the text and keeps the server's copy of it in step as it is typed into -
/// and [`crate::lsp`] refuses, loudly, to answer about a document it was never told about. So
/// asking anyway would be either that refusal in the reader's face or a document opened behind
/// their back that nothing afterwards closes, and neither is worth what a server would add to
/// a jump the search already makes.
///
/// Which also settles which line the click was on, a question a diff makes real: a removed row
/// is text the file does not contain any more, so there is no position in it for a server to
/// be asked about even if one were listening. A search wants the name and nothing else, and
/// reads it the same off a removal as off an addition.
///
/// Keyed by review, so ⌘-clicking twice in a row looks up the second name rather than racing
/// the first - the same as a file pane, and for the same reason.
pub(crate) fn look_up_in_review(app: &mut App, session_id: &str, word: String) {
    let for_call = session_id.to_string();
    let for_park = session_id.to_string();
    let name = word.clone();

    app.tasks.spawn_keyed(
        Some(format!("definition:review:{session_id}")),
        move |backend| Ok(backend.search_contents(&for_call, &name)?.matches),
        move |model, result| {
            let matches = match result {
                Ok(matches) => matches,
                Err(error) => {
                    model.error(format!("could not look up {word}: {error}"));
                    return;
                }
            };
            let landing = landing_of(rank(&word, matches));
            model.review(&for_park).looking_up = Some(LookedUp { word, landing });
        },
    );
}

/// Ask the server, and let the repo search answer whenever the server does not.
///
/// A server that errors is a server that did not answer, so the search runs: a language
/// server dropping its connection mid-session must not take go-to-definition down with it,
/// and the repo is still there to be searched. The search's own error is the caller's, the
/// way it always was - it is the last thing there is to try.
fn answer_for(
    backend: &dyn Backend,
    session_id: &str,
    file_path: &str,
    asks_a_server: bool,
    word: &str,
    at: LspPosition,
) -> anyhow::Result<Answer> {
    let languages = SessionLanguages::new(backend, session_id);
    // A status that could not be had is a file with no server, as far as a click is
    // concerned: the search is what answers, and nothing is said about it - see
    // [`SessionLanguages::status`].
    let asks = match asks_a_server {
        true => asks_about(languages.status(file_path)),
        false => AsksAbout::Elsewhere,
    };
    match asks {
        AsksAbout::Wait => return Ok(Answer::Indexing),
        AsksAbout::Server => {
            if let Ok(locations) = languages.definition(file_path, at)
                && !locations.is_empty()
            {
                return Ok(Answer::Server(locations));
            }
        }
        // The repo search, which is this window's own fallback and the reason the crate
        // stops where it does - see [`crate::native::definition_ranking`].
        AsksAbout::Elsewhere => {}
    }
    Ok(Answer::Searched(
        backend.search_contents(session_id, word)?.matches,
    ))
}

/// Where a server's answer sends the window.
///
/// A server that answered at all is taken at its word - no ranking, no guessing. One place
/// is the place. Several are the several the language really has, a trait method and the
/// impls of it, and those are a choice the same way two plausible lines are.
fn landing_of_locations(locations: Vec<LspLocation>) -> Landing {
    let mut rows: Vec<ContentMatch> = locations.into_iter().map(row_of).collect();
    match rows.len() {
        1 => Landing::Straight(rows.remove(0)),
        _ => Landing::Offer(rows),
    }
}

/// A place the server named, as a row the palette can offer.
///
/// The palette's rows are [`ContentMatch`]es because they are the search's rows, and the two
/// kinds of answer are offered in the same list rather than in two. A server's answer has no
/// line of text to read the row by - it names a file and a line and nothing else - so the row
/// reads as the file's name and the line number, the way the file finder's rows read a file
/// by its name, with the path underneath saying which `mod.rs` this one is. Fetching the line
/// itself would be a round trip per row for a list that is usually two long.
fn row_of(found: LspLocation) -> ContentMatch {
    ContentMatch {
        line: format!("{}:{}", file_name_of(&found.file_path), found.line_number),
        file_path: found.file_path,
        line_number: found.line_number,
    }
}

fn file_name_of(file_path: &str) -> &str {
    file_path.rsplit('/').next().unwrap_or(file_path)
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
    match looked_up.landing {
        // Silence would read as the click having missed, so it says so.
        Landing::Nowhere => app
            .model
            .error(format!("nothing in the repo holds {}", looked_up.word)),
        Landing::Straight(found) => {
            app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::File {
                session_id: session_id.to_string(),
                file_path: found.file_path,
                at: Some(OpenAt {
                    line: found.line_number,
                    query: looked_up.word,
                }),
            }));
        }
        Landing::Offer(candidates) => app.model.palette.show_definitions(
            looked_up.word,
            session_id.to_string(),
            candidates,
        ),
    }
}

/// The decision the ranking is there to make.
///
/// A jump is only taken without asking when one line reads as a definition and nothing else
/// scored as well as it did. Two lines that both read as definitions - a trait method and the
/// impl of it, the same name in two crates - are a choice, not an answer, and so is a pile of
/// mentions with no definition among them.
fn landing_of(ranked: Vec<Ranked>) -> Landing {
    let Some(best) = ranked.first() else {
        return Landing::Nowhere;
    };
    let alone = ranked.get(1).is_none_or(|next| next.score < best.score);
    if best.reads_as_a_definition() && alone {
        return Landing::Straight(
            ranked
                .into_iter()
                .next()
                .expect("the best hit was just read")
                .found,
        );
    }
    Landing::Offer(ranked.into_iter().map(|ranked| ranked.found).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(file_path: &str, line: &str) -> ContentMatch {
        ContentMatch {
            file_path: file_path.to_string(),
            line_number: 1,
            line: line.to_string(),
        }
    }

    /// One definition and nothing else near it is a jump; anything less is a question.
    #[test]
    fn one_clear_definition_opens_and_anything_less_is_offered() {
        let one = rank(
            "greet",
            vec![
                found("src/main.rs", "greet(name);"),
                found("src/lib.rs", "pub fn greet(name: &str) -> String {"),
            ],
        );
        assert!(matches!(landing_of(one), Landing::Straight(found) if found.file_path == "src/lib.rs"));

        // Two files declaring the same name is a choice, not an answer.
        let two = rank(
            "greet",
            vec![
                found("src/one.rs", "pub fn greet() {}"),
                found("src/two.rs", "pub fn greet() {}"),
            ],
        );
        assert!(matches!(landing_of(two), Landing::Offer(candidates) if candidates.len() == 2));

        // Mentions only: nowhere stands out enough to be jumped to.
        let none = rank("greet", vec![found("src/main.rs", "greet(name);")]);
        assert!(matches!(landing_of(none), Landing::Offer(candidates) if candidates.len() == 1));

        assert!(matches!(landing_of(rank("greet", Vec::new())), Landing::Nowhere));
    }

    /// A server that answered is taken at its word: one place is a jump, several are the
    /// several the language has and are offered rather than guessed between.
    #[test]
    fn one_place_from_the_server_is_a_jump_and_several_are_offered() {
        let one = landing_of_locations(vec![LspLocation {
            file_path: "src/lib.rs".to_string(),
            line_number: 12,
        }]);
        assert!(matches!(one, Landing::Straight(found)
            if found.file_path == "src/lib.rs" && found.line_number == 12));

        let two = landing_of_locations(vec![
            LspLocation {
                file_path: "src/one.rs".to_string(),
                line_number: 3,
            },
            LspLocation {
                file_path: "src/two.rs".to_string(),
                line_number: 4,
            },
        ]);
        assert!(matches!(two, Landing::Offer(candidates) if candidates.len() == 2));
    }

    /// The server names a file and a line and no text at all, so the row is read by the
    /// file's name with the path underneath saying which one it is.
    #[test]
    fn a_place_the_server_named_reads_as_the_file_and_the_line() {
        let row = row_of(LspLocation {
            file_path: "src/native/definition.rs".to_string(),
            line_number: 120,
        });

        assert_eq!(row.line, "definition.rs:120");
        assert_eq!(row.file_path, "src/native/definition.rs");
        assert_eq!(row.line_number, 120);
    }
}
