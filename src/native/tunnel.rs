//! Cloudflare tunnel - Tools › Start Cloudflare Tunnel makes this window's server reachable
//! from another device through a quick tunnel: `cloudflared tunnel --url`, which needs no
//! Cloudflare account and gives a random `https://….trycloudflare.com` address that forwards to
//! the server.
//!
//! The address alone lets nobody in: the server still asks for a login. The QR code carries
//! the same address `Open in Web` opens, with a login ticket in its fragment - see
//! [`crate::native::login_link`] - so a device that scans it is logged in, and the code is
//! made again from a new ticket before the old one runs out.

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{Mutex, Once, mpsc},
    time::{Duration, Instant},
};

use egui::{RichText, Sense};

use crate::pass_keys::{OPEN_IN_WEB_TICKET_LIFETIME, SERVE_TICKET_LIFETIME};

use super::{app::App, login_link, model::ToastKind};

const CLOUDFLARED: &str = "cloudflared";

/// The address in front of every tunnel quick tunnels are given.
const QUICK_TUNNEL_HOST_SUFFIX: &str = ".trycloudflare.com";

/// A `cloudflared` running, and what is known of the address it was given.
pub(crate) struct Tunnel {
    process: Child,
    addresses: mpsc::Receiver<String>,
    /// `https://….trycloudflare.com`, once `cloudflared` has printed it.
    public_url: Option<String>,
    /// The address the QR code holds, and when its ticket was minted.
    link: Option<(String, Instant)>,
    /// Whether the window with the QR code is up.
    showing_qr: bool,
}

/// The processes of the tunnels running, for the exit that never drops a [`Tunnel`]: a
/// process that ends through `exit` runs no destructors of what is still alive in `main`.
static RUNNING_PROCESSES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

extern "C" fn stop_tunnels_at_exit() {
    // A lock poisoned by a panic still holds the list, and there is no time to be fussy.
    let running = RUNNING_PROCESSES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for pid in running.iter() {
        // SAFETY: `kill` on a process id and a signal touches no memory of ours.
        unsafe { libc::kill(*pid as libc::pid_t, libc::SIGTERM) };
    }
}

impl Tunnel {
    fn started(process: Child, addresses: mpsc::Receiver<String>) -> Self {
        static AT_EXIT: Once = Once::new();
        // SAFETY: registers a plain function that takes and returns nothing.
        AT_EXIT.call_once(|| unsafe {
            libc::atexit(stop_tunnels_at_exit);
        });
        RUNNING_PROCESSES
            .lock()
            .expect("the list of tunnel processes was poisoned")
            .push(process.id());
        Self {
            process,
            addresses,
            public_url: None,
            link: None,
            showing_qr: false,
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // A tunnel left running would go on forwarding to the server after the window that
        // asked for it is gone.
        let _ = self.process.kill();
        let _ = self.process.wait();
        let id = self.process.id();
        if let Ok(mut running) = RUNNING_PROCESSES.lock() {
            running.retain(|pid| *pid != id);
        }
    }
}

/// The quick tunnel's address in one line of what `cloudflared` prints, which is a box drawn
/// around it, a log line, or both.
fn public_url_in(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '|')
        .unwrap_or(rest.len());
    let url = &rest[..end];
    url.ends_with(QUICK_TUNNEL_HOST_SUFFIX)
        .then(|| url.to_string())
}

impl App {
    /// Stop the tunnel.
    pub(crate) fn stop_tunnel(&mut self) {
        if self.tunnel.take().is_some() {
            self.model
                .log_message(ToastKind::Info, "cloudflare tunnel stopped");
        } else {
            self.model
                .log_message(ToastKind::Info, "no cloudflare tunnel is running");
        }
    }

    /// Open the tunnel's address in this machine's browser, logged in like the QR code is.
    pub(crate) fn open_tunnel_in_browser(&mut self) {
        let Some(public_url) = self.tunnel.as_ref().map(|tunnel| tunnel.public_url.clone()) else {
            self.model.log_message(
                ToastKind::Error,
                "no cloudflare tunnel is running. Start one first!",
            );
            return;
        };
        let Some(public_url) = public_url else {
            self.model.log_message(
                ToastKind::Info,
                "the cloudflare tunnel has no address yet, try again in a moment",
            );
            return;
        };
        let Some(address) = self.tunnel_address(&public_url, OPEN_IN_WEB_TICKET_LIFETIME) else {
            return;
        };
        if let Err(error) = webbrowser::open(&address) {
            self.model.log_message(
                ToastKind::Error,
                format!("could not open a browser on {public_url}/moon: {error}"),
            );
        }
    }

