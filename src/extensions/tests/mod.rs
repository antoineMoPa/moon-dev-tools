//! Scripts run the way a pane runs them: on a thread of their own, answering in views and
//! effects.
//!
//! What the tests share is here: a scratch folder, a script to start, and what it has said.
//! The tests themselves are by what they are about - see the modules below.

mod host;
mod lifecycle;
mod shipped;

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::{Effect, Element, Extension, Output, Running, Source, host::Host};

/// How long a test waits for a script to answer before calling it stuck.
const PATIENCE: Duration = Duration::from_secs(10);

/// A folder of the test's own, gone when it is.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "moonreview-extension-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("failed to make the scratch folder");
        Self { dir }
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.dir.join(relative);
        std::fs::create_dir_all(path.parent().expect("a file is in a folder"))
            .expect("failed to make the folder");
        std::fs::write(&path, contents).expect("failed to write the file");
        path
    }

    /// A program on the scratch folder's `bin/`, for a script to run in place of the real one.
    fn program(&self, name: &str, script: &str) {
        let path = self.write(&format!("bin/{name}"), script);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("failed to make the program runnable");
    }

    /// A PATH with the scratch folder's programs, and the system's own for `sh` and `git`.
    fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.dir.join("bin").display())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start(extension: Extension, root: &Path, path: String) -> Running {
    let host = Host {
        project_root: root.to_path_buf(),
        path,
    };
    Running::start(extension, host, || {})
}

fn script(source: &'static str) -> Extension {
    Extension {
        name: "test".to_string(),
        about: String::new(),
        source: Source::Shipped(source),
    }
}

/// Everything a script has said so far.
#[derive(Default)]
struct Heard {
    view: Option<Element>,
    error: Option<String>,
    effects: Vec<Effect>,
    /// What the last version to load said about `on_key`.
    takes_keys: Option<bool>,
}

impl Heard {
    /// Take in what the script says until `done` is true of it.
    fn until(&mut self, running: &Running, done: impl Fn(&Self) -> bool) {
        let until = Instant::now() + PATIENCE;
        loop {
            for output in running.take_outputs() {
                match output {
                    Output::Drawn { view, error } => {
                        if let Some(view) = view {
                            self.view = Some(view);
                        }
                        self.error = error;
                    }
                    Output::Loaded { takes_keys } => self.takes_keys = Some(takes_keys),
                    Output::Effect(effect) => self.effects.push(effect),
                }
            }
            if done(self) {
                return;
            }
            assert!(
                Instant::now() < until,
                "the script never got there: last error {:?}, texts {:?}",
                self.error,
                self.texts()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Every word on the view, depth first: texts, headings, button labels, table cells.
    fn texts(&self) -> Vec<String> {
        let mut texts = Vec::new();
        if let Some(view) = &self.view {
            texts_of(view, &mut texts);
        }
        texts
    }

    fn shows(&self, text: &str) -> bool {
        self.texts().iter().any(|shown| shown == text)
    }

    /// The one text of a view that is a single `text(..)`.
    fn only_text(&self) -> String {
        let texts = self.texts();
        assert_eq!(texts.len(), 1, "a view of one text: {texts:?}");
        texts[0].clone()
    }

    /// How many errors have been written to the window's messages.
    fn logged_failures(&self) -> usize {
        self.effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Failed(_)))
            .count()
    }

    /// The row of the view's table that shows `text` in one of its cells.
    fn row_showing(&self, text: &str) -> super::TableRow {
        let Some(Element::Column { children }) = &self.view else {
            panic!("both shipped extensions answer with a column");
        };
        let rows = children
            .iter()
            .find_map(|child| match child {
                Element::Table { rows, .. } => Some(rows),
                _ => None,
            })
            .expect("the view has a table");
        rows.iter()
            .find(|row| {
                let mut texts = Vec::new();
                row.cells.iter().for_each(|cell| texts_of(cell, &mut texts));
                texts.iter().any(|shown| shown == text)
            })
            .cloned()
            .unwrap_or_else(|| panic!("no row shows {text}"))
    }

    /// Every word in the cells of the selected row of the view's table.
    fn selected_row(&self) -> Vec<String> {
        let Some(Element::Column { children }) = &self.view else {
            panic!("both shipped extensions answer with a column");
        };
        let rows = children
            .iter()
            .find_map(|child| match child {
                Element::Table { rows, .. } => Some(rows),
                _ => None,
            })
            .expect("the view has a table");
        let row = rows
            .iter()
            .find(|row| row.selected)
            .expect("a row is selected");
        let mut texts = Vec::new();
        for cell in &row.cells {
            texts_of(cell, &mut texts);
        }
        texts
    }
}

fn texts_of(element: &Element, texts: &mut Vec<String>) {
    match element {
        Element::Text { text, .. } | Element::Heading { text } | Element::Code { text } => {
            texts.push(text.clone());
        }
        Element::Button { label, .. } => texts.push(label.clone()),
        Element::Input { value, .. } => texts.push(value.clone()),
        Element::Row { children } | Element::Column { children } => {
            children.iter().for_each(|child| texts_of(child, texts));
        }
        Element::Table { columns, rows } => {
            texts.extend(columns.iter().cloned());
            for row in rows {
                row.cells.iter().for_each(|cell| texts_of(cell, texts));
            }
        }
        Element::Separator => {}
    }
}

fn system_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}
