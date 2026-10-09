//! Server asks - the socket a `moon serve` answers its shells on: `moon launch`, and what is
//! asked about the agents in the shells it holds - `moon agent start`, `tell` and `view`, and
//! `moon wire post @handle`.
//!
//! A `moon serve` is no window, and is written down as none: no shell is ever sent to it with
//! a file to open or a folder - see [`super::running`]. Its shells are started with its
//! process in [`super::WINDOW_ENV`] all the same, and that is how a command typed in one of
//! them finds this socket, which is named after the process the way a window's is.
//!
//! On a server that gives each person a Unix user the shells run as users who cannot enter
//! the server's home, so the socket is where all of them reach it - see [`super::SOCKET_ENV`]
//! - and each ask is answered for whoever asked: the person whose Unix user the process at
//! the other end of the connection runs as. What is done for them is done as them - see
//! [`crate::unix_users::as_user`]. Any of them may tell or look at any agent: the agents
//! of a server are shared, whoever started each.

use std::{
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    thread,
};

use anyhow::{Context, Result};

use super::{
    Answer, Ask, StartsApplications, answered, launched, listen_on_own_socket, read_ask,
    remove_records,
    window::{AgentAsks, not_a_line_of_the_wire},
    write_answer,
};
use crate::{
    api::{AgentKind, AppState, OpenSessionRequest, SessionOwner},
    moontasks::{
        StartFolder, StartResourceRequest, TaskResourceKind,
        wire::{from_the_agent_of, typed_line},
    },
    server::profiles::Profiles,
    terminal::Shown,
    unix_users::Person,
};

/// What a server keeps so its shells can reach it. Dropping it takes the socket away.
pub(crate) struct ServerAsks {
    shells: HoldsTheShells,
}

/// The shells a server holds and the people they are started for.
struct Shells {
    state: AppState,
    profiles: Profiles,
}

/// Who holds the server's shells, once the server has said - see
/// [`ServerAsks::shells_held_by`].
type HoldsTheShells = Arc<Mutex<Option<Arc<Shells>>>>;

impl ServerAsks {
    /// Open the server's socket and start answering on it, with `applications` starting what
    /// is asked for.
    pub(crate) fn listen(applications: Arc<dyn StartsApplications>) -> Result<Self> {
        let listener = listen_on_own_socket()?;
        let asks = Self {
            shells: Arc::new(Mutex::new(None)),
        };
        let shells = asks.shells.clone();
        thread::Builder::new()
            .name("moon-server-asks".to_string())
            .spawn(move || {
                // One ask per connection, each answered before the next is read, as a
                // window's are: a start is waited on for a second or two, and a shell asks
                // only as fast as somebody types.
                for stream in listener.incoming().flatten() {
                    // Taken out of the lock before the answer, which a start is waited on in.
                    let shells = shells.lock().expect("the shells lock").clone();
                    if let Err(error) = answer(stream, applications.as_ref(), shells.as_deref()) {
                        eprintln!("[moonreview] could not answer a `moon` ask: {error}");
                    }
                }
            })
            .context("failed to start the thread answering shells")?;
        Ok(asks)
    }

    /// Say which shells this server holds - `state`'s - and who its people are. Until it is
    /// said, what is asked about an agent is refused, and on a server that gives each person
    /// a Unix user so is everything else: it answers nobody it cannot name.
    pub(crate) fn shells_held_by(&self, state: AppState, profiles: Profiles) {
        *self.shells.lock().expect("the shells lock") = Some(Arc::new(Shells { state, profiles }));
    }
}

impl Drop for ServerAsks {
    fn drop(&mut self) {
        remove_records(std::process::id());
    }
}

/// How long a connection has to say what it asks. The ask is one line, written as the
/// connection is made.
const ASKED_WITHIN: std::time::Duration = std::time::Duration::from_secs(2);

/// What a server answers everything that is a window's to do with.
const HAS_NO_WINDOW: &str = "this moon is a `moon serve`, which has no window to do that in";

/// Read one ask off a connection and answer it, for whoever asked and as them - see
/// [`asker_of`]. That nobody can be named is the answer too, rather than a connection hung
/// up on: the command that asked prints it.
fn answer(
    stream: UnixStream,
    applications: &dyn StartsApplications,
    shells: Option<&Shells>,
) -> Result<()> {
    // A connection that asks nothing is hung up on: this thread answers one ask at a time,
    // and on a shared server its socket is open to every process of the machine.
    stream.set_read_timeout(Some(ASKED_WITHIN))?;
    let ask = read_ask(&stream)?;
    let answer = answered(asker_of(&stream, shells).and_then(|asker| match &asker {
        Some(person) => crate::unix_users::as_user(person, || {
            answer_to(ask, applications, shells, Some(person))
        }),
        None => Ok(answer_to(ask, applications, shells, None)),
    }));
    write_answer(&stream, &answer)
}

/// Who asks, on a server that gives each person a Unix user: the person whose user the
/// process at the other end of the connection runs as. A process of any other user is
/// refused - no shell of this server runs as one. Nobody, on every other server: its shells
/// all run as the server's own user.
fn asker_of(stream: &UnixStream, shells: Option<&Shells>) -> Result<Option<Person>> {
    if !crate::unix_users::each_person_has_one() {
        return Ok(None);
    }
    let shells = shells.context("this server has not said who its people are yet")?;
    let uid = uid_at_the_other_end(stream)?;
    shells
        .profiles
        .person_with_uid(uid)?
        .with_context(|| {
            format!(
                "the Unix user with id {uid} is not one this server made for somebody who \
                 signed in to it"
            )
        })
        .map(Some)
}

