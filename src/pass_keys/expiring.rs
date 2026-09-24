//! Tokens signed with an expiry in them: login tickets and browser sessions - see
//! [`super::tickets`] and [`super::browser_sessions`]. Both are `<prefix><payload>.<mac>`: the
//! payload is the second the token expires at, a big-endian `u64` of Unix time, then sixteen
//! random bytes; the MAC is HMAC-SHA256 under the machine's pass-key secret of the kind's
//! domain and the payload. Both halves are base64url without padding.
//!
//! The domain is what keeps the kinds apart: a MAC over `ticket` and a payload is never one
//! over `browser-session` and the same payload, and neither is ever a pass key's, whose MAC is
//! over a bare sixteen-byte id. The prefixes differ as well, so a token of one kind does not
//! even read as another.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::Mac;

use super::{KeyMac, PassKeys, SEPARATOR};

/// One kind of expiring token: what it starts with, and what its MAC is taken over first.
pub(super) struct TokenKind {
    pub(super) prefix: &'static str,
    pub(super) mac_domain: &'static [u8],
}

const EXPIRY_LENGTH: usize = 8;
const NONCE_LENGTH: usize = 16;
const PAYLOAD_LENGTH: usize = EXPIRY_LENGTH + NONCE_LENGTH;

/// The random part of a token, which tells two tokens of the same second apart.
pub(super) type Nonce = [u8; NONCE_LENGTH];

impl PassKeys {
    /// A new token of `kind`, good until the Unix second `expiry`.
    pub(super) fn token_expiring_at(&self, kind: &TokenKind, expiry: u64) -> String {
        let mut nonce: Nonce = [0u8; NONCE_LENGTH];
        getrandom_03::fill(&mut nonce).expect("the OS has no randomness to make a token from");
        let mut payload = [0u8; PAYLOAD_LENGTH];
        payload[..EXPIRY_LENGTH].copy_from_slice(&expiry.to_be_bytes());
        payload[EXPIRY_LENGTH..].copy_from_slice(&nonce);
        let mac = self.token_mac_of(kind, &payload).finalize().into_bytes();
        format!(
            "{}{}{SEPARATOR}{}",
            kind.prefix,
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(mac)
        )
    }

    /// The expiry and nonce of `token`, when this secret signed it as a token of `kind` -
    /// whether or not it has expired since. Anything that does not read as one is simply not
    /// one, as with [`PassKeys::admits`], and its MAC is compared in constant time for the same
    /// reason.
    pub(super) fn signed_token(&self, kind: &TokenKind, token: &str) -> Option<(u64, Nonce)> {
        let (payload, mac) = token.strip_prefix(kind.prefix)?.split_once(SEPARATOR)?;
        let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
        let mac = URL_SAFE_NO_PAD.decode(mac).ok()?;
        let payload: [u8; PAYLOAD_LENGTH] = payload.try_into().ok()?;
        self.token_mac_of(kind, &payload).verify_slice(&mac).ok()?;
        let expiry = u64::from_be_bytes(
            payload[..EXPIRY_LENGTH]
                .try_into()
                .expect("the expiry is the payload's first eight bytes"),
        );
        let nonce: Nonce = payload[EXPIRY_LENGTH..]
            .try_into()
            .expect("the nonce is the rest of the payload");
        Some((expiry, nonce))
    }

    fn token_mac_of(&self, kind: &TokenKind, payload: &[u8; PAYLOAD_LENGTH]) -> KeyMac {
        let mut mac = KeyMac::new_from_slice(&self.secret).expect("HMAC takes a key of any length");
        mac.update(kind.mac_domain);
        mac.update(payload);
        mac
    }
}

pub(super) fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is set before 1970")
        .as_secs()
}
