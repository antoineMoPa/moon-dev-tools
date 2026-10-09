//! Current user - the person a thread of the server is doing its own work as: what it writes
//! is theirs, and what it runs, runs as them.
//!
//! A request is answered by the server's own code - a file saved, a hunk staged, a card
//! written - and the server is root. [`as_user`] is how that work is done as the person who
//! asked: for as long as it lasts they are the thread's current user, files the thread reads
//! and makes are read and made as their Unix user, and a program started through [`command`]
//! - git, above all - runs as that user.
//!
//! The current user is this thread's and no other's, so the work inside is synchronous:
//! nothing in it is awaited, and a thread it starts has no current user until it is given
//! one - see [`carried`].
//!
//! A thread started in the middle of it is the trap: Linux hands a new thread the file
//! identity of the thread that started it, for good, while its current user starts out as
//! nobody. So a thread that goes on to read or write files is started through [`carried`],
//! and the server's runtime makes each of its threads the server's as it starts - see
//! [`be_the_server_on_this_thread`]. The threads that only carry bytes - a pty's reader, a
//! pipe's - are started as they are and never look at a file.

use std::{cell::RefCell, process::Command};

use anyhow::{Context, Result, bail};

use super::{Person, each_person_has_one};

thread_local! {
    /// The person this thread is working as, and `None` when it is the server being itself.
    static CURRENT_USER: RefCell<Option<Person>> = const { RefCell::new(None) };
}

/// Do `work` on this thread as `person` - see the module. Whoever the thread's current user
/// was before is its current user again afterwards, however `work` ends.
pub(crate) fn as_user<T>(person: &Person, work: impl FnOnce() -> T) -> Result<T> {
    let _switched = Switched::to(Some(person.clone()))?;
    Ok(work())
}

/// Do `work` as the server itself, in the middle of work done as a person: reading what only
/// root reads - another person's home, where their agent keeps its sessions. Nothing is run
/// in here: [`command`] refuses a thread with no current user.
pub(crate) fn as_the_server<T>(work: impl FnOnce() -> T) -> Result<T> {
    let _switched = Switched::to(None)?;
    Ok(work())
}

/// Make this thread the server's own, whoever's file identity it was started with: what each
/// thread of the server's runtime does first, since one of them may be started by a thread
/// that is working as a person at that moment.
pub(crate) fn be_the_server_on_this_thread() {
    if each_person_has_one() {
        take_file_identity(None).expect("a thread of the server takes the server's file identity");
    }
}

/// The person this thread is working as - see [`as_user`]. `None` for the server being
/// itself, which is every thread of a server that gives nobody a Unix user.
pub(crate) fn current_user() -> Option<Person> {
    CURRENT_USER.with(|user| user.borrow().clone())
}

/// The GitHub login of whoever this is being done for, to write on what is made: this
/// thread's current user, in a server; the person this shell was started for, in a `moon`
/// typed in one. `None` wherever nobody has a Unix user of their own.
pub(crate) fn person_this_is_done_for() -> Option<String> {
    current_user()
        .map(|person| person.github_login)
        .or_else(crate::instances::shell_person)
}

/// `work` for another thread to run, as this thread's current user: how work a request
/// starts and does not wait for - an agent sent a comment - stays the person's.
pub(crate) fn carried<T>(work: impl FnOnce() -> T) -> impl FnOnce() -> Result<T> {
    let person = current_user();
    move || match &person {
        Some(person) => as_user(person, work),
        None => Ok(work()),
    }
}

/// `program`, to be run as this thread's current user.
///
/// On a server that gives nobody a Unix user that is the server's own user, and this is
/// `Command::new`. On one that does, a thread with no current user is refused: the server is
/// root there, and runs no program as itself.
pub(crate) fn command(program: &str) -> Result<Command> {
    match current_user() {
        Some(person) => person.unix_user.command(program),
        None if each_person_has_one() => bail!(
            "{program} was about to be run for nobody, on a server that runs each person's \
             programs as their own Unix user"
        ),
        None => Ok(Command::new(program)),
    }
}

/// One stretch of a thread working as somebody else than it was, ended by dropping it.
struct Switched {
    before: Option<Person>,
}

