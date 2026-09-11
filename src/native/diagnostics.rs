//! What the language server behind a file tab found wrong with it: underlined in the text,
//! told in a tooltip when the pointer is over it, and counted in the tab's header.
//!
//! Servers push these whenever they like - rust-analyzer as it reads a file, and again after
//! every `cargo check`, which a save sets off. The repo side keeps what was pushed, and the
//! tab asks for it on a timer: locally a read of a map, on a `--remote` review one round trip
//! a second for each tab on screen.

use std::{
    ops::Range,
    time::{Duration, Instant},
};

use egui::Color32;
use egui_frames::PaneId;
use egui_moon_code_ide::{LanguageSource, LspDiagnostic, LspSeverity};
use egui_moon_editor::Underline;

use crate::native::{app::App, language_source::SessionLanguages, theme::Palette};

/// How often a tab on screen asks what its server has found. A `cargo check` takes seconds,
/// so a second is well inside the time anybody waits for one to show.
const ASKED_EVERY: Duration = Duration::from_secs(1);

/// What a tab's server last said is wrong with it, and when it was last asked.
#[derive(Default)]
pub(crate) struct Diagnosed {
    found: Vec<LspDiagnostic>,
    asked_at: Option<Instant>,
    asking: bool,
}

impl Diagnosed {
    pub(crate) fn found(&self) -> &[LspDiagnostic] {
        &self.found
    }

    /// Whether it is time to ask again, marking the question out if it is. Never twice at
    /// once, and not again until the last answer has been in for [`ASKED_EVERY`].
    fn due(&mut self, now: Instant) -> bool {
        if self.asking
            || self
                .asked_at
                .is_some_and(|at| now.duration_since(at) < ASKED_EVERY)
        {
            return false;
        }
        self.asking = true;
        true
    }

    /// The answer, as it comes back. A question that could not be answered keeps what was
    /// last found rather than wiping it: a link that blinks is not a file that was fixed.
    fn answered(&mut self, found: Option<Vec<LspDiagnostic>>, now: Instant) {
        self.asking = false;
        self.asked_at = Some(now);
        if let Some(found) = found {
            self.found = found;
        }
    }
}

/// Ask what the server behind this tab has found, when it is time to. Called as the tab draws,
/// so only a tab on screen asks.
pub(crate) fn follow(app: &mut App, ctx: &egui::Context, pane_id: PaneId, session_id: &str) {
    let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
        return;
    };
    // Nothing is published about a file its server has not been told about.
    if !editor.server_heard().was_opened() {
        return;
    }
    // A window nobody is touching draws no frames, and the next question needs one.
    ctx.request_repaint_after(ASKED_EVERY);
    if !editor.diagnosed_mut().due(Instant::now()) {
        return;
    }
    let file_path = editor.file_path.clone();
    let for_call = session_id.to_string();
    app.tasks.spawn_keyed(
        Some(format!("lsp-diagnostics:{pane_id}")),
        move |backend| SessionLanguages::new(backend, &for_call).diagnostics(&file_path),
        move |model, result| {
            if let Some(editor) = model.file_editors.get_mut(&pane_id) {
                editor.diagnosed_mut().answered(result.ok(), Instant::now());
            }
        },
    );
}

/// The colour a diagnostic of each grade is drawn in: an error in the colour of removed lines,
/// a warning in the warning colour, the rest quieter.
pub(crate) fn colour_of(severity: LspSeverity, palette: &Palette) -> Color32 {
    match severity {
        LspSeverity::Error => palette.removed,
        LspSeverity::Warning => palette.warn,
        LspSeverity::Information => palette.accent,
        LspSeverity::Hint => palette.muted,
    }
}

/// The stretch of `text` a diagnostic is about, in bytes. `None` for one that does not fit the
/// text as it stands - typed into since the server published it.
///
/// A diagnostic about a point - a missing `;` - is widened to the character after it, or the
/// one before it at the end of a line, so there is something to draw a line under.
fn range_in(text: &str, diagnostic: &LspDiagnostic) -> Option<Range<usize>> {
    let start = moon_lsp::edits::offset_of(text, &diagnostic.start).ok()?;
    let end = moon_lsp::edits::offset_of(text, &diagnostic.end).ok()?;
    if end > start {
        return Some(start..end);
    }
    let after = text[start..]
        .chars()
        .next()
        .filter(|character| *character != '\n')
        .map(|character| start..start + character.len_utf8());
    after.or_else(|| {
        text[..start]
            .chars()
            .next_back()
            .filter(|character| *character != '\n')
            .map(|character| start - character.len_utf8()..start)
    })
}

/// What the editor underlines, worst last so it is drawn over anything quieter it overlaps.
pub(crate) fn underlines(text: &str, found: &[LspDiagnostic], palette: &Palette) -> Vec<Underline> {
    let mut worst_last: Vec<&LspDiagnostic> = found.iter().collect();
    worst_last.sort_by_key(|diagnostic| std::cmp::Reverse(rank_of(diagnostic.severity)));
    worst_last
        .into_iter()
        .filter_map(|diagnostic| {
            Some(Underline {
                range: range_in(text, diagnostic)?,
                color: colour_of(diagnostic.severity, palette),
            })
        })
        .collect()
}

