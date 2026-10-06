//! The wire in a window - see `crate::moontasks::wire` for what it is.
//!
//! `Tools › Wire` and the palette's `wire` open the file its broadcasts are kept in, in a tab
//! that follows the file as agents post and that nothing can be saved from: `moon wire post`
//! is the file's only writer, and the tab says so - see `FileEditor::only_written_by`.
//!
//! The other half is the direct messages: lines a shell asked this window to type into one of
//! the shells it holds. Each is handed to the shell, which types it once nothing there is in
//! the way, and written in the window's Messages - what one agent said to another is there to
//! be read back, and nowhere else. Written there and not put up as a toast: agents talk among
//! themselves a good deal, and none of it is said to the person at the window.

use crate::native::app::App;

impl App {
    /// Open the file the wire's broadcasts are kept in, in the tab already on it or a new
    /// one. The file is made on the way on a board nobody has posted to yet, which is a call
    /// to the repo's side, so the tab opens once that comes back.
    pub(crate) fn open_wire(&mut self) {
        let session_id = self.model.root_session_id.clone();
        let for_call = session_id.clone();
        self.tasks.spawn(
            move |backend| backend.open_wire(&for_call),
            move |model, result| match result {
                Ok(file_path) => {
                    crate::native::file_pane::tab_on_file(model, session_id, file_path);
                }
                Err(error) => model.error(format!("could not open the wire: {error}")),
            },
        );
    }

    /// Hand the direct messages that arrived for this window's shells to those shells, in the
    /// order they arrived, and write each in the Messages. One that could not be handed over
    /// is a toast as well, since the agent that sent it was told it had gone.
    ///
    /// Written down as it is handed over, which is before it is typed: a shell holds a line
    /// back while its agent is asking for a person or somebody is typing in it - see `told`
    /// in `crate::terminal`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn type_wired_lines(&mut self, wired: Vec<crate::instances::window::WiredLine>) {
        use crate::{
            moontasks::wire::{logged_line, typed_line},
            native::model::ToastKind,
        };

        for line in wired {
            let logged = logged_line(&line.sender, &line.recipient, &line.message);
            let typed = typed_line(&line.sender, &line.message);
            match self.backend().tell_terminal(&line.terminal_id, &typed) {
                Ok(()) => self.model.log_message(ToastKind::Info, logged),
                Err(error) => self
                    .model
                    .error(format!("could not deliver {logged}: {error}")),
            }
        }
    }
}