    /// Put the window with the QR code back up, after it was closed.
    pub(crate) fn view_tunnel_link(&mut self) {
        match &mut self.tunnel {
            Some(tunnel) if tunnel.public_url.is_some() => tunnel.showing_qr = true,
            Some(_) => self.model.log_message(
                ToastKind::Info,
                "the cloudflare tunnel has no address yet, try again in a moment",
            ),
            None => self.model.log_message(
                ToastKind::Error,
                "no cloudflare tunnel is running. Start one first!",
            ),
        }
    }

    /// Start the tunnel. A second one would be a second address to the same server, so with
    /// one running already this says so.
    pub(crate) fn start_tunnel(&mut self) {
        if self.tunnel.is_some() {
            self.model.log_message(
                ToastKind::Info,
                "a cloudflare tunnel is already running. View its link from the menu",
            );
            return;
        }
        let server = self
            .backend()
            .connect_target()
            .map_or_else(crate::api::server_url, |target| target.address);
        let mut process = match Command::new(CLOUDFLARED)
            .args(["tunnel", "--no-autoupdate", "--url", &server])
            // A window started from a launcher has a bare PATH, without Homebrew's.
            .env("PATH", crate::shell_path::installed_tools_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(process) => process,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.model.log_message(
                    ToastKind::Error,
                    "cloudflared is not installed. Install it and try again!",
                );
                return;
            }
            Err(error) => {
                self.model.log_message(
                    ToastKind::Error,
                    format!("could not start cloudflared: {error}"),
                );
                return;
            }
        };
        let stderr = process
            .stderr
            .take()
            .expect("stderr was piped when the process was spawned");
        let (sender, addresses) = mpsc::channel();
        std::thread::spawn(move || {
            let mut sent = false;
            // Read to the end even once the address is out: a pipe nobody reads fills up, and
            // `cloudflared` blocks on its logging.
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !sent && let Some(url) = public_url_in(&line) {
                    sent = sender.send(url).is_ok();
                }
            }
        });
        self.tunnel = Some(Tunnel::started(process, addresses));
        self.model
            .log_message(ToastKind::Info, "starting a cloudflare tunnel…");
    }

    /// Notice the address arriving, and the process ending.
    pub(crate) fn poll_tunnel(&mut self, ctx: &egui::Context) {
        let Some(tunnel) = &mut self.tunnel else {
            return;
        };
        if let Ok(url) = tunnel.addresses.try_recv() {
            self.model
                .log_message(ToastKind::Info, format!("cloudflare tunnel: {url}"));
            tunnel.public_url = Some(url);
            tunnel.showing_qr = true;
        }
        match tunnel.process.try_wait() {
            Ok(None) => ctx.request_repaint_after(Duration::from_millis(500)),
            Ok(Some(status)) => {
                self.tunnel = None;
                self.model
                    .log_message(ToastKind::Error, format!("cloudflared stopped: {status}"));
            }
            Err(error) => {
                self.tunnel = None;
                self.model.log_message(
                    ToastKind::Error,
                    format!("could not tell whether cloudflared is running: {error}"),
                );
            }
        }
    }

    /// The window with the tunnel's address and the QR code that opens it on another device.
    pub(crate) fn draw_tunnel(&mut self, ctx: &egui::Context) {
        let Some(tunnel) = &self.tunnel else {
            return;
        };
        let (Some(public_url), true) = (tunnel.public_url.clone(), tunnel.showing_qr) else {
            return;
        };
        let stale = tunnel
            .link
            .as_ref()
            .is_none_or(|(_, minted)| login_link::qr_ticket_wants_renewing(*minted));
        if stale {
            let Some(address) = self.tunnel_address(&public_url, login_link::QR_TICKET_LIFETIME)
            else {
                return;
            };
            if let Some(tunnel) = &mut self.tunnel {
                tunnel.link = Some((address, Instant::now()));
            }
        }
        let Some((address, _)) = self.tunnel.as_ref().and_then(|tunnel| tunnel.link.clone()) else {
            return;
        };

        let palette = self.palette_of();
        let mut stop = false;
        let mut open_in_browser = false;
        let mut copy_link = false;
        let open = login_link::qr_window(ctx, &palette, "remote access", |ui| {
            // The address a phone would scan, opened here: logged in, like the QR code.
            let link = crate::native::widgets::clickable(
                ui.add(
                    egui::Label::new(
                        RichText::new(&public_url)
                            .monospace()
                            .color(palette.accent)
                            .underline(),
                    )
                    .sense(Sense::click()),
                ),
            );
            if link.clicked() {
                open_in_browser = true;
            }
            link.context_menu(|ui| {
                if crate::native::widgets::quiet_button(ui, "copy link").clicked() {
                    copy_link = true;
                    ui.close();
                }
            });
            ui.add_space(6.0);
            login_link::paint_qr(ui, &palette, &address);
            ui.add_space(6.0);
            if crate::native::widgets::clickable(ui.button("stop tunnel")).clicked() {
                stop = true;
            }
        });
        // Closing the window only puts the code away: the tunnel is stopped from the menu or
        // its button.
        if let Some(tunnel) = &mut self.tunnel {
            tunnel.showing_qr = open;
        }
        if stop {
            self.stop_tunnel();
        } else if open_in_browser {
            self.open_tunnel_in_browser();
        } else if copy_link {
            self.copy_tunnel_link(ctx, &public_url);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    /// Put a link on the clipboard, to send to someone or something. Not the QR code's: that
    /// one is good for a minute, which is the time to scan it, and a link that is pasted in a
    /// message is opened later than that. This one is good for the longest a ticket can be,
    /// and for one login all the same.
    fn copy_tunnel_link(&mut self, ctx: &egui::Context, public_url: &str) {
        let Some(address) = self.tunnel_address(public_url, SERVE_TICKET_LIFETIME) else {
            return;
        };
        ctx.copy_text(address);
        self.model.log_message(
            ToastKind::Info,
            format!(
                "copied a link that logs one browser in, good for {} minutes",
                SERVE_TICKET_LIFETIME.as_secs() / 60
            ),
        );
    }

    /// What a remote device opens: the page `Open in Web` opens, on the tunnel's address.
    fn tunnel_address(&mut self, public_url: &str, lifetime: Duration) -> Option<String> {
        let Some(repo) = self.model.root_repo_path() else {
            self.model
                .log_message(ToastKind::Error, "no repo is open yet to open remotely");
            return None;
        };
        let ticket = match self.backend().mint_login_ticket(lifetime) {
            Ok(ticket) => ticket,
            Err(error) => {
                self.model.log_message(
                    ToastKind::Error,
                    format!("could not make a login ticket for the remote device: {error:#}"),
                );
                return None;
            }
        };
        Some(login_link::address(public_url, &repo, self.frame(), &ticket))
    }
}

#[cfg(test)]
mod tests {
    use super::{Tunnel, public_url_in, stop_tunnels_at_exit};

    #[test]
    fn an_exit_that_drops_nothing_still_stops_the_tunnel() {
        let process = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("failed to start sleep");
        let (_sender, addresses) = std::sync::mpsc::channel();
        let mut tunnel = Tunnel::started(process, addresses);

        stop_tunnels_at_exit();

        let status = tunnel.process.wait().expect("failed to wait for sleep");
        assert!(
            !status.success(),
            "the process was still running to its end"
        );
    }

    #[test]
    fn the_address_is_found_in_the_box_cloudflared_draws_around_it() {
        assert_eq!(
            public_url_in(
                "2026-09-30T10:00:00Z INF |  https://quiet-lake-1234.trycloudflare.com  |"
            ),
            Some("https://quiet-lake-1234.trycloudflare.com".to_string())
        );
    }

    #[test]
    fn other_addresses_cloudflared_logs_are_not_the_tunnel() {
        assert_eq!(
            public_url_in("INF Requesting new quick Tunnel on https://api.trycloudflare.com..."),
            None
        );
        assert_eq!(
            public_url_in("INF Thank you for trying Cloudflare Tunnel"),
            None
        );
    }
}
