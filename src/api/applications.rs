//! Installed applications API types - what a server answers when a window asks which
//! applications with windows it has installed. The server side is `crate::desktop_entries`,
//! which reads them; the window side is `crate::native::applications_offered`, which is what
//! `moon › Applications` and the palette list.

use serde::{Deserialize, Serialize};

/// What `GET /api/applications` answers.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstalledApplications {
    /// A server on Linux: what its desktop entries offer a menu, sorted by name.
    Listed(Vec<InstalledApplication>),
    /// A server off Linux, which has no desktop entries to read - and no desktop to start
    /// an application on either, see `crate::display`. Said rather than answered as an empty
    /// list, which would read as a machine with nothing installed.
    NotOnLinux,
}

/// An application installed on the server, as its desktop entry describes it.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub(crate) struct InstalledApplication {
    /// The entry's `Name`, in the server's language where the entry has it.
    pub(crate) name: String,
    /// The line of shell that starts it: the entry's `Exec`, opened on no file.
    pub(crate) command: String,
    /// The entry's `Categories` as it lists them - `Network`, `WebBrowser` - which a menu
    /// groups it by.
    pub(crate) categories: Vec<String>,
}