/// The Unix user the process that connected runs as, which the kernel wrote down as it
/// connected.
#[cfg(target_os = "linux")]
fn uid_at_the_other_end(stream: &UnixStream) -> Result<u32> {
    use std::os::fd::AsRawFd;

    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: both pointers are to locals that outlive the call, and `length` is the size of
    // what `credentials` points to.
    let asked = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if asked != 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to read who is at the other end of the socket");
    }
    Ok(credentials.uid)
}

/// Only Linux gives each person a Unix user - see [`crate::unix_users`] - so nowhere else is
/// there anybody to tell apart.
#[cfg(not(target_os = "linux"))]
fn uid_at_the_other_end(_stream: &UnixStream) -> Result<u32> {
    anyhow::bail!("who is at the other end of a socket is read on Linux only")
}

/// Answer one ask. What is asked about an agent is answered here and now, as a window
/// answers it - see [`AgentAsks`] - and so is a direct message of the wire, which a window
/// leaves for its next frame: a server draws none, and has no Messages to write the line in.
/// Everything that opens something is a window's to do, and is refused.
fn answer_to(
    ask: Ask,
    applications: &dyn StartsApplications,
    shells: Option<&Shells>,
    asker: Option<&Person>,
) -> Answer {
    let about_agents = |ask: &dyn Fn(&dyn AgentAsks) -> Result<Answer>| match shells {
        Some(shells) => {
            crate::api::mark_activity(&shells.state.last_activity);
            answered(ask(&AskedBy { shells, asker }))
        }
        None => Answer::Refused {
            reason: "this server answers nothing about agents".to_string(),
        },
    };
    match ask {
        Ask::Launch { command, folder } => launched(applications, asker, &command, &folder),
        Ask::StartAgent {
            repo_path,
            task_id,
            agent,
        } => about_agents(&|agents| {
            let run = agents.start(&repo_path, &task_id, agent)?;
            Ok(Answer::Started { run })
        }),
        Ask::Tell { terminal_id, line } => about_agents(&|agents| {
            agents.tell(&terminal_id, &line)?;
            Ok(Answer::Told)
        }),
        Ask::Shown {
            terminal_id,
            wanted,
        } => about_agents(&|agents| {
            let text = agents.shown(&terminal_id, wanted)?;
            Ok(Answer::Shown { text })
        }),
        Ask::Wire {
            terminal_id,
            sender,
            recipient,
            message,
        } => match not_a_line_of_the_wire(&sender, &recipient, &message) {
            Some(refused) => refused,
            None => about_agents(&|agents| {
                // Whose agent sent it is the server's to say: it knows who asked.
                let login = asker.map(|person| person.github_login.as_str());
                let typed = typed_line(&sender, &from_the_agent_of(login, &message));
                agents.tell(&terminal_id, &typed)?;
                Ok(Answer::Wired)
            }),
        },
        Ask::OpenFile { .. }
        | Ask::StillOpen { .. }
        | Ask::OpenShell { .. }
        | Ask::PickFile { .. } => Answer::Refused {
            reason: HAS_NO_WINDOW.to_string(),
        },
    }
}

/// The agents of a server, as one shell asks after them: what a window's moon does for the
/// window - see `crate::native::agent_asks` - done for whoever asked.
struct AskedBy<'asked> {
    shells: &'asked Shells,
    /// Who asked - see [`asker_of`]. An agent they start is theirs: it runs as their Unix
    /// user, and its card says so.
    asker: Option<&'asked Person>,
}

impl AgentAsks for AskedBy<'_> {
    /// The agent comes up in the repo and unattended, as one a window is asked for does. The
    /// session it is started from is the asker's own on that repo, kept apart from every
    /// browser's by being named after their Unix user - and the one on the repo that is
    /// nobody's, on a server that runs everything as itself.
    fn start(&self, repo_path: &str, task_id: &str, agent: AgentKind) -> Result<String> {
        let state = &self.shells.state;
        let owner = self.asker.map(|person| SessionOwner {
            namespace: person.unix_user.name.clone(),
            person: Some(person.clone()),
        });
        let session_id = crate::service::open_session_for_profile(
            state,
            OpenSessionRequest {
                repo_path: repo_path.to_string(),
                diff_target: None,
                active_commit: None,
            },
            owner,
        )?
        .session_id;
        let terminal_id = crate::moontasks::service::start_resource(
            state,
            &session_id,
            task_id,
            StartResourceRequest {
                kind: TaskResourceKind::Agent,
                agent,
                opens_in: StartFolder::Repo,
                unattended: true,
            },
        )?;
        state
            .terminals
            .name(&terminal_id)
            .context("the run was started with no name")
    }

    fn tell(&self, terminal_id: &str, line: &str) -> Result<()> {
        self.shells.state.terminals.tell(terminal_id, line)
    }

    fn shown(&self, terminal_id: &str, wanted: Shown) -> Result<String> {
        self.shells.state.terminals.shown(terminal_id, wanted)
    }
}