/// Worst first: an error is 0.
fn rank_of(severity: LspSeverity) -> usize {
    match severity {
        LspSeverity::Error => 0,
        LspSeverity::Warning => 1,
        LspSeverity::Information => 2,
        LspSeverity::Hint => 3,
    }
}

/// The diagnostics about the byte `offset` of `text`, worst first - what the tooltip over it
/// says.
pub(crate) fn at<'a>(
    text: &str,
    found: &'a [LspDiagnostic],
    offset: usize,
) -> Vec<&'a LspDiagnostic> {
    let mut here: Vec<&LspDiagnostic> = found
        .iter()
        .filter(|diagnostic| {
            range_in(text, diagnostic).is_some_and(|range| range.contains(&offset))
        })
        .collect();
    here.sort_by_key(|diagnostic| rank_of(diagnostic.severity));
    here
}

/// What the tab's header says about what was found, and the grade of the worst of it. `None`
/// for a file with no errors or warnings: information and hints are there to be found by
/// pointing at them, not counted at the top of every file.
pub(crate) fn said_in_header(found: &[LspDiagnostic]) -> Option<(String, LspSeverity)> {
    let count = |severity| {
        found
            .iter()
            .filter(|diagnostic| diagnostic.severity == severity)
            .count()
    };
    let (errors, warnings) = (count(LspSeverity::Error), count(LspSeverity::Warning));
    let said = [(errors, "error"), (warnings, "warning")]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, thing)| match count {
            1 => format!("1 {thing}"),
            _ => format!("{count} {thing}s"),
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let worst = match errors {
        0 => LspSeverity::Warning,
        _ => LspSeverity::Error,
    };
    (!said.is_empty()).then_some((said, worst))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_moon_code_ide::LspPosition;

    fn diagnostic(line: usize, columns: Range<usize>, severity: LspSeverity) -> LspDiagnostic {
        LspDiagnostic {
            start: LspPosition {
                line,
                column: columns.start,
            },
            end: LspPosition {
                line,
                column: columns.end,
            },
            severity,
            message: "wrong".to_string(),
            source: None,
        }
    }

    /// A diagnostic about a point is widened to a character so there is a line to draw, and
    /// one that no longer fits the text is not drawn anywhere.
    #[test]
    fn a_point_is_widened_to_a_character_and_what_does_not_fit_is_left_out() {
        let text = "let x = 1\nlet y\n";
        assert_eq!(
            range_in(text, &diagnostic(0, 4..5, LspSeverity::Error)),
            Some(4..5)
        );
        assert_eq!(
            range_in(text, &diagnostic(0, 4..4, LspSeverity::Error)),
            Some(4..5)
        );
        // At the end of a line, the character before it.
        assert_eq!(
            range_in(text, &diagnostic(0, 9..9, LspSeverity::Error)),
            Some(8..9)
        );
        assert_eq!(
            range_in(text, &diagnostic(7, 0..1, LspSeverity::Error)),
            None
        );
    }

    /// The tooltip says the worst first, and the header counts errors and warnings only.
    #[test]
    fn the_worst_is_said_first_and_only_errors_and_warnings_are_counted() {
        let text = "let x = 1\n";
        let found = [
            diagnostic(0, 4..5, LspSeverity::Hint),
            diagnostic(0, 4..5, LspSeverity::Error),
            diagnostic(0, 8..9, LspSeverity::Warning),
            diagnostic(0, 0..3, LspSeverity::Warning),
        ];
        let here = at(text, &found, 4);
        assert_eq!(
            here.iter()
                .map(|diagnostic| diagnostic.severity)
                .collect::<Vec<_>>(),
            [LspSeverity::Error, LspSeverity::Hint]
        );
        assert_eq!(
            said_in_header(&found),
            Some(("1 error · 2 warnings".to_string(), LspSeverity::Error))
        );
        assert_eq!(
            said_in_header(&[diagnostic(0, 4..5, LspSeverity::Hint)]),
            None
        );
    }

    /// A tab asks again once the last answer has been in a while, never twice at once, and a
    /// question that could not be answered leaves what was found.
    #[test]
    fn a_tab_asks_again_once_the_last_answer_has_been_in_a_while() {
        let start = Instant::now();
        let mut diagnosed = Diagnosed::default();
        assert!(diagnosed.due(start));
        assert!(!diagnosed.due(start), "one question at a time");
        diagnosed.answered(Some(vec![diagnostic(0, 0..1, LspSeverity::Error)]), start);
        assert!(!diagnosed.due(start + ASKED_EVERY / 2));
        assert!(diagnosed.due(start + ASKED_EVERY));
        diagnosed.answered(None, start + ASKED_EVERY);
        assert_eq!(diagnosed.found().len(), 1, "a failed question is not a fix");
    }
}
