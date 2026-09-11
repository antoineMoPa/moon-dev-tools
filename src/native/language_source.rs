//! The window's answer to the questions an editor puts to a language server.
//!
//! [`egui_moon_code_ide::LanguageSource`] is a trait rather than a registry precisely
//! because of this window: a review of a repo on another machine has no files here to start a
//! server on, so [`crate::backend::Backend`] carries the same questions over HTTP and the
//! servers run beside the repo. A local review calls straight through instead. Which of the
//! two is in play is the backend's business and nothing above it can tell.
//!
//! It is built inside a worker closure rather than held anywhere, because that is where a
//! `&dyn Backend` exists and where a call is allowed to block - see
//! [`crate::native::tasks`]. The pieces of the crate this window uses run on the window's own
//! threads, so it drives them itself rather than letting the crate own one.

use anyhow::Result;
use egui_moon_code_ide::{
    LanguageSource, LspCompletion, LspFileEdit, LspLocation, LspPosition, LspStatus,
};

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
    /// of having no server already reads as: nothing more is sent about it, and a ⌘-click in it
    /// says there is nothing behind the file rather than waiting on an answer nobody will give.
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

    fn places(
        &self,
        file_path: &str,
        at: LspPosition,
        which: egui_moon_code_ide::LspPlaces,
    ) -> Result<Vec<LspLocation>> {
        self.backend
            .lsp_places(self.session_id, file_path, at, which)
    }

    fn completion(&self, file_path: &str, at: LspPosition) -> Result<Vec<LspCompletion>> {
        self.backend.lsp_completion(self.session_id, file_path, at)
    }

    fn prepare_rename(&self, file_path: &str, at: LspPosition) -> Result<Option<String>> {
        self.backend
            .lsp_prepare_rename(self.session_id, file_path, at)
    }

    fn rename(&self, file_path: &str, at: LspPosition, new_name: &str) -> Result<Vec<LspFileEdit>> {
        self.backend
            .lsp_rename(self.session_id, file_path, at, new_name)
    }

    fn format(
        &self,
        file_path: &str,
        options: egui_moon_code_ide::LspFormatting,
    ) -> Result<Vec<egui_moon_code_ide::LspTextEdit>> {
        self.backend.lsp_format(self.session_id, file_path, options)
    }

    fn hover(&self, file_path: &str, at: LspPosition) -> Result<Option<String>> {
        self.backend.lsp_hover(self.session_id, file_path, at)
    }

    fn diagnostics(&self, file_path: &str) -> Result<Vec<egui_moon_code_ide::LspDiagnostic>> {
        self.backend.lsp_diagnostics(self.session_id, file_path)
    }

    fn did_save(&self, file_path: &str) -> Result<()> {
        self.backend.lsp_did_save(self.session_id, file_path)
    }

    fn code_actions(
        &self,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Vec<egui_moon_code_ide::LspCodeAction>> {
        self.backend
            .lsp_code_actions(self.session_id, file_path, at)
    }

    fn signature_help(
        &self,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Option<egui_moon_code_ide::LspSignature>> {
        self.backend
            .lsp_signature_help(self.session_id, file_path, at)
    }

    /// What the server behind this file said opens a completion list on its own, carried
    /// over the backend like everything else - a `--remote` review asks the server sitting
    /// beside the repo, which is the only side that has one.
    ///
    /// A list that could not be had is a file that opens no list of its own, which is what
    /// every other way of having no triggers already reads as: nothing serves the file, the
    /// server named none, or the call did not land. The trait has no error to report here
    /// and the pane's next move is the same for all three - go on offering to finish words -
    /// so the error is dropped here rather than carried up to be dropped there. The pane
    /// asks this once, and once its file's server is `Ready`, so the answer it keeps is one
    /// a started server really gave.
    fn trigger_characters(&self, file_path: &str) -> Vec<char> {
        self.backend
            .lsp_trigger_characters(self.session_id, file_path)
            .unwrap_or_default()
    }
}
