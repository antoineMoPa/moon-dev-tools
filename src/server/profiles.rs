//! Persistent people, identified by GitHub, over the server's short-lived admissions.
//! Projects and tasks remain shared; profiles own credentials and the automatically saved browser layout.
mod github;
mod routes;

/// Whether people sign in to this server with GitHub: `MOON_GITHUB_CLIENT_ID` is set.
pub(crate) fn people_sign_in_with_github() -> bool {
    github::client_id().is_some()
}

#[cfg(test)]
mod tests;

use super::users::UserId;
use anyhow::{Context, Result, bail};
pub(super) use routes::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(crate) struct Profiles {
    path: PathBuf,
    inner: Arc<Mutex<Stored>>,
    pending: Arc<Mutex<HashMap<String, github::Pending>>>,
    generations: Arc<Mutex<HashMap<String, u64>>>,
    /// Held while a Unix user is made for a sign-in - see [`Profiles::with_its_unix_user`] -
    /// so two sign-ins of one new account make one user. Apart from the lock on the profiles
    /// themselves, which every request reads.
    making_unix_users: Arc<Mutex<()>>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Stored {
    accounts: HashMap<String, Account>,
    admissions: HashMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Account {
    id: String,
    login: String,
    name: String,
    email: String,
    token: String,
    /// The Unix user everything started for this person runs as, on a server that gives each
    /// person one - see [`crate::unix_users`]. Kept against the account's id, so a login
    /// renamed on GitHub keeps its user and its home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unix_user: Option<String>,
    #[serde(default)]
    layout: Option<Layout>,
    #[serde(default, skip_serializing)]
    windows: HashMap<String, Layout>,
}

pub(super) use crate::api::profiles::{Layout, ProfileView};

impl Profiles {
    pub(crate) fn kept_at(secret: &Path) -> Result<Self> {
        let path = secret.with_extension("profiles.json");
        let mut inner: Stored = match fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context("could not read remote profiles")?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(e) => return Err(e.into()),
        };
        for account in inner.accounts.values_mut() {
            if account.layout.is_none() {
                let mut old: Vec<_> = account.windows.drain().collect();
                old.sort_by(|(a, x), (b, y)| {
                    let rank = |id: &str, layout: &Layout| {
                        (
                            layout.layout.as_ref().is_none_or(|s| s.trim().is_empty()),
                            !matches!(id, "default" | "current"),
                        )
                    };
                    rank(a, x).cmp(&rank(b, y)).then(a.cmp(b))
                });
                account.layout = old.into_iter().next().map(|(_, layout)| layout);
            }
            account.windows.clear();
        }
        #[cfg(unix)]
        if path.exists() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .context("could not protect remote profile credentials")?;
        }
        Ok(Self {
            path,
            inner: Arc::new(Mutex::new(inner)),
            pending: Arc::default(),
            generations: Arc::default(),
            making_unix_users: Arc::default(),
        })
    }

    fn change<T>(&self, change: impl FnOnce(&mut Stored) -> Result<T>) -> Result<T> {
        let mut guard = self.inner.lock().unwrap();
        let mut next = guard.clone();
        let result = change(&mut next)?;
        let draft = self
            .path
            .with_extension(format!("{}.tmp", crate::moontasks::store::new_uuid()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let saved = (|| -> Result<()> {
            let mut file = options.open(&draft)?;
            file.write_all(&serde_json::to_vec(&next)?)?;
            file.sync_all()?;
            fs::rename(&draft, &self.path)?;
            Ok(())
        })();
        if saved.is_err() {
            let _ = fs::remove_file(&draft);
        }
        saved.context("could not save remote profile")?;
        *guard = next;
        Ok(result)
    }

    pub(crate) fn forget_admission(&self, user: &UserId) -> Result<()> {
        self.begin_auth(user);
        self.change(|inner| {
            inner.admissions.remove(&user.to_string());
            Ok(())
        })
    }

    pub(crate) fn forget_all_admissions(&self) -> Result<()> {
        let mut generations = self.generations.lock().unwrap();
        for generation in generations.values_mut() {
            *generation += 1;
        }
        self.pending.lock().unwrap().clear();
        self.change(|inner| {
            inner.admissions.clear();
            Ok(())
        })
    }

    fn account(&self, user: &UserId) -> Option<Account> {
        let inner = self.inner.lock().unwrap();
        inner
            .admissions
            .get(&user.to_string())
            .and_then(|id| inner.accounts.get(id))
            .cloned()
    }

    pub(crate) fn namespace(&self, user: &UserId) -> String {
        self.inner
            .lock()
            .unwrap()
            .admissions
            .get(&user.to_string())
            .cloned()
            .unwrap_or_else(|| user.to_string())
    }

    fn view(&self, user: &UserId) -> ProfileView {
        let account = self.account(user);
        ProfileView {
            profile_namespace: account
                .as_ref()
                .map_or_else(|| user.to_string(), |a| a.id.clone()),
            id: account.as_ref().map(|a| a.id.clone()),
            // Signed in is what the window goes by to offer more than the sign-in pane.
            login: account
                .as_ref()
                .filter(|_| self.has_signed_in(user))
                .map(|a| a.login.clone()),
            unix_user: account.as_ref().and_then(|a| a.unix_user.clone()),
            sign_in_required: crate::unix_users::each_person_has_one(),
            layout: account.and_then(|a| a.layout),
            device_flow: github::client_id().is_some(),
        }
    }

    /// The person whose Unix user has this uid: who a process on this machine belongs to,
    /// when it asks the server for something over a socket - see `crate::instances`.
    pub(crate) fn person_with_uid(&self, uid: u32) -> Result<Option<crate::unix_users::Person>> {
        let mut people = self.people()?;
        people.retain(|person| person.unix_user.uid == uid);
        Ok(people.pop())
    }

    /// Who this admission signed in as, for the users list: the GitHub login, and the Unix
    /// user on a server that gives each person one.
    pub(crate) fn signed_in_as(&self, user: &UserId) -> Option<(String, Option<String>)> {
        self.account(user)
            .map(|account| (account.login, account.unix_user))
    }

    /// Every person this server has made a Unix user for, signed in right now or not.
    pub(crate) fn people(&self) -> Result<Vec<crate::unix_users::Person>> {
        let accounts: Vec<Account> =
            self.inner.lock().unwrap().accounts.values().cloned().collect();
        let mut people = Vec::new();
        for account in accounts {
            let Some(name) = &account.unix_user else {
                continue;
            };
            if let Some(unix_user) = crate::unix_users::UnixUser::named(name)? {
                people.push(crate::unix_users::Person {
                    github_login: account.login,
                    unix_user,
                });
            }
        }
        Ok(people)
    }

    /// Whether this admission has signed in with GitHub, which a server that gives each
    /// person a Unix user asks for before anything else - see `super::auth`.
    ///
    /// On such a server an account with no Unix user has not: one that signed in while the
    /// server ran as another user, and signs in again to be given its own.
    pub(crate) fn has_signed_in(&self, user: &UserId) -> bool {
        self.account(user).is_some_and(|account| {
            account.unix_user.is_some() || !crate::unix_users::each_person_has_one()
        })
    }

    /// The person this admission signed in as, on a server that gives each a Unix user.
    /// `None` for an admission that has not signed in, and for one whose Unix user the
    /// machine no longer has.
    pub(crate) fn person(&self, user: &UserId) -> Result<Option<crate::unix_users::Person>> {
        let Some(account) = self.account(user) else {
            return Ok(None);
        };
        let Some(name) = account.unix_user else {
            return Ok(None);
        };
        Ok(
            crate::unix_users::UnixUser::named(&name)?.map(|unix_user| crate::unix_users::Person {
                github_login: account.login,
                unix_user,
            }),
        )
    }

    /// Whose a session opened by this admission is: its profile, and its person on a server
    /// that gives each person a Unix user. Such a server only gets here signed in, and every
    /// account that signed in to it has a user: an account without one is refused rather than
    /// run as the server.
    pub(crate) fn session_owner(&self, user: &UserId) -> Result<crate::api::SessionOwner> {
        let namespace = self.namespace(user);
        if !crate::unix_users::each_person_has_one() {
            return Ok(crate::api::SessionOwner {
                namespace,
                person: None,
            });
        }
        let person = self.person(user)?.context(
            "Sign in with GitHub from Account first: this server has no Unix user for you yet",
        )?;
        Ok(crate::api::SessionOwner {
            namespace,
            person: Some(person),
        })
    }

    fn begin_auth(&self, user: &UserId) -> u64 {
        let mut generations = self.generations.lock().unwrap();
        let generation = generations.entry(user.to_string()).or_default();
        *generation += 1;
        self.pending
            .lock()
            .unwrap()
            .retain(|_, p| p.user != user.to_string());
        *generation
    }

    fn connect_if_current(&self, user: &UserId, account: Account, generation: u64) -> Result<()> {
        let account = self.with_its_unix_user(account)?;
        let generations = self.generations.lock().unwrap();
        anyhow::ensure!(
            generations.get(&user.to_string()) == Some(&generation),
            "Sign-in was cancelled; start again"
        );
        self.record(user, account)
    }

    #[cfg(test)]
    fn connect(&self, user: &UserId, account: Account) -> Result<()> {
        let account = self.with_its_unix_user(account)?;
        self.record(user, account)
    }

    /// The account with the Unix user it runs as, on a server that gives each person one:
    /// the one recorded for it, made when the machine has none. A user that cannot be made
    /// fails the sign-in, and nothing of it is saved.
    ///
    /// Done before the sign-in is written down and with no lock a request waits on: it runs
    /// `useradd`, and the person's own shell profile and git.
    fn with_its_unix_user(&self, mut account: Account) -> Result<Account> {
        if !crate::unix_users::each_person_has_one() {
            return Ok(account);
        }
        let _one_at_a_time = self.making_unix_users.lock().unwrap();
        let recorded = self
            .inner
            .lock()
            .unwrap()
            .accounts
            .get(&account.id)
            .and_then(|previous| previous.unix_user.clone());
        let unix_user =
            crate::unix_users::of_account(recorded.as_deref(), &account.login, &account.id)?;
        crate::unix_users::tell_git_who_they_are(&unix_user, &account.name, &account.email)?;
        account.unix_user = Some(unix_user.name);
        Ok(account)
    }

    fn record(&self, user: &UserId, mut account: Account) -> Result<()> {
        self.change(|inner| {
            if let Some(previous) = inner.accounts.get(&account.id) {
                account.layout = previous.layout.clone();
                if account.unix_user.is_none() {
                    account.unix_user = previous.unix_user.clone();
                }
            }
            inner
                .admissions
                .insert(user.to_string(), account.id.clone());
            inner.accounts.insert(account.id.clone(), account);
            Ok(())
        })
    }

    /// Identity only for a personal interactive shell. Credentials stay in short-lived
    /// commit/push runs; an unsigned-in shell must not silently commit as the host.
    pub(crate) fn shell_git_environment(&self, user: &UserId) -> Vec<(String, String)> {
        let account = self.account(user);
        let name = account.as_ref().map(|a| a.name.as_str()).unwrap_or("");
        let email = account.as_ref().map(|a| a.email.as_str()).unwrap_or("");
        vec![
            ("GIT_AUTHOR_NAME".into(), name.into()),
            ("GIT_COMMITTER_NAME".into(), name.into()),
            ("GIT_AUTHOR_EMAIL".into(), email.into()),
            ("GIT_COMMITTER_EMAIL".into(), email.into()),
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            ("GIT_CONFIG_KEY_0".into(), "commit.gpgSign".into()),
            ("GIT_CONFIG_VALUE_0".into(), "false".into()),
        ]
    }

    pub(crate) fn git_environment(&self, user: &UserId) -> Result<Vec<(String, String)>> {
        let account = self
            .account(user)
            .context("Connect your GitHub account from Account before committing or pushing")?;
        if account.token.is_empty() {
            bail!("Reconnect your GitHub account before committing or pushing");
        }
        let mut env = vec![
            ("GIT_AUTHOR_NAME".into(), account.name.clone()),
            ("GIT_COMMITTER_NAME".into(), account.name),
            ("GIT_AUTHOR_EMAIL".into(), account.email.clone()),
            ("GIT_COMMITTER_EMAIL".into(), account.email),
            ("GH_TOKEN".into(), account.token),
            ("GITHUB_TOKEN".into(), "".into()),
            ("GH_ENTERPRISE_TOKEN".into(), "".into()),
            ("GITHUB_ENTERPRISE_TOKEN".into(), "".into()),
            ("GH_HOST".into(), "github.com".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            // Never fall through to the host's SSH agent for a remote user's push.
            ("GIT_SSH_COMMAND".into(), "false".into()),
            ("GIT_ASKPASS".into(), "false".into()),
        ];
        // Reset inherited helpers and scope ours to GitHub HTTPS. Token values stay in the
        // child environment, never the script, command display, git config, or API response.
        let config = [
            ("credential.helper", ""),
            (
                "credential.https://github.com.helper",
                "!f() { protocol=; host=; while IFS= read -r line && [ -n \"$line\" ]; do case \"$line\" in protocol=*) protocol=${line#protocol=} ;; host=*) host=${line#host=} ;; esac; done; if [ \"$1\" = get ] && [ \"$protocol\" = https ] && [ \"$host\" = github.com ]; then printf 'username=x-access-token\\npassword=%s\\n' \"$GH_TOKEN\"; fi; }; f",
            ),
            ("url.https://github.com/.insteadOf", "git@github.com:"),
            ("url.https://github.com/.insteadOf", "ssh://git@github.com/"),
            ("commit.gpgSign", "false"),
            ("http.extraHeader", ""),
        ];
        // The run ignores the person's own git settings, and with them the leave to work in a
        // checkout that is somebody else's - see `crate::unix_users::tell_git_who_they_are`.
        let shared_checkout = [("safe.directory", "*")];
        let config: Vec<(&str, &str)> = match crate::unix_users::each_person_has_one() {
            true => config.into_iter().chain(shared_checkout).collect(),
            false => config.into_iter().collect(),
        };
        env.push(("GIT_CONFIG_COUNT".into(), config.len().to_string()));
        for (i, (key, value)) in config.iter().enumerate() {
            env.push((format!("GIT_CONFIG_KEY_{i}"), (*key).into()));
            env.push((format!("GIT_CONFIG_VALUE_{i}"), (*value).into()));
        }
        Ok(env)
    }
}
