//! Which language server serves which file, as two constant tables.
//!
//! Adding a language is a row in [`EXTENSIONS`], and a [`ServerSpec`] beside the others if
//! its server is a new one. Nothing else in this module knows the name of a language, so
//! that row is the whole of the change.
//!
//! The two tables are separate because the mapping is not one to one: `.ts`, `.tsx`, `.js`
//! and `.jsx` are four languages as far as the protocol is concerned - each has its own
//! `languageId` - and one server as far as the machine is concerned, so a project mixing
//! them is indexed once rather than four times.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// One language server: what it is called here, and how it is started. The command speaks
/// the protocol on its stdin and stdout, which every server in the table below does.
pub(crate) struct ServerSpec {
    /// What the server is held under in the registry, and what a message about it reads as.
    pub(crate) name: &'static str,
    pub(crate) command: &'static str,
    pub(crate) args: &'static [&'static str],
}

/// One file extension: what the protocol calls that language, and the server behind it.
pub(crate) struct ExtensionSpec {
    pub(crate) extension: &'static str,
    /// The protocol's `languageId` - `typescriptreact` rather than `typescript` for a
    /// `.tsx`, which is the difference between a server parsing JSX and choking on it.
    pub(crate) language_id: &'static str,
    pub(crate) server: &'static ServerSpec,
}

pub(crate) const RUST_ANALYZER: ServerSpec = ServerSpec {
    name: "rust",
    command: "rust-analyzer",
    args: &[],
};

pub(crate) const TYPESCRIPT: ServerSpec = ServerSpec {
    name: "typescript",
    command: "typescript-language-server",
    args: &["--stdio"],
};

pub(crate) const PYRIGHT: ServerSpec = ServerSpec {
    name: "python",
    command: "pyright-langserver",
    args: &["--stdio"],
};

/// Every extension a server is offered for. An extension that is not here has no server
/// behind it, which is most of a repo - markdown, configuration, images.
pub(crate) const EXTENSIONS: &[ExtensionSpec] = &[
    ExtensionSpec {
        extension: "rs",
        language_id: "rust",
        server: &RUST_ANALYZER,
    },
    ExtensionSpec {
        extension: "ts",
        language_id: "typescript",
        server: &TYPESCRIPT,
    },
    ExtensionSpec {
        extension: "tsx",
        language_id: "typescriptreact",
        server: &TYPESCRIPT,
    },
    ExtensionSpec {
        extension: "js",
        language_id: "javascript",
        server: &TYPESCRIPT,
    },
    ExtensionSpec {
        extension: "jsx",
        language_id: "javascriptreact",
        server: &TYPESCRIPT,
    },
    ExtensionSpec {
        extension: "mjs",
        language_id: "javascript",
        server: &TYPESCRIPT,
    },
    ExtensionSpec {
        extension: "py",
        language_id: "python",
        server: &PYRIGHT,
    },
];

/// The row for a file, by its extension. `None` for a file no server in the table serves.
pub(crate) fn for_file(file_path: &str) -> Option<&'static ExtensionSpec> {
    let extension = Path::new(file_path).extension()?.to_str()?;
    EXTENSIONS
        .iter()
        .find(|spec| spec.extension.eq_ignore_ascii_case(extension))
}

/// Where a server's command is on this machine, if it is installed at all.
///
/// The PATH is the one the user's login shell has rather than this process's - a window
/// started from a desktop launcher inherits neither homebrew nor `~/.local/bin`, and every
/// server would read as missing. See [`crate::shell_path`], which the agents are found the
/// same way.
///
/// The answer is remembered: a pane asks for a file's status as it draws, and installing a
/// language server while the window is open is not a thing that happens mid-session.
pub(crate) fn installed_at(command: &str) -> Option<PathBuf> {
    static FOUND: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let cache = FOUND.get_or_init(|| Mutex::new(HashMap::new()));

    if let Some(found) = cache.lock().unwrap().get(command) {
        return found.clone();
    }
    let found = look_up_on_path(command);
    cache
        .lock()
        .unwrap()
        .insert(command.to_string(), found.clone());
    found
}

fn look_up_on_path(command: &str) -> Option<PathBuf> {
    std::env::split_paths(crate::shell_path::installed_tools_path())
        .map(|directory| directory.join(command))
        .find(|candidate| is_runnable(candidate))
}

/// A file on PATH that this user can actually run. The permission bits are the difference
/// between a server that is installed and a same-named file that happens to sit beside one.
#[cfg(unix)]
fn is_runnable(candidate: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(candidate)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_runnable(candidate: &Path) -> bool {
    candidate.is_file()
}
