//! Extensions: panes written as [Rhai](https://rhai.rs) scripts, read while the window runs.
//!
//! An extension is one `.rhai` file. The window calls four of its functions and draws what
//! they answer:
//!
//! - `init(project_root)` answers with the extension's state, whatever the script wants it to
//!   be - usually a map.
//! - `view()` answers with what the pane shows, as a tree of maps - see [`view::Element`].
//! - `update(event)` is called with the event a button or a table row carried when it was
//!   clicked, and changes the state.
//! - `on_key(key)`, if the script has one, is called with each key pressed while the pane has
//!   the keyboard.
//! - `tick()`, if the script has one, is called as often as it asked with `every(ms)`.
//!
//! The state is `this` inside all but `init`, so `update` changes the state by assigning to
//! `this.something` and `view` reads it the same way.
//!
//! What a script can do beyond changing its state is the functions [`host`] binds: run a
//! program, make an HTTP request, read a folder, open a file or a shell in the window, say
//! something in a toast.
//!
//! Two are shipped in the executable - `files` and `docker`, out of `extensions/` at the top of
//! this repo - and every `.rhai` file in `~/.moonreview/extensions/` is one more, or takes the
//! place of the shipped one of the same name. A file there is read again whenever it changes,
//! and the pane keeps its state across the change, so an extension is written with the pane it
//! draws open beside it.

pub(crate) mod host;
#[cfg(test)]
mod tests;
mod view;
mod worker;

use std::path::PathBuf;

pub(crate) use view::{Element, Ink, TableRow};
pub(crate) use worker::{Effect, Input, Output, Running};

/// An extension the window can open.
#[derive(Clone, Debug)]
pub(crate) struct Extension {
    /// What it goes by: its file's name, less the `.rhai`. The palette offers it under this.
    pub(crate) name: String,
    /// What it is for, out of the `//!` lines it starts with. The palette says this under the
    /// name.
    pub(crate) about: String,
    pub(crate) source: Source,
}

#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// Built into the executable.
    Shipped(&'static str),
    /// A file of the person's, read again whenever it changes.
    File(PathBuf),
}

struct Shipped {
    name: &'static str,
    source: &'static str,
}

const SHIPPED: &[Shipped] = &[
    Shipped {
        name: "files",
        source: include_str!("../../extensions/files.rhai"),
    },
    Shipped {
        name: "docker",
        source: include_str!("../../extensions/docker.rhai"),
    },
];

/// What a script's file is called.
const EXTENSION: &str = "rhai";

/// Where the person's own extensions are kept: `~/.moonreview/extensions`.
///
/// None under test, so a test sees the shipped extensions and nothing that happens to be
/// installed on the machine running it.
fn user_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        Some(crate::settings::moonreview_dir()?.join("extensions"))
    }
}

/// Every extension there is, by name: the shipped ones, and the person's, which take the place
/// of a shipped one of the same name.
///
/// Read from disk on every call. It is a directory of a few small files, and reading it again
/// is what makes a newly written extension show up in the palette without a restart.
pub(crate) fn all() -> Vec<Extension> {
    let mut extensions: Vec<Extension> = SHIPPED
        .iter()
        .map(|shipped| Extension {
            name: shipped.name.to_string(),
            about: about_of(shipped.source),
            source: Source::Shipped(shipped.source),
        })
        .collect();

    for path in user_files() {
        let Some(name) = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
        else {
            continue;
        };
        // A file that cannot be read is listed anyway, with nothing to say about itself: the
        // pane it opens is where the read failing is reported.
        let about = std::fs::read_to_string(&path)
            .map(|source| about_of(&source))
            .unwrap_or_default();
        extensions.retain(|extension| extension.name != name);
        extensions.push(Extension {
            name,
            about,
            source: Source::File(path),
        });
    }

    extensions.sort_by(|one, other| one.name.cmp(&other.name));
    extensions
}

pub(crate) fn named(name: &str) -> Option<Extension> {
    all().into_iter().find(|extension| extension.name == name)
}

/// The `.rhai` files of [`user_dir`].
fn user_files() -> Vec<PathBuf> {
    let Some(dir) = user_dir() else {
        return Vec::new();
    };
    // No folder is the ordinary case: nobody has written an extension yet.
    let Ok(listed) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    listed
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == EXTENSION)
        })
        .collect()
}

/// The `//!` lines a script starts with, as one line of prose.
fn about_of(source: &str) -> String {
    source
        .lines()
        .map_while(|line| line.strip_prefix("//!"))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}
