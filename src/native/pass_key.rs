//! Tools › Generate Pass Key: a key to the server this window reviews through - see
//! `crate::pass_keys` - put on the clipboard, for a browser's login page or another machine's
//! `--pass-key`. It is a key to the same server `Open in Web` opens a browser on: the one this
//! window carries, or the one it was pointed at with `--remote`.

use super::app::App;

impl App {
    pub(crate) fn generate_pass_key(&mut self, ctx: &egui::Context) {
        match self.backend().mint_pass_key() {
            Ok(key) => {
                ctx.copy_text(key);
                self.model.info("a new pass key is on the clipboard");
            }
            Err(error) => self
                .model
                .error(format!("could not make a pass key: {error:#}")),
        }
    }
}
