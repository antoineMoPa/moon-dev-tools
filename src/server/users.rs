//! Who the server has let in, and who it no longer does - what `Tools › Users` shows and acts on.
//!
//! A pass key is checked by its MAC alone - see [`crate::pass_keys`] - so on its own the server
//! knows nothing of who holds one. This is what it comes to know: every key and browser session
//! a request has shown since the server started, by id, with the address the last one came
//! from and when. And what it is told: the ids kicked out, which the MAC would still admit and
//! so are refused here; and kicking everyone out, which is a new secret - see
//! [`PassKeys::renewed_at`] - after which nothing made under the old one reads as a key,
//! ticket or session.
//!
//! Kicked ids are kept in a file beside the secret, so a kick outlives the server. A new secret
//! empties the file: nothing made under the old one is admitted anyway.
//!
//! A shell's websocket is admitted once, when it opens, so a kick alone would leave it open.
//! Every open socket listens for kicks - see [`Users::kicks`] - and hangs up when its user is
//! among the kicked.

use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    net::IpAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use axum::{
    Extension, Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::watch;

use crate::pass_keys::PassKeys;

/// What the file of kicked ids is called, after the secret's name: `pass-key-secret.kicked`.
const KICKED_SUFFIX: &str = "kicked";

const KEY_PREFIX: &str = "key:";
const BROWSER_PREFIX: &str = "browser:";

/// A user of the server, by what it shows: the id of its pass key, or of its browser session -
/// see [`PassKeys::admitted_key_id`] and [`PassKeys::admitted_browser_session_id`]. Never the
/// key or the session itself: an id names one and cannot be used in its place, so it can be
/// listed, kept in a file, and named in a route.
///
/// Written `key:<id>` or `browser:<id>`, which is how it travels and is kept.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum UserId {
    PassKey(String),
    BrowserSession(String),
}

impl UserId {
    fn kind(&self) -> UserKind {
        match self {
            Self::PassKey(_) => UserKind::PassKey,
            Self::BrowserSession(_) => UserKind::Browser,
        }
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PassKey(id) => write!(f, "{KEY_PREFIX}{id}"),
            Self::BrowserSession(id) => write!(f, "{BROWSER_PREFIX}{id}"),
        }
    }
}

impl FromStr for UserId {
    type Err = anyhow::Error;

    fn from_str(written: &str) -> Result<Self> {
        if let Some(id) = written.strip_prefix(KEY_PREFIX) {
            return Ok(Self::PassKey(id.to_string()));
        }
        if let Some(id) = written.strip_prefix(BROWSER_PREFIX) {
            return Ok(Self::BrowserSession(id.to_string()));
        }
        bail!("{written:?} is no user id: one is `key:<id>` or `browser:<id>`")
    }
}

impl Serialize for UserId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for UserId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UserKind {
    /// A native window, `curl`, or an extension: `Authorization: Bearer <pass key>`.
    PassKey,
    /// A logged-in browser: the session in its cookie.
    Browser,
}

/// What `GET /api/users` answers with, last seen first.
#[derive(Serialize, Deserialize)]
pub(crate) struct UserList {
    pub(crate) users: Vec<User>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct User {
    pub(crate) id: UserId,
    pub(crate) kind: UserKind,
    /// Where the last request came from.
    pub(crate) ip: IpAddr,
    /// Unix seconds.
    pub(crate) first_seen: u64,
    pub(crate) last_seen: u64,
    pub(crate) requests: u64,
    pub(crate) kicked: bool,
    /// Whether this is the user asking for the list.
    pub(crate) you: bool,
}

/// What the server has seen of one user.
#[derive(Clone, Copy)]
struct Seen {
    ip: IpAddr,
    first_seen: u64,
    last_seen: u64,
    requests: u64,
}

/// Whether a request whose key or session the secret admits goes on to its route.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    In,
    Kicked,
}

/// The server's users, shared by every request: cheap to clone, one state behind them all.
#[derive(Clone)]
pub(crate) struct Users {
    shared: Arc<Mutex<Shared>>,
    /// Counts the kicks, for whoever holds a connection open to hear of one - see
    /// [`Users::kicks`].
    kicks: watch::Sender<u64>,
}

struct Shared {
    keys: PassKeys,
    secret_path: PathBuf,
    kicked: HashSet<UserId>,
    seen: HashMap<UserId, Seen>,
}

impl Users {
    /// The users of this machine's server: the secret in `~/.moonreview` and the kicks kept
    /// beside it.
    pub(crate) fn for_this_machine() -> Result<Self> {
        Self::kept_at(&crate::pass_keys::secret_path()?)
    }

    /// The users of the secret at `secret_path` - made there first when there is none, as
    /// [`PassKeys::kept_at`] does - and the kicks in the file beside it.
    pub(crate) fn kept_at(secret_path: &Path) -> Result<Self> {
        let keys = PassKeys::kept_at(secret_path)?;
        let kicked = read_kicked(&kicked_path(secret_path))?;
        Ok(Self {
            shared: Arc::new(Mutex::new(Shared {
                keys,
                secret_path: secret_path.to_path_buf(),
                kicked,
                seen: HashMap::new(),
            })),
            kicks: watch::Sender::new(0),
        })
    }

