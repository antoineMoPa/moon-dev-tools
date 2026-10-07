//! Login link - builds a link that logs a browser in: the page `moon serve` serves at `/moon`,
//! on the repo and the frame a window is on, with a login ticket in its fragment. See
//! `src/server/login.html`, which redeems the ticket and takes it off the address.
//!
//! As a QR code, the link gets to a device that has no keyboard worth typing an address on.
//! The code is drawn by the window of a tunnel - see `crate::native::tunnel` - and, in a
//! browser, by the window of the page's own server - see [`page_qr`]. A ticket in a code is
//! good for [`QR_TICKET_LIFETIME`], and a code sits on the screen for as long as it takes to
//! find the device, so whoever draws one makes it again from a new ticket before the old one
//! runs out - see [`qr_ticket_wants_renewing`].

#[cfg(target_arch = "wasm32")]
pub(crate) mod page_qr;

use std::{path::Path, time::Duration};

use egui::{Align2, Color32, RichText, Sense, vec2};
use web_time::Instant;

use crate::{backend::remote::urlencode, cli::Frame, native::theme::Palette};

/// How long the ticket in a QR code is good for: the time it takes to point a camera at it.
pub(crate) const QR_TICKET_LIFETIME: Duration = Duration::from_secs(60);

/// A new ticket is minted this long before the one in the QR code runs out.
const QR_TICKET_REFRESH_MARGIN: Duration = Duration::from_secs(15);

const QR_SIZE: f32 = 240.0;

/// Modules of empty margin a QR code needs around it to be read.
const QR_QUIET_ZONE: usize = 4;

/// The link: `server`'s page on `repo` and `frame`, which `ticket` logs into. `server` is an
/// origin - `https://moon.example.com` - with no slash after it.
pub(crate) fn address(server: &str, repo: &Path, frame: Frame, ticket: &str) -> String {
    format!(
        "{server}/moon/?repo={}&frame={}#ticket={ticket}",
        urlencode(&repo.to_string_lossy()),
        urlencode(frame.subcommand()),
    )
}

/// Whether the ticket of a QR code, minted at `minted`, is close enough to running out that
/// the code is to be made again.
pub(crate) fn qr_ticket_wants_renewing(minted: Instant) -> bool {
    minted.elapsed() + QR_TICKET_REFRESH_MARGIN >= QR_TICKET_LIFETIME
}

/// The window a QR code is shown in: `title`, a close mark, and `body` centered under them.
/// Says whether it is still up, which it is until its close mark is pressed.
pub(crate) fn qr_window(
    ctx: &egui::Context,
    palette: &Palette,
    title: &str,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut open = true;
    egui::Window::new(title)
        // Its own header rather than the window's: the close button of that one shows an
        // arrow where everything else clickable shows a hand, and is framed where ours are
        // not.
        .title_bar(false)
        // Above the board: it takes a press on anything at the middle layer or below for its
        // own, which is how a press on this window's title would pick a card up.
        .order(egui::Order::Foreground)
        // Where it opens, and no more: an anchored window could not be dragged.
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.viewport_rect().center())
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::native::widgets::close_button(ui, palette).clicked() {
                        open = false;
                    }
                });
            });
            ui.separator();
            ui.vertical_centered(body);
        });
    open
}

/// The QR code of `address`, black on white whatever the theme: that is what a camera reads.
pub(crate) fn paint_qr(ui: &mut egui::Ui, palette: &Palette, address: &str) {
    let code = match qrcode::QrCode::new(address.as_bytes()) {
        Ok(code) => code,
        Err(error) => {
            ui.label(
                RichText::new(format!("could not make a QR code: {error}")).color(palette.warn),
            );
            return;
        }
    };
    let modules = code.width();
    let colors = code.to_colors();
    let (rect, _) = ui.allocate_exact_size(vec2(QR_SIZE, QR_SIZE), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let cell = QR_SIZE / (modules + 2 * QR_QUIET_ZONE) as f32;
    for (index, color) in colors.iter().enumerate() {
        if *color != qrcode::Color::Dark {
            continue;
        }
        let x = (index % modules + QR_QUIET_ZONE) as f32 * cell;
        let y = (index / modules + QR_QUIET_ZONE) as f32 * cell;
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min + vec2(x, y), vec2(cell, cell)).expand(0.25),
            0.0,
            Color32::BLACK,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repo and the frame are spelled so they survive an address, and the ticket rides in
    /// the fragment, which a browser never sends to a server.
    #[test]
    fn the_link_names_the_repo_and_frame_and_carries_the_ticket_in_its_fragment() {
        assert_eq!(
            address(
                "https://moon.example.com",
                Path::new("/home/dev/my repo"),
                Frame::Tasks,
                "t.payload.mac",
            ),
            format!(
                "https://moon.example.com/moon/?repo=%2Fhome%2Fdev%2Fmy%20repo&frame={}#ticket=t.payload.mac",
                Frame::Tasks.subcommand()
            )
        );
    }

    #[test]
    fn a_ticket_is_renewed_before_it_runs_out_and_not_as_soon_as_it_is_minted() {
        let now = Instant::now();
        assert!(!qr_ticket_wants_renewing(now));
        assert!(qr_ticket_wants_renewing(
            now - (QR_TICKET_LIFETIME - QR_TICKET_REFRESH_MARGIN)
        ));
    }
}
