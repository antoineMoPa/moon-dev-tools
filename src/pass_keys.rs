//! Pass keys: what a request to the server shows to be let in.
//!
//! The server answers with the repo, writes files and runs shells, so being able to reach its
//! port must not be enough to use it: any process on the machine can, and so can any page the
//! browser has open, with a `fetch` or a `WebSocket` to localhost. A pass key is what tells a
//! request that was meant for it apart from one that was not.
//!
//! A key is `<id>.<mac>`: sixteen random bytes, and their HMAC-SHA256 under a secret this
//! machine keeps in `~/.moonreview/pass-key-secret`, both base64url without padding. Checking
//! one is recomputing the MAC, so the server remembers nothing about the keys it has handed out:
//! there can be any number, each window or browser holding its own. Taking every key back is
//! making a new secret - `Tools › Users` does, through `POST /api/users/kick-all`, and the
//! server that answers swaps to it as it goes - see `crate::server::users`. Any other server
//! running on the old secret keeps its loaded copy until it is restarted. Taking one key back
//! is that module's too: a server keeps the ids it has kicked, since the MAC alone would
//! still admit them. The secret is the machine's rather than a repo's because one server serves
//! every repo on the machine, and `.moontasks` may well be committed, which is no place for it.
//!
//! A pass key never travels in a URL or in a process's arguments: a browser is sent a login
//! ticket instead - see [`tickets`] - and a window started through `open` is handed its key in
//! a file - see [`handoff`].

mod browser_sessions;
mod expiring;
pub(crate) mod handoff;
mod tickets;

pub(crate) use browser_sessions::BROWSER_SESSION_LIFETIME;
pub(crate) use tickets::{
    LONGEST_TICKET_LIFETIME, OPEN_IN_WEB_TICKET_LIFETIME, RedeemedTickets, SERVE_TICKET_LIFETIME,
};

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The file under `~/.moonreview` the secret is kept in.
const SECRET_FILE_NAME: &str = "pass-key-secret";
const SECRET_LENGTH: usize = 32;
const ID_LENGTH: usize = 16;
/// Between the id and the MAC. Not a character base64url uses, so a key splits one way only.
const SEPARATOR: char = '.';

type KeyMac = Hmac<Sha256>;

/// The secret keys are made and checked with. Cheap to clone: it is the 32 bytes and nothing
/// else.
#[derive(Clone)]
pub(crate) struct PassKeys {
    secret: [u8; SECRET_LENGTH],
}

impl PassKeys {
    /// The keys of this machine: the secret in `~/.moonreview`, made on the first call that
    /// needs it. Under test it is a scratch file instead - see [`secret_path`].
    pub(crate) fn for_this_machine() -> Result<Self> {
        Self::kept_at(&secret_path()?)
    }

