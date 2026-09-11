//! Putting edits a language server worked out into the files they are for - what a rename and
//! a code action both end in.
//!
//! A file open in a tab takes its edits into the tab's buffer, unsaved, the way typing would:
//! the tab is what the person is looking at, and its unsaved edits are theirs to keep or throw
//! away. A file nobody has open is written.
//!
//! The edits are places in the texts the server had when it worked them out, so what every tab
//! holds is taken down as the question goes out, and a tab that no longer reads that way -
//! typed into while the server was working, or opened since - refuses the whole of it. Half a
//! rename is a project that does not build, and an edit landing on whatever sits at its line
//! and column now is worse than none.

use std::{collections::HashMap, ops::Range, time::Duration};

use anyhow::{Context, bail};
use egui_frames::PaneId;
use egui_moon_code_ide::{CanAnswer, LspFileEdit};

use crate::native::{app::App, model::Model};

/// How long a question that needs every tab's text on the server waits for it to get there.
/// The document sync sends once the typing has paused, so this is several of those pauses and
/// a round trip each; a wait longer than that is a link or a server that is down rather than
/// slow, and the person is better told than left looking at a key that did nothing.
pub(crate) const HEARD_WITHIN: Duration = Duration::from_secs(5);

/// How soon something waiting on the document sync looks again. The sync lands on a frame of
/// its own, and nothing else draws one while a window sits idle.
pub(crate) const LOOKS_AGAIN_IN: Duration = Duration::from_millis(50);

/// A tab whose text has not reached its server yet, by the file it shows. `None` once every
/// tab's has.
pub(crate) fn tab_behind_its_server(model: &Model) -> Option<String> {
    model
        .file_editors
        .values()
        .find(|editor| {
            editor.server_heard().was_opened()
                && editor.server_heard().can_answer_about(editor.text()) == CanAnswer::NotThisText
        })
        .map(|editor| editor.file_path.clone())
}

/// What every loaded tab holds, by file: the texts the server is working from once none is
/// behind it.
pub(crate) fn what_the_tabs_hold(model: &Model) -> HashMap<String, String> {
    model
        .file_editors
        .values()
        .filter(|editor| editor.is_loaded())
        .map(|editor| (editor.file_path.clone(), editor.text().to_string()))
        .collect()
}

/// What a tab still behind its server says, once the wait is over.
pub(crate) fn not_heard(file_path: &str) -> String {
    format!("the language server has not heard what is in {file_path} yet - try again in a moment")
}

/// How much a set of edits changed, for the line that says so.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) places: usize,
    pub(crate) files: usize,
    /// How many of those files are open in a tab, and so edited there and not saved.
    pub(crate) files_in_tabs: usize,
}

impl Counts {
    /// What is left for the person to do: save the tabs that were edited.
    pub(crate) fn unsaved_note(&self) -> String {
        match self.files_in_tabs {
            0 => String::new(),
            count => format!(
                " - {} open in tabs, edited there and not saved yet",
                counted(count, "file")
            ),
        }
    }
}

pub(crate) fn counted(count: usize, thing: &str) -> String {
    match count {
        1 => format!("1 {thing}"),
        _ => format!("{count} {thing}s"),
    }
}

/// Put `files` into the tabs showing them and onto disk for the rest, against `told` - what
/// the tabs held when the server was asked. `doing` names what is being done, for what is said
/// when it cannot be; `said` is what is said once it is done.
pub(crate) fn put_in(
    app: &mut App,
    session_id: String,
    told: &HashMap<String, String>,
    files: Vec<LspFileEdit>,
    doing: String,
    said: impl FnOnce(&Counts) -> String,
) {
    let tabs: Vec<(PaneId, &str, &str)> = app
        .model
        .file_editors
        .iter()
        .filter(|(_, editor)| editor.is_loaded())
        .map(|(pane_id, editor)| (*pane_id, editor.file_path.as_str(), editor.text()))
        .collect();
    let plan = match plan_of(&tabs, told, files) {
        Ok(plan) => plan,
        Err(error) => {
            app.model.error(format!("{doing} stopped: {error}"));
            return;
        }
    };

    for (pane_id, ranges) in plan.in_tabs {
        if let Some(editor) = app.model.file_editors.get_mut(&pane_id) {
            editor.take_edits(ranges);
        }
    }
    let said = said(&plan.counts);
    if plan.on_disk.is_empty() {
        app.model.info(said);
        return;
    }

    let on_disk = plan.on_disk;
    app.tasks.spawn(
        move |backend| {
            for file in &on_disk {
                let content = backend.file_content(&session_id, &file.file_path)?.content;
                let edited = moon_lsp::edits::apply(&content, &file.edits).with_context(|| {
                    format!(
                        "{} changed on disk while the server was working",
                        file.file_path
                    )
                })?;
                backend
                    .write_file(&session_id, &file.file_path, &edited)
                    .with_context(|| format!("could not write {}", file.file_path))?;
            }
            Ok(())
        },
        move |model, result| match result {
            Ok(()) => model.info(said),
            Err(error) => model.error(format!(
                "{doing} changed the open tabs, but not every other file: {error:#}"
            )),
        },
    );
}

