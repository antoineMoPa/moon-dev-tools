//! Tools › Show QR Code: this page on another device, logged in. A QR code of the address
//! the page was served from - which, on a hosted instance, is one a phone reaches too - on
//! the repo and the frame this window is on.
//!
//! The ticket in the code is the server's to make, so the page asks for it - see
//! `POST /api/login-ticket` - and draws the code once it has one. The code already up stays
//! up while the next ticket is on its way.

use web_time::Instant;

use crate::native::{app::App, login_link};

/// One ticket asked for at a time, however many frames are drawn while it is on its way.
const TICKET_TASK_KEY: &str = "page-qr-ticket";

const WINDOW_TITLE: &str = "open on another device";

/// The window with the QR code, and the ticket in the code.
#[derive(Default)]
pub(crate) struct PageQr {
    /// Whether the window is up.
    showing: bool,
    /// The ticket the code holds, and when it was minted.
    ticket: Option<(String, Instant)>,
}

/// The address this page was served from: the server's, as the browser reached it.
fn page_origin() -> String {
    web_sys::window()
        .expect("the window is drawn in a page")
        .location()
        .origin()
        .expect("a page served over http has an origin")
}

impl App {
    /// Put the window with the QR code up.
    pub(crate) fn show_page_qr(&mut self) {
        self.model.page_qr.showing = true;
    }

    /// The window with this server's address and the QR code that opens it on another device.
    pub(crate) fn draw_page_qr(&mut self, ctx: &egui::Context) {
        if !self.model.page_qr.showing {
            return;
        }
        let Some(repo) = self.model.root_repo_path() else {
            self.model.page_qr.showing = false;
            self.model
                .error("no repo is open yet to open on another device");
            return;
        };
        let stale = self
            .model
            .page_qr
            .ticket
            .as_ref()
            .is_none_or(|(_, minted)| login_link::qr_ticket_wants_renewing(*minted));
        if stale {
            self.tasks.spawn_keyed(
                Some(TICKET_TASK_KEY.to_string()),
                |backend| backend.mint_login_ticket(login_link::QR_TICKET_LIFETIME),
                |model, minted| match minted {
                    Ok(ticket) => model.page_qr.ticket = Some((ticket, Instant::now())),
                    Err(error) => {
                        model.page_qr = PageQr::default();
                        model.error(format!(
                            "could not make a login ticket for the other device: {error:#}"
                        ));
                    }
                },
            );
        }
        let Some((ticket, _)) = &self.model.page_qr.ticket else {
            return;
        };

        let origin = page_origin();
        let address = login_link::address(&origin, &repo, self.frame(), ticket);
        let palette = self.palette_of();
        let open = login_link::qr_window(ctx, &palette, WINDOW_TITLE, |ui| {
            ui.label(egui::RichText::new(&origin).monospace());
            ui.add_space(6.0);
            login_link::paint_qr(ui, &palette, &address);
        });
        if !open {
            // The ticket goes with the window: one put back up later starts from a new one
            // rather than from a code that ran out while nobody was looking.
            self.model.page_qr = PageQr::default();
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
}