    /// The keys of the secret at `path`, which is written first when there is none yet.
    ///
    /// A file that is there but is not a secret this wrote - the wrong length, or readable by
    /// other users - is refused rather than replaced: replacement would make new servers
    /// disagree with running ones, and a secret others could read was never a secret.
    pub(crate) fn kept_at(path: &Path) -> Result<Self> {
        if !path.exists() {
            write_new_secret(path)?;
        }
        refuse_a_secret_others_can_read(path)?;
        let read = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        let secret: [u8; SECRET_LENGTH] = read.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!(
                "{} holds {} bytes where a pass-key secret is {SECRET_LENGTH}; delete it to make \
                 a new one, which takes back every pass key handed out",
                path.display(),
                read.len()
            )
        })?;
        Ok(Self { secret })
    }

    /// The keys of a secret made afresh at `path`, over whatever was there: every key, ticket
    /// and browser session of the old secret is refused from here on. The new secret is
    /// written whole to a file of its own and moved into place, so a reader never sees half of
    /// one.
    pub(crate) fn renewed_at(path: &Path) -> Result<Self> {
        let (draft, secret) = write_draft_secret(path)?;
        if let Err(error) = fs::rename(&draft, path) {
            let _ = fs::remove_file(&draft);
            return Err(error).with_context(|| format!("failed to replace {}", path.display()));
        }
        Ok(Self { secret })
    }

    /// A new key. Every call is a different one, and every one of them lets its holder in.
    pub(crate) fn generate(&self) -> String {
        let mut id = [0u8; ID_LENGTH];
        getrandom_03::fill(&mut id).expect("the OS has no randomness to make a pass key from");
        let mac = self.mac_of(&id).finalize().into_bytes();
        format!(
            "{}{SEPARATOR}{}",
            URL_SAFE_NO_PAD.encode(id),
            URL_SAFE_NO_PAD.encode(mac)
        )
    }

    /// Whether `key` is one of this secret's. Anything that does not read as a key at all is
    /// simply not one: what a request presents is the requester's to get wrong.
    ///
    /// The MAC is compared in constant time, so how long a refusal takes says nothing about
    /// how close the guess was.
    pub(crate) fn admits(&self, key: &str) -> bool {
        self.admitted_key_id(key).is_some()
    }

    /// The id of `key` - the half before the separator, which names the key without being
    /// it - when `key` is one of this secret's; nothing for any other string.
    pub(crate) fn admitted_key_id(&self, key: &str) -> Option<String> {
        let (id, mac) = key.split_once(SEPARATOR)?;
        let (Ok(id_bytes), Ok(mac)) = (URL_SAFE_NO_PAD.decode(id), URL_SAFE_NO_PAD.decode(mac))
        else {
            return None;
        };
        (id_bytes.len() == ID_LENGTH && self.mac_of(&id_bytes).verify_slice(&mac).is_ok())
            .then(|| id.to_string())
    }

    fn mac_of(&self, id: &[u8]) -> KeyMac {
        let mut mac = KeyMac::new_from_slice(&self.secret).expect("HMAC takes a key of any length");
        mac.update(id);
        mac
    }
}

/// The keys of this test process's scratch secret - see [`secret_path`] - for a test that
/// serves on a real socket and has to be let in like any other client.
#[cfg(test)]
pub(crate) fn of_this_test_run() -> PassKeys {
    PassKeys::for_this_machine().expect("expected a scratch pass-key secret")
}

/// Where this run's secret is kept.
///
/// Under test it is a scratch file, one per test process rather than per test: a test's
/// server answers on threads of its own, which have to find the same secret the test made its
/// key with. A test run must never make or read the real one.
pub(crate) fn secret_path() -> Result<PathBuf> {
    #[cfg(test)]
    {
        Ok(std::env::temp_dir().join(format!(
            "moonreview-test-{SECRET_FILE_NAME}-{}",
            std::process::id()
        )))
    }
    #[cfg(not(test))]
    {
        Ok(crate::settings::moonreview_dir()
            .context("no home directory to keep the pass-key secret in")?
            .join(SECRET_FILE_NAME))
    }
}

/// Write a fresh secret to `path`, readable by this user only.
///
/// Written in full to a file of its own and then linked into place, which fails when `path`
/// already exists: two processes starting at once - a window and `moon serve`, or two tests -
/// both find no secret, and the one that loses the race reads the winner's rather than
/// overwriting it and leaving the winner with keys nobody accepts.
fn write_new_secret(path: &Path) -> Result<()> {
    let (draft, _) = write_draft_secret(path)?;
    let linked = fs::hard_link(&draft, path);
    fs::remove_file(&draft).with_context(|| format!("failed to remove {}", draft.display()))?;
    match linked {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to write {}", path.display())),
    }
}

/// A fresh secret in a draft file beside `path`, readable by this user only, for the caller to
/// link or move into place: the draft's path, and the secret in it.
fn write_draft_secret(path: &Path) -> Result<(PathBuf, [u8; SECRET_LENGTH])> {
    let dir = path
        .parent()
        .with_context(|| format!("{} is in no folder", path.display()))?;
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;

    let mut secret = [0u8; SECRET_LENGTH];
    getrandom_03::fill(&mut secret)
        .map_err(|error| anyhow::anyhow!("the OS has no randomness for a secret: {error}"))?;
    let mut nonce = [0u8; 8];
    getrandom_03::fill(&mut nonce)
        .map_err(|error| anyhow::anyhow!("the OS has no randomness for a secret: {error}"))?;
    let draft = dir.join(format!(
        ".{SECRET_FILE_NAME}-{}",
        URL_SAFE_NO_PAD.encode(nonce)
    ));

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&draft)
        .and_then(|mut file| file.write_all(&secret).and_then(|()| file.sync_all()));
    if let Err(error) = written {
        let _ = fs::remove_file(&draft);
        return Err(error).with_context(|| format!("failed to write {}", draft.display()));
    }
    Ok((draft, secret))
}