    /// The keys requests are checked against right now: the secret in force, which
    /// [`Users::kick_everyone`] replaces.
    pub(crate) fn keys(&self) -> PassKeys {
        self.lock().keys.clone()
    }

    /// A request from `user` at `ip` came in with a key or session the secret admits: whether
    /// it goes on. Seen either way, so a kicked key still knocking shows as such.
    pub(crate) fn let_in(&self, user: &UserId, ip: IpAddr) -> Admission {
        let now = unix_seconds_now();
        let mut shared = self.lock();
        shared
            .seen
            .entry(user.clone())
            .and_modify(|seen| {
                seen.ip = ip;
                seen.last_seen = now;
                seen.requests += 1;
            })
            .or_insert(Seen {
                ip,
                first_seen: now,
                last_seen: now,
                requests: 1,
            });
        match shared.kicked.contains(user) {
            true => Admission::Kicked,
            false => Admission::In,
        }
    }

    pub(crate) fn is_kicked(&self, user: &UserId) -> bool {
        self.lock().kicked.contains(user)
    }

    /// Everyone seen since the server started, last seen first. `asker` is marked as `you`.
    pub(crate) fn list(&self, asker: &UserId) -> Vec<User> {
        let shared = self.lock();
        let mut users: Vec<User> = shared
            .seen
            .iter()
            .map(|(id, seen)| User {
                id: id.clone(),
                kind: id.kind(),
                ip: seen.ip,
                first_seen: seen.first_seen,
                last_seen: seen.last_seen,
                requests: seen.requests,
                kicked: shared.kicked.contains(id),
                you: id == asker,
            })
            .collect();
        users.sort_by(|one, other| {
            other
                .last_seen
                .cmp(&one.last_seen)
                .then_with(|| one.id.to_string().cmp(&other.id.to_string()))
        });
        users
    }

    /// Refuse `user` from here on, and remember it past this server's life. Only someone seen
    /// can be kicked: a kick names a row of the list.
    pub(crate) fn kick(&self, user: &UserId) -> Result<()> {
        let mut shared = self.lock();
        if !shared.seen.contains_key(user) {
            bail!("no user {user} has been seen by this server");
        }
        shared.kicked.insert(user.clone());
        write_kicked(&kicked_path(&shared.secret_path), &shared.kicked)?;
        drop(shared);
        self.kicks.send_modify(|kicks| *kicks += 1);
        Ok(())
    }

    /// A new secret in place of the old: every key, ticket and browser session there was stops
    /// working, this request's included. Everyone seen so far is marked kicked, since that is
    /// what they are; the file of kicks is emptied, since the ids it held cannot come back.
    pub(crate) fn kick_everyone(&self) -> Result<()> {
        let mut shared = self.lock();
        shared.keys = PassKeys::renewed_at(&shared.secret_path)?;
        let everyone: Vec<UserId> = shared.seen.keys().cloned().collect();
        shared.kicked = everyone.into_iter().collect();
        let kicked_file = kicked_path(&shared.secret_path);
        if kicked_file.exists() {
            fs::remove_file(&kicked_file)
                .with_context(|| format!("failed to remove {}", kicked_file.display()))?;
        }
        drop(shared);
        self.kicks.send_modify(|kicks| *kicks += 1);
        Ok(())
    }

    /// Told of every kick - see [`watch::Receiver::changed`] - for a connection that was
    /// admitted when it opened to ask [`Users::is_kicked`] again.
    pub(crate) fn kicks(&self) -> watch::Receiver<u64> {
        self.kicks.subscribe()
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .expect("no request panics while holding the users")
    }
}

/// The users of this test process's scratch secret - see [`crate::pass_keys::secret_path`].
#[cfg(test)]
pub(crate) fn of_this_test_run() -> Users {
    Users::for_this_machine().expect("expected a scratch pass-key secret")
}

fn kicked_path(secret_path: &Path) -> PathBuf {
    let name = secret_path
        .file_name()
        .expect("a secret is a file")
        .to_string_lossy();
    secret_path.with_file_name(format!("{name}.{KICKED_SUFFIX}"))
}

/// The ids in the kicked file, one per line; none when there is no file yet.
fn read_kicked(path: &Path) -> Result<HashSet<UserId>> {
    let written = match fs::read_to_string(path) {
        Ok(written) => written,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    written
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.parse()
                .with_context(|| format!("{} holds a line that is no user id", path.display()))
        })
        .collect()
}

fn write_kicked(path: &Path, kicked: &HashSet<UserId>) -> Result<()> {
    let mut lines: Vec<String> = kicked.iter().map(UserId::to_string).collect();
    lines.sort();
    let mut written = lines.join("\n");
    written.push('\n');
    fs::write(path, written).with_context(|| format!("failed to write {}", path.display()))
}

fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is set after 1970")
        .as_secs()
}

/// `GET /api/users`: everyone the server has seen, the asker marked.
pub(super) async fn list(
    State(users): State<Users>,
    Extension(asker): Extension<UserId>,
) -> Json<UserList> {
    Json(UserList {
        users: users.list(&asker),
    })
}

/// `POST /api/users/{id}/kick`: refuse that user from here on.
pub(super) async fn kick(
    State(users): State<Users>,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let user: UserId = id
        .parse()
        .map_err(|error| (StatusCode::BAD_REQUEST, format!("{error}\n")))?;
    users
        .kick(&user)
        .map_err(|error| (StatusCode::NOT_FOUND, format!("{error:#}\n")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/users/kick-all`: a new secret, so no key or login there was works any more -
/// the asker's included, which is the point: the asker makes itself a new key on the machine.
pub(super) async fn kick_everyone(
    State(users): State<Users>,
) -> Result<StatusCode, (StatusCode, String)> {
    users
        .kick_everyone()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}\n")))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("moonreview-users-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("pass-key-secret")
    }

    const LOCALHOST: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

    fn seen_key(users: &Users, id: &str) -> UserId {
        let user = UserId::PassKey(id.to_string());
        assert_eq!(users.let_in(&user, LOCALHOST), Admission::In);
        user
    }

    #[test]
    fn a_user_id_is_written_and_read_back_as_its_kind_and_id() {
        for (id, written) in [
            (UserId::PassKey("abc".to_string()), "key:abc"),
            (UserId::BrowserSession("xyz".to_string()), "browser:xyz"),
        ] {
            assert_eq!(id.to_string(), written);
            assert_eq!(written.parse::<UserId>().expect("expected an id"), id);
        }
        assert!("abc".parse::<UserId>().is_err());
    }

    #[test]
    fn a_kick_is_refused_from_then_on_and_outlives_the_server() {
        let secret = scratch("kick");
        let users = Users::kept_at(&secret).expect("expected users");
        let kicked = seen_key(&users, "one");
        let kept = seen_key(&users, "two");

        users.kick(&kicked).expect("expected the kick");

        assert_eq!(users.let_in(&kicked, LOCALHOST), Admission::Kicked);
        assert_eq!(users.let_in(&kept, LOCALHOST), Admission::In);
        let restarted = Users::kept_at(&secret).expect("expected users again");
        assert_eq!(restarted.let_in(&kicked, LOCALHOST), Admission::Kicked);
        assert_eq!(restarted.let_in(&kept, LOCALHOST), Admission::In);
    }

    #[test]
    fn only_someone_seen_can_be_kicked() {
        let users = Users::kept_at(&scratch("unseen")).expect("expected users");

        let refused = users
            .kick(&UserId::PassKey("nobody".to_string()))
            .expect_err("an unseen user cannot be kicked");

        assert!(refused.to_string().contains("key:nobody"), "got {refused}");
    }

    #[test]
    fn kicking_everyone_is_a_new_secret_and_an_empty_file_of_kicks() {
        let secret = scratch("everyone");
        let users = Users::kept_at(&secret).expect("expected users");
        let old_key = users.keys().generate();
        let one = seen_key(&users, "one");
        users.kick(&one).expect("expected the kick");
        assert!(kicked_path(&secret).exists());
        let kicks = users.kicks();

        users.kick_everyone().expect("expected a new secret");

        assert!(
            !users.keys().admits(&old_key),
            "the old key should be refused"
        );
        assert!(users.is_kicked(&one));
        assert!(
            !kicked_path(&secret).exists(),
            "the kicks of the old secret are gone"
        );
        assert!(kicks.has_changed().expect("the users are still there"));
        let restarted = Users::kept_at(&secret).expect("expected users again");
        assert!(!restarted.keys().admits(&old_key));
        assert!(restarted.keys().admits(&users.keys().generate()));
    }

    #[test]
    fn the_list_is_last_seen_first_with_the_asker_marked() {
        let users = Users::kept_at(&scratch("list")).expect("expected users");
        let first = seen_key(&users, "first");
        let asker = seen_key(&users, "asker");

        let listed = users.list(&asker);

        let ids: Vec<&UserId> = listed.iter().map(|user| &user.id).collect();
        assert!(
            ids.contains(&&first) && ids.contains(&&asker),
            "got {ids:?}"
        );
        let you: Vec<bool> = listed.iter().map(|user| user.you).collect();
        assert_eq!(you.iter().filter(|you| **you).count(), 1);
        assert!(
            listed
                .windows(2)
                .all(|pair| pair[0].last_seen >= pair[1].last_seen),
            "last seen first"
        );
        assert!(
            listed
                .iter()
                .all(|user| user.ip == LOCALHOST && user.requests == 1)
        );
    }
}