impl Switched {
    fn to(person: Option<Person>) -> Result<Self> {
        take_file_identity(person.as_ref())?;
        let before = CURRENT_USER.with(|user| user.replace(person));
        Ok(Self { before })
    }
}

impl Drop for Switched {
    fn drop(&mut self) {
        let before = self.before.take();
        // A thread left with a person's identity would do the next request's work as them,
        // and the next request is anybody's.
        take_file_identity(before.as_ref())
            .expect("a thread takes back the file identity it had before");
        CURRENT_USER.with(|user| user.replace(before));
    }
}

/// Make this thread read and write files as `person`'s Unix user, or as the server again.
///
/// The ids files are checked against are a thread's own on Linux, apart from the ids the
/// process runs as, and root may set them to anyone's. Becoming somebody who is not root also
/// puts down root's leave to pass every permission check, so what the person may not read,
/// this thread may not either.
#[cfg(target_os = "linux")]
fn take_file_identity(person: Option<&Person>) -> Result<()> {
    // SAFETY: reads this process's own credentials.
    let (uid, gid) = match person {
        Some(person) => (person.unix_user.uid, person.unix_user.gid),
        None => unsafe { (libc::geteuid(), libc::getegid()) },
    };
    // SAFETY: all of these change the calling thread's credentials and nothing else. Neither
    // call says whether it worked: each answers with the id from before, so each is asked
    // again.
    let (group_before, user_before) = unsafe { (libc::setfsgid(gid), libc::setfsuid(uid)) };
    let taken = unsafe { libc::setfsgid(gid) as u32 == gid && libc::setfsuid(uid) as u32 == uid };
    if !taken {
        // Half of an identity is nobody's: back to the one the thread came with.
        unsafe {
            libc::setfsuid(user_before as u32);
            libc::setfsgid(group_before as u32);
        }
        bail!("this thread could not take the file identity {uid}:{gid}");
    }
    Ok(())
}

/// Only Linux gives each person a Unix user - see [`super::decide_for_this_server`] - so
/// anywhere else there is only ever the server's own identity to take, which the thread has.
#[cfg(not(target_os = "linux"))]
fn take_file_identity(person: Option<&Person>) -> Result<()> {
    anyhow::ensure!(
        person.is_none() || !each_person_has_one(),
        "a thread takes a person's file identity on Linux only"
    );
    Ok(())
}

/// The person a session's work is done as: the session's own person, on a server that gives
/// each a Unix user, where a session that is nobody's is refused; nobody on any other server.
pub(crate) fn user_to_work_as(session_person: Option<Person>) -> Result<Option<Person>> {
    if !each_person_has_one() {
        return Ok(None);
    }
    session_person
        .map(Some)
        .context("this session is nobody's, on a server that does each person's work as them")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unix_users::UnixUser;

    /// The user the tests run as, which is the one identity a test may take.
    fn this_user() -> Person {
        // SAFETY: reads this process's own credentials.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        Person {
            github_login: "someone".to_string(),
            unix_user: UnixUser {
                name: "moon-someone".to_string(),
                uid,
                gid,
                home: std::env::temp_dir(),
                shell: "/bin/sh".to_string(),
            },
        }
    }

    #[test]
    fn a_thread_works_as_a_person_only_for_as_long_as_the_work_lasts() {
        let person = this_user();

        let inside = as_user(&person, current_user).expect("expected to work as them");

        assert_eq!(inside, Some(person));
        assert_eq!(current_user(), None);
    }

    #[test]
    fn a_server_that_gives_nobody_a_unix_user_runs_a_program_as_itself() {
        let command = command("git").expect("expected a command");

        assert_eq!(command.get_program(), "git");
        assert_eq!(command.get_args().count(), 0);
    }

    #[test]
    fn work_carried_to_another_thread_is_still_the_persons_and_the_servers_work_is_nobodys() {
        let person = this_user();

        let (carried_to_a_thread, as_server) = as_user(&person, || {
            let work = carried(current_user);
            let on_a_thread = std::thread::spawn(work).join().expect("the thread ran");
            (on_a_thread, as_the_server(current_user))
        })
        .expect("expected to work as them");

        assert_eq!(
            carried_to_a_thread.expect("expected to work as them"),
            Some(person)
        );
        assert_eq!(as_server.expect("expected to be the server"), None);
    }
}
