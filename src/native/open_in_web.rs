//! Tools › Open in Web: this window's repo in a browser, as the page `moon serve` hands out at
//! `/moon` - see `crate::server::web_page`. The page is on the server this window reviews
//! through: the one it carries, or the one it was pointed at with `--remote`. It opens on the
//! repo and the frame this window is on, logged in already: the address carries a login ticket
//! in its fragment, good for [`OPEN_IN_WEB_TICKET_LIFETIME`] and one login, which the page's
//! login redeems and takes off the address - see `src/server/login.html`. Never a pass key: the
//! address goes through the arguments of whatever opens the browser, and into its session
//! restore, where a ticket is soon worth nothing.

use crate::{backend::remote::urlencode, pass_keys::OPEN_IN_WEB_TICKET_LIFETIME};

use super::app::App;

impl App {
    pub(crate) fn open_in_web(&mut self) {
        let Some(repo) = self.model.root_repo_path() else {
            self.model.error("no repo is open yet to open in a browser");
            return;
        };
        let server = self
            .backend()
            .connect_target()
            .map_or_else(crate::api::server_url, |target| target.address);
        let ticket = match self
            .backend()
            .mint_login_ticket(OPEN_IN_WEB_TICKET_LIFETIME)
        {
            Ok(ticket) => ticket,
            Err(error) => {
                self.model.error(format!(
                    "could not make a login ticket to open a browser with: {error:#}"
                ));
                return;
            }
        };
        let address = format!(
            "{server}/moon/?repo={}&frame={}#ticket={ticket}",
            urlencode(&repo.to_string_lossy()),
            urlencode(self.frame().subcommand()),
        );
        if let Err(error) = webbrowser::open(&address) {
            self.model.error(format!(
                "could not open a browser on {server}/moon: {error}"
            ));
        }
    }
}