/// Where a set of edits goes: the edits for each tab showing a file, and the files nobody has
/// open.
struct Plan<Tab> {
    /// The edits for each tab showing a file the edits change, as byte ranges of its text.
    in_tabs: Vec<(Tab, Vec<(Range<usize>, String)>)>,
    /// The files no tab is showing, to be written.
    on_disk: Vec<LspFileEdit>,
    counts: Counts,
}

/// Work out where each file's edits go: into every tab showing it, or onto disk.
///
/// `tabs` is every loaded tab - its key, its file and its text now - and `told` what the tabs
/// held when the server was asked, by file. A file a tab shows has to still read the way it did
/// then in every tab showing it, or the whole set is refused: the edits are places in that
/// text, and nowhere else.
fn plan_of<Tab: Copy>(
    tabs: &[(Tab, &str, &str)],
    told: &HashMap<String, String>,
    files: Vec<LspFileEdit>,
) -> anyhow::Result<Plan<Tab>> {
    let mut counts = Counts {
        places: files.iter().map(|file| file.edits.len()).sum(),
        files: files.len(),
        files_in_tabs: 0,
    };
    let mut in_tabs = Vec::new();
    let mut on_disk = Vec::new();
    for file in files {
        let showing: Vec<&(Tab, &str, &str)> = tabs
            .iter()
            .filter(|(_, file_path, _)| *file_path == file.file_path)
            .collect();
        if showing.is_empty() {
            on_disk.push(file);
            continue;
        }
        let Some(text) = told.get(&file.file_path) else {
            bail!(
                "{} was opened while the server was working, so nothing was changed",
                file.file_path
            );
        };
        if showing.iter().any(|(_, _, now)| now != text) {
            bail!(
                "{} was edited while the server was working, so nothing was changed",
                file.file_path
            );
        }
        let ranges = moon_lsp::edits::byte_ranges(text, &file.edits)
            .with_context(|| format!("the edits do not fit {}", file.file_path))?;
        counts.files_in_tabs += 1;
        in_tabs.extend(showing.iter().map(|(tab, _, _)| (*tab, ranges.clone())));
    }
    Ok(Plan {
        in_tabs,
        on_disk,
        counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_moon_code_ide::{LspPosition, LspTextEdit};

    fn edited(file_path: &str, line: usize, columns: Range<usize>) -> LspFileEdit {
        LspFileEdit {
            file_path: file_path.to_string(),
            edits: vec![LspTextEdit {
                start: LspPosition {
                    line,
                    column: columns.start,
                },
                end: LspPosition {
                    line,
                    column: columns.end,
                },
                new_text: "hello".to_string(),
            }],
        }
    }

    fn told(files: &[(&str, &str)]) -> HashMap<String, String> {
        files
            .iter()
            .map(|(file_path, text)| (file_path.to_string(), text.to_string()))
            .collect()
    }

    /// A file open in a tab takes its edits in the tab - every tab showing it - and a file
    /// nobody has open is left to be written.
    #[test]
    fn a_file_in_a_tab_is_edited_there_and_one_nobody_has_open_is_written() {
        let lib = "pub fn greet() {}\n";
        let tabs = [(1, "src/lib.rs", lib), (2, "src/lib.rs", lib)];
        let plan = plan_of(
            &tabs,
            &told(&[("src/lib.rs", lib)]),
            vec![
                edited("src/lib.rs", 0, 7..12),
                edited("src/main.rs", 2, 4..9),
            ],
        )
        .expect("expected the edits to be planned");

        assert_eq!(
            plan.in_tabs,
            vec![
                (1, vec![(7..12, "hello".to_string())]),
                (2, vec![(7..12, "hello".to_string())])
            ]
        );
        assert_eq!(
            plan.on_disk
                .iter()
                .map(|file| file.file_path.as_str())
                .collect::<Vec<_>>(),
            ["src/main.rs"]
        );
        assert_eq!(
            plan.counts,
            Counts {
                places: 2,
                files: 2,
                files_in_tabs: 1
            }
        );
    }

    /// A tab typed into while the server was working, or opened since, refuses the whole set:
    /// its edits are places in a text it no longer holds.
    #[test]
    fn a_tab_that_moved_while_the_server_was_working_refuses_the_whole_of_it() {
        let lib = "pub fn greet() {}\n";
        let typed_into = [(1, "src/lib.rs", "// typed\npub fn greet() {}\n")];
        let refused = plan_of(
            &typed_into,
            &told(&[("src/lib.rs", lib)]),
            vec![edited("src/lib.rs", 0, 7..12)],
        )
        .err()
        .expect("a tab typed into since has to refuse the edits");
        assert!(refused.to_string().contains("was edited"), "{refused}");

        let opened = [(1, "src/lib.rs", lib)];
        let refused = plan_of(&opened, &told(&[]), vec![edited("src/lib.rs", 0, 7..12)])
            .err()
            .expect("a tab opened since has to refuse the edits");
        assert!(refused.to_string().contains("was opened"), "{refused}");
    }
}
