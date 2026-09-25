//! Browser sessions: what a logged-in browser's cookie holds, in place of a pass key.
//!
//! Logging a browser in - with a pasted pass key or a login link's ticket, see
//! [`crate::server::web_page`] - gives it a session: an expiring token of its own kind,
//! `s.<payload>.<mac>` - see [`super::expiring`] - good for [`BROWSER_SESSION_LIFETIME`]. A
//! pass key never goes into the cookie, so a browser's login runs out on its own, however long
//! the key it was made from lasts, and a cookie copied off a machine is worth a week at most.
//!
//! A session is only ever a cookie. It is no pass key - `Authorization` takes pass keys alone
//! - and no ticket, so it cannot be spent to log another browser in.

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

use super::{
    PassKeys,
    expiring::{TokenKind, unix_seconds_now},
};

/// How long a browser stays logged in: a week, after which it logs in again - with a pass
/// key, or through `Tools › Open in Web`.
pub(crate) const BROWSER_SESSION_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const BROWSER_SESSION: TokenKind = TokenKind {
    prefix: "s.",
    mac_domain: b"browser-session",
};

impl PassKeys {
    /// A new browser session, good for [`BROWSER_SESSION_LIFETIME`] from now.
    pub(crate) fn browser_session(&self) -> String {
        self.token_expiring_at(
            &BROWSER_SESSION,
            unix_seconds_now() + BROWSER_SESSION_LIFETIME.as_secs(),
        )
    }

    /// Whether `session` is one this machine's secret made and that has not run out. The
    /// server asks for the id instead - see [`Self::admitted_browser_session_id`] - since who
    /// the session is decides whether it is let in; this is the tests' short way of asking.
    #[cfg(test)]
    pub(crate) fn admits_browser_session(&self, session: &str) -> bool {
        self.admits_browser_session_at(session, unix_seconds_now())
    }

    /// The id of `session` - its random half, base64url, which names the login without being
    /// it - when it is one this machine's secret made and that has not run out.
    pub(crate) fn admitted_browser_session_id(&self, session: &str) -> Option<String> {
        self.admitted_browser_session_id_at(session, unix_seconds_now())
    }

    #[cfg(test)]
    fn admits_browser_session_at(&self, session: &str, now: u64) -> bool {
        self.admitted_browser_session_id_at(session, now).is_some()
    }

    fn admitted_browser_session_id_at(&self, session: &str, now: u64) -> Option<String> {
        self.signed_token(&BROWSER_SESSION, session)
            .filter(|(expiry, _)| now < *expiry)
            .map(|(_, nonce)| URL_SAFE_NO_PAD.encode(nonce))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass_keys::{OPEN_IN_WEB_TICKET_LIFETIME, RedeemedTickets};

    fn keys(name: &str) -> PassKeys {
        let path = std::env::temp_dir().join(format!(
            "moon-browser-sessions-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        PassKeys::kept_at(&path).expect("expected a scratch secret")
    }

    #[test]
    fn a_session_lets_a_browser_in_for_a_week_and_not_after() {
        let keys = keys("week");
        let session = keys.browser_session();
        let now = unix_seconds_now();

        assert!(keys.admits_browser_session_at(&session, now));
        assert!(keys.admits_browser_session_at(
            &session,
            now + BROWSER_SESSION_LIFETIME.as_secs() - 60
        ));
        assert!(!keys.admits_browser_session_at(
            &session,
            now + BROWSER_SESSION_LIFETIME.as_secs() + 1
        ));
    }

    #[test]
    fn a_session_is_no_pass_key_and_no_ticket_and_they_are_no_session() {
        let keys = keys("apart");
        let session = keys.browser_session();
        let key = keys.generate();
        let ticket = keys.login_ticket(OPEN_IN_WEB_TICKET_LIFETIME);

        assert!(!keys.admits(&session), "a session was let in as a pass key");
        assert!(
            RedeemedTickets::default().redeem(&keys, &session).is_err(),
            "a session was redeemed as a ticket"
        );
        assert!(!keys.admits_browser_session(&key), "a pass key was let in as a session");
        assert!(!keys.admits_browser_session(&ticket), "a ticket was let in as a session");
    }

    #[test]
    fn a_session_of_another_secret_or_altered_is_refused() {
        let ours = keys("ours");
        let theirs = keys("theirs");
        assert!(!ours.admits_browser_session(&theirs.browser_session()));

        let session = ours.browser_session();
        let (head, mac) = session.rsplit_once('.').expect("expected a mac");
        let altered = format!("{head}.{}", mac.chars().rev().collect::<String>());
        assert!(!ours.admits_browser_session(&altered));
        assert!(!ours.admits_browser_session("s.garbage.more"));
    }
}
