//! Login tickets: what a browser is sent to log in with, in place of a pass key.
//!
//! A link that logs a browser in has to carry something the browser can show the server, and
//! a link travels badly: it is in the arguments of whatever opens the browser (`open`,
//! `xdg-open`), in the terminal `moon serve` printed it to, and in a browser's session restore.
//! A pass key there would be a lasting credential left lying around, so a link carries a ticket
//! instead: good for a minute or a few, and for one login. Redeeming it is what makes the
//! browser's own pass key - see [`crate::server::web_page`] - which then never leaves the
//! cookie.
//!
//! A ticket is an expiring token of its own kind - `t.<payload>.<mac>`, see [`super::expiring`].
//! Checking one is recomputing the MAC, as with a pass key, so any server of this machine can
//! redeem a ticket another made - which matters because the window that made it may be serving on a
//! port other than the one the link names, lost to another moon started first.
//!
//! The two never stand in for each other. A pass key's MAC is over its sixteen-byte id alone
//! and a ticket's over thirty bytes that start with `ticket`, so no MAC is ever both; and the
//! formats differ besides - a ticket's `t` is no id, and a pass key has no `t.` to read as a
//! ticket.
//!
//! One login per ticket is the one thing the signature cannot say. The server that redeems a
//! ticket keeps its random bytes until it expires - see [`RedeemedTickets`] - and refuses them
//! a second time. That memory is the process's own: another moon server on the machine could
//! redeem the same ticket once more within its lifetime, which the short lifetimes bound.

use std::{collections::HashMap, sync::Mutex, time::Duration};

use super::{
    PassKeys,
    expiring::{Nonce, TokenKind, unix_seconds_now},
};

/// How long the ticket `Tools › Open in Web` opens a browser with is good for: the browser is
/// started there and then, so a minute is the time it takes to come up, with room to spare.
pub(crate) const OPEN_IN_WEB_TICKET_LIFETIME: Duration = Duration::from_secs(60);

/// How long the ticket `moon serve` prints is good for: a person has to find the line and get
/// it to a browser, maybe across an SSH session.
pub(crate) const SERVE_TICKET_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// The longest a ticket may be asked for with `POST /api/login-ticket`. A ticket is only for
/// the moment of logging in; anything meant to last is a pass key.
pub(crate) const LONGEST_TICKET_LIFETIME: Duration = SERVE_TICKET_LIFETIME;

const TICKET: TokenKind = TokenKind {
    prefix: "t.",
    mac_domain: b"ticket",
};

/// Why a ticket did not log a browser in. Each is said on the login page it lands back on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TicketRefusal {
    /// Not a ticket this machine's secret signed: garbled, altered, or another machine's.
    NotValid,
    Expired,
    UsedAlready,
}

impl TicketRefusal {
    /// What the login page says. An expired ticket and a used one read the same to the person
    /// holding the link - it no longer works, and they need a new one.
    pub(crate) fn explanation(&self) -> &'static str {
        match self {
            Self::NotValid => "that login link is not valid for this server",
            Self::Expired | Self::UsedAlready => "that login link has expired or was used already",
        }
    }
}

impl PassKeys {
    /// A new ticket, good for `lifetime` from now and for one login.
    pub(crate) fn login_ticket(&self, lifetime: Duration) -> String {
        self.ticket_expiring_at(unix_seconds_now() + lifetime.as_secs())
    }

    fn ticket_expiring_at(&self, expiry: u64) -> String {
        self.token_expiring_at(&TICKET, expiry)
    }

    fn signed_ticket(&self, ticket: &str) -> Option<(u64, Nonce)> {
        self.signed_token(&TICKET, ticket)
    }
}

/// The tickets a server has let a browser in with, each kept until it expires - after which
/// the signature refuses it anyway, and remembering it is no longer needed.
#[derive(Default)]
pub(crate) struct RedeemedTickets {
    /// Each redeemed ticket's nonce, and the second it expires at.
    expiries: Mutex<HashMap<Nonce, u64>>,
}

impl RedeemedTickets {
    /// Let a browser in with `ticket`, once: a signed, unexpired ticket not redeemed here
    /// before is remembered as redeemed from now on.
    pub(crate) fn redeem(&self, keys: &PassKeys, ticket: &str) -> Result<(), TicketRefusal> {
        self.redeem_at(keys, ticket, unix_seconds_now())
    }

