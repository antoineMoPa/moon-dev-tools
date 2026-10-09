//! Agent homes - whose home directory an agent keeps its own files in, and how the server
//! reads and writes there.
//!
//! An agent keeps its sessions, its login and what it shows under the home of the Unix user it
//! runs as. Where the server runs everything as itself, that is the server's own home. Where
//! each person has a Unix user - see [`crate::unix_users`] - it is the home of whoever started
//! the run, which only that person and root can read: a request's thread works as whoever
//! asked, and a run is whoever's started it.

use std::{path::PathBuf, process::Command};

use anyhow::Result;

use crate::{
    api::AppState,
    moontasks::store::StartedBy,
    unix_users::{Person, UnixUser},
};

/// The home an agent's files are in, by whose it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AgentHome {
    /// The home of the user the server runs as, which every agent it starts runs as too.
    OfTheServer,
    /// The home of one person, on a server that gives each a Unix user.
    Of(Person),
}

impl AgentHome {
    /// The home of whoever a shell's program runs as - see `TerminalSpec::runs_as`.
    pub(crate) fn of_whoever_runs(runs_as: Option<&Person>) -> Self {
        match runs_as {
            Some(person) => Self::Of(person.clone()),
            None => Self::OfTheServer,
        }
    }

    /// The home of the person a session works for, which is where the agents they start keep
    /// their sessions.
    pub(crate) fn of_session(state: &AppState, session_id: &str) -> Result<Self> {
        let person =
            crate::unix_users::user_to_work_as(crate::api::person_of(state, session_id)?)?;
        Ok(Self::of_whoever_runs(person.as_ref()))
    }

    /// The home of whoever started a recorded run. `None` when the server gives each person a
    /// Unix user and the run names none this machine has: there is no home its agent could
    /// have written in.
    ///
    /// The server's own on a server that runs everything as itself, whatever the record says:
    /// a board can be carried to one from a server that gave each person a Unix user.
    pub(crate) fn of_run(started_by: Option<&StartedBy>) -> Result<Option<Self>> {
        if !crate::unix_users::each_person_has_one() {
            return Ok(Some(Self::OfTheServer));
        }
        let Some(started_by) = started_by else {
            return Ok(None);
        };
        Ok(person_who_started(started_by)?.map(Self::Of))
    }

    /// Whether this is the home of the same Unix user as `other`. Not asked of who they are on
    /// GitHub, where a person can take another name and keep their user and their home.
    pub(crate) fn has_the_owner_of(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::OfTheServer, Self::OfTheServer) => true,
            (Self::Of(one), Self::Of(other)) => one.unix_user.uid == other.unix_user.uid,
            (Self::OfTheServer, Self::Of(_)) | (Self::Of(_), Self::OfTheServer) => false,
        }
    }

    /// The directory itself. `None` for a server whose account has no home directory.
    pub(crate) fn dir(&self) -> Option<PathBuf> {
        match self {
            Self::OfTheServer => std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from),
            Self::Of(person) => Some(person.unix_user.home.clone()),
        }
    }

    /// Do `read` on this thread, able to read the home. A person's home is theirs alone to
    /// read: the asker's own is read as them, and anybody else's as the server, which is root
    /// wherever a home has a person.
    pub(crate) fn reading<T>(&self, read: impl FnOnce() -> T) -> Result<T> {
        let working_as = crate::unix_users::current_user().map(|person| person.unix_user.uid);
        match self {
            Self::Of(owner) if working_as != Some(owner.unix_user.uid) => {
                crate::unix_users::as_the_server(read)
            }
            _ => Ok(read()),
        }
    }

    /// Do `work` on this thread as the home's owner, so that what it makes in the home is
    /// theirs, whoever asked for it.
    pub(crate) fn as_its_owner<T>(&self, work: impl FnOnce() -> T) -> Result<T> {
        match self {
            Self::Of(owner) => crate::unix_users::as_user(owner, work),
            Self::OfTheServer => Ok(work()),
        }
    }

    /// `program`, to be run as the home's owner and looked for where they installed it: on a
    /// person's own login `PATH`, and on the server's - see [`crate::shell_path`] - where the
    /// server runs everything as itself.
    pub(crate) fn command(&self, program: &str) -> Result<Command> {
        match self {
            Self::Of(owner) => owner.unix_user.command(program),
            Self::OfTheServer => crate::shell_path::installed_tool(program),
        }
    }
}

/// The person a run's record names, as this machine has them. `None` when it has no such Unix
/// user: moon removes none, so the record was written on another machine.
pub(crate) fn person_who_started(started_by: &StartedBy) -> Result<Option<Person>> {
    Ok(
        UnixUser::named(&started_by.unix_user)?.map(|unix_user| Person {
            github_login: started_by.github_login.clone(),
            unix_user,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No test's server gives anybody a Unix user, which is every server but a shared one.
    #[test]
    fn where_nobody_has_a_unix_user_a_run_is_read_from_the_servers_home_whoever_started_it() {
        let carried_from_a_shared_server = StartedBy {
            github_login: "someone".to_string(),
            unix_user: "moon-someone".to_string(),
        };

        for started_by in [None, Some(&carried_from_a_shared_server)] {
            assert_eq!(
                AgentHome::of_run(started_by).expect("expected a home"),
                Some(AgentHome::OfTheServer)
            );
        }
        assert_eq!(
            AgentHome::OfTheServer.dir(),
            std::env::var_os("HOME").map(PathBuf::from)
        );
    }
}
