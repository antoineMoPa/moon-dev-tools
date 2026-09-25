//! Putting this window's project down and going back to its launch screen, to open another -
//! Project > Switch Project.
//!
//! A new window would do the same on this machine, but a remote window's new window is a
//! second process asked for a pass key it has to be handed, and a browser's has no process to
//! start at all. Going back in place keeps the connection the window already has, so the
//! launch screen offers the recent projects of whichever machine the window reads.

use std::sync::Arc;

use crate::native::Launch;

use super::App;

impl App {
    /// Take the window back to its launch screen, on the same backend it is connected to.
    ///
    /// Everything of the project goes with it - its tabs, its reviews, its board - and is built
    /// again from nothing when the next one opens, the way a window that just started would.
    /// The shells are not killed: they belong to the server rather than to the tabs, and a
    /// window that opens their project again takes them back up.
    ///
    /// What belongs to the window rather than to the project is kept: the menu bar, the
    /// socket `moon open` reaches it on, the theme, and the arrangement of its frames, which
    /// the next project opens in the way a restarted window opens in the last one's.
    ///
    /// A file with edits that are not on disk refuses the switch, since the switch would throw
    /// them away without a tab left to close twice.
    pub(crate) fn switch_project(&mut self, ctx: &egui::Context) {
        let unsaved: Vec<&str> = self
            .model
            .file_editors
            .values()
            .filter(|editor| editor.is_dirty())
            .map(|editor| editor.file_path.as_str())
            .collect();
        if !unsaved.is_empty() {
            self.model.error(format!(
                "unsaved edits in {} - save or close them to switch project",
                unsaved.join(", ")
            ));
            return;
        }

        let fresh = App::new(
            ctx.clone(),
            Launch {
                backend: Arc::clone(self.backend()),
                open: None,
                frame: self.frame,
            },
        );
        let mut left = std::mem::replace(self, fresh);

        self.set_theme(left.model.theme);
        self.model.restored_layout = Some(left.model.layout);
        self.asks_language_servers = left.asks_language_servers;
        // Installing either twice would stack a second copy on the context.
        self.loaders_installed = left.loaders_installed;
        self.fonts_installed = left.fonts_installed;
        // What the title bar says now, which names the project left: the fresh one's would
        // read as already said, and the bar would go on naming it.
        self.window_title = left.window_title;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.menu = left.menu.take();
            self.shell_asks = left.shell_asks.take();
            self.sessions_for_asked_files = Arc::clone(&left.sessions_for_asked_files);
            self.window_is_in_front = left.window_is_in_front;
        }
        ctx.request_repaint();
    }
}