#[cfg(unix)]
fn refuse_a_secret_others_can_read(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        bail!(
            "{} can be read by other users (mode {:o}); `chmod 600` it, or delete it to make a \
             new one, which takes back every pass key handed out",
            path.display(),
            mode & 0o777
        );
    }
    Ok(())
}

/// Windows keeps a file in the home folder to its user already.
#[cfg(not(unix))]
fn refuse_a_secret_others_can_read(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "moonreview-pass-keys-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir.join(SECRET_FILE_NAME)
    }

    #[test]
    fn a_generated_key_is_admitted() {
        let keys = PassKeys::kept_at(&scratch("admitted")).expect("expected a secret");

        let key = keys.generate();

        assert!(keys.admits(&key), "{key} should be admitted");
        assert_ne!(key, keys.generate(), "every key should be a new one");
    }

    #[test]
    fn a_key_with_its_id_or_its_mac_changed_is_refused() {
        let keys = PassKeys::kept_at(&scratch("tampered")).expect("expected a secret");
        let key = keys.generate();
        let (id, mac) = key.split_once(SEPARATOR).expect("expected an id and a mac");
        let other = keys.generate();
        let (other_id, other_mac) = other.split_once(SEPARATOR).expect("expected a key");

        assert!(!keys.admits(&format!("{other_id}{SEPARATOR}{mac}")));
        assert!(!keys.admits(&format!("{id}{SEPARATOR}{other_mac}")));
    }

    #[test]
    fn a_key_of_another_secret_is_refused() {
        let ours = PassKeys::kept_at(&scratch("ours")).expect("expected a secret");
        let theirs = PassKeys::kept_at(&scratch("theirs")).expect("expected a secret");

        assert!(!ours.admits(&theirs.generate()));
    }

    #[test]
    fn what_is_no_key_at_all_is_refused() {
        let keys = PassKeys::kept_at(&scratch("garbage")).expect("expected a secret");

        for garbage in ["", ".", "no-separator", "not base64!.also not", "AAAA.AAAA"] {
            assert!(!keys.admits(garbage), "{garbage:?} should be refused");
        }
    }

    /// The secret is read back, not made again, so a key outlives the process that made it.
    #[test]
    fn the_secret_is_kept_between_runs_and_only_its_owner_can_read_it() {
        let path = scratch("kept");
        let key = PassKeys::kept_at(&path)
            .expect("expected a secret")
            .generate();

        let again = PassKeys::kept_at(&path).expect("expected the same secret");

        assert!(again.admits(&key));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)
                .expect("expected the file")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    /// A renewed secret is another secret: the old keys are refused, the new ones admitted,
    /// and what is on disk from then on is the new one.
    #[test]
    fn a_renewed_secret_refuses_the_old_keys_and_is_what_is_read_back() {
        let path = scratch("renewed");
        let old = PassKeys::kept_at(&path).expect("expected a secret");
        let old_key = old.generate();

        let new = PassKeys::renewed_at(&path).expect("expected a renewed secret");
        let new_key = new.generate();

        assert!(!new.admits(&old_key), "the old key should be refused");
        assert!(new.admits(&new_key));
        let read_back = PassKeys::kept_at(&path).expect("expected the renewed secret");
        assert!(read_back.admits(&new_key));
        assert!(!read_back.admits(&old_key));
        assert_eq!(
            fs::read_dir(path.parent().expect("a file is in a folder"))
                .expect("expected the folder")
                .count(),
            1,
            "no draft is left beside the secret"
        );
    }

    #[test]
    fn a_secret_of_the_wrong_length_is_refused_rather_than_replaced() {
        let path = scratch("wrong-length");
        PassKeys::kept_at(&path).expect("expected a secret");
        fs::write(&path, b"short").expect("expected to overwrite the secret");

        let error = PassKeys::kept_at(&path)
            .err()
            .expect("a short secret should be refused");

        assert!(error.to_string().contains("5 bytes"), "got {error}");
        assert_eq!(fs::read(&path).expect("expected the file"), b"short");
    }
}