    fn redeem_at(&self, keys: &PassKeys, ticket: &str, now: u64) -> Result<(), TicketRefusal> {
        let (expiry, nonce) = keys.signed_ticket(ticket).ok_or(TicketRefusal::NotValid)?;
        if now >= expiry {
            return Err(TicketRefusal::Expired);
        }
        let mut expiries = self
            .expiries
            .lock()
            .expect("the redeemed tickets lock is poisoned");
        expiries.retain(|_, kept_until| now < *kept_until);
        if expiries.insert(nonce, expiry).is_some() {
            return Err(TicketRefusal::UsedAlready);
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass_keys::SEPARATOR;

    fn keys(name: &str) -> PassKeys {
        let dir =
            std::env::temp_dir().join(format!("moonreview-tickets-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        PassKeys::kept_at(&dir.join("secret")).expect("expected a secret")
    }

    #[test]
    fn a_ticket_logs_in_once() {
        let keys = keys("once");
        let redeemed = RedeemedTickets::default();
        let ticket = keys.login_ticket(OPEN_IN_WEB_TICKET_LIFETIME);

        assert_eq!(redeemed.redeem(&keys, &ticket), Ok(()));
        assert_eq!(
            redeemed.redeem(&keys, &ticket),
            Err(TicketRefusal::UsedAlready)
        );
        assert_eq!(
            redeemed.redeem(&keys, &keys.login_ticket(OPEN_IN_WEB_TICKET_LIFETIME)),
            Ok(()),
            "another ticket is a login of its own"
        );
    }

    #[test]
    fn an_expired_ticket_is_refused() {
        let keys = keys("expired");
        let redeemed = RedeemedTickets::default();
        let ticket = keys.ticket_expiring_at(1_000);

        assert_eq!(
            redeemed.redeem_at(&keys, &ticket, 999),
            Ok(()),
            "a second before it expires, it is good"
        );
        let later = keys.ticket_expiring_at(1_000);
        assert_eq!(
            redeemed.redeem_at(&keys, &later, 1_000),
            Err(TicketRefusal::Expired)
        );
        assert_eq!(
            redeemed.redeem(&keys, &keys.ticket_expiring_at(unix_seconds_now() - 1)),
            Err(TicketRefusal::Expired)
        );
    }

    /// Once a ticket has expired its nonce is let go of, and the signature refuses it alone.
    #[test]
    fn a_redeemed_ticket_is_forgotten_once_it_has_expired() {
        let keys = keys("pruned");
        let redeemed = RedeemedTickets::default();
        redeemed
            .redeem_at(&keys, &keys.ticket_expiring_at(1_000), 500)
            .expect("expected the ticket to redeem");

        redeemed
            .redeem_at(&keys, &keys.ticket_expiring_at(3_000), 2_000)
            .expect("expected the ticket to redeem");

        assert_eq!(redeemed.expiries.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_ticket_with_its_payload_or_its_mac_changed_is_refused() {
        let keys = keys("tampered");
        let redeemed = RedeemedTickets::default();
        let ticket = keys.login_ticket(OPEN_IN_WEB_TICKET_LIFETIME);
        let (payload, mac) = ticket[TICKET.prefix.len()..]
            .split_once(SEPARATOR)
            .expect("expected a payload and a mac");
        let other = keys.login_ticket(SERVE_TICKET_LIFETIME);
        let (other_payload, other_mac) = other[TICKET.prefix.len()..]
            .split_once(SEPARATOR)
            .expect("expected a payload and a mac");

        for forged in [
            format!("{}{other_payload}{SEPARATOR}{mac}", TICKET.prefix),
            format!("{}{payload}{SEPARATOR}{other_mac}", TICKET.prefix),
        ] {
            assert_eq!(
                redeemed.redeem(&keys, &forged),
                Err(TicketRefusal::NotValid)
            );
        }
    }

    #[test]
    fn a_ticket_of_another_secret_is_refused() {
        let ours = keys("ours");
        let theirs = keys("theirs");

        assert_eq!(
            RedeemedTickets::default()
                .redeem(&ours, &theirs.login_ticket(OPEN_IN_WEB_TICKET_LIFETIME)),
            Err(TicketRefusal::NotValid)
        );
    }

    /// A ticket is for logging in once, a pass key for as long as the secret lasts: neither is
    /// let in where the other is asked for.
    #[test]
    fn a_ticket_is_no_pass_key_and_a_pass_key_no_ticket() {
        let keys = keys("apart");
        let ticket = keys.login_ticket(SERVE_TICKET_LIFETIME);
        let key = keys.generate();

        assert!(!keys.admits(&ticket), "{ticket} was let in as a pass key");
        assert_eq!(
            RedeemedTickets::default().redeem(&keys, &key),
            Err(TicketRefusal::NotValid)
        );
        assert_eq!(
            RedeemedTickets::default().redeem(&keys, &format!("{}{key}", TICKET.prefix)),
            Err(TicketRefusal::NotValid),
            "a pass key dressed as a ticket is still a pass key's MAC"
        );
    }

    #[test]
    fn what_is_no_ticket_at_all_is_refused() {
        let keys = keys("garbage");

        for garbage in ["", "t.", "t..", "t.AAAA.AAAA", "ticket", "not base64!.x"] {
            assert_eq!(
                RedeemedTickets::default().redeem(&keys, garbage),
                Err(TicketRefusal::NotValid),
                "{garbage:?}"
            );
        }
    }
}
