//! The window's answer to the six questions an editor puts to a language server.
//!
//! [`egui_moon_code_ide::LanguageSource`] is a trait rather than a registry precisely
//! because of this window: a review of a repo on another machine has no files here to start a
//! server on, so [`crate::backend::Backend`] carries the same six questions over HTTP and the
//! servers run beside the repo. A local review calls straight through instead. Which of the
//! two is in play is the backend's business and nothing above it can tell.
//!
//! It is built inside a worker closure rather than held anywhere, because that is where a
//! `&dyn Backend` exists and where a call is allowed to block - see
//! [`crate::native::tasks`]. The pieces of the crate this window uses run on the window's own
//! threads, so it drives them itself rather than letting the crate own one.

use anyhow::Result;
use egui_moon_code_ide::{LanguageSource, LspCompletion, LspLocation, LspPosition, LspStatus};

use crate::backend::Backend;

/// One review session's language servers, reached through the backend.
pub(crate) struct SessionLanguages<'a> {
    backend: &'a dyn Backend,
    session_id: &'a str,
}

impl<'a> SessionLanguages<'a> {
    pub(crate) fn new(backend: &'a dyn Backend, session_id: &'a str) -> Self {
        Self {
            backend,
            session_id,
        }
    }
}

impl LanguageSource for SessionLanguages<'_> {
    /// A status that could not be had is a file with no server, which is what every other way
    /// of having no server already reads as: nothing more is sent about it, and the ⌘-click
    /// falls through to the repo search the way it does for markdown.
    fn status(&self, file_path: &str) -> LspStatus {
        self.backend
            .lsp_status(self.session_id, file_path)
            .unwrap_or(LspStatus::Unavailable)
    }

    fn did_open(&self, file_path: &str, text: &str) -> Result<()> {
        self.backend.lsp_did_open(self.session_id, file_path, text)
    }

    fn did_change(&self, file_path: &str, text: &str) -> Result<()> {
        self.backend
            .lsp_did_change(self.session_id, file_path, text)
    }

    fn did_close(&self, file_path: &str) -> Result<()> {
        self.backend.lsp_did_close(self.session_id, file_path)
    }

    fn definition(&self, file_path: &str, at: LspPosition) -> Result<Vec<LspLocation>> {
        self.backend
            .lsp_definition(self.session_id, file_path, at)
    }

    fn completion(&self, file_path: &str, at: LspPosition) -> Result<Vec<LspCompletion>> {
        self.backend
            .lsp_completion(self.session_id, file_path, at)
    }
}
