//! Agent logins - which agents one person can start: the ones installed for them, and whether
//! they are logged in to each.
//!
//! Where the server runs everything as itself there is one user to ask about, and it was asked
//! as the server started - `AppState::agent_availability`. Whether an agent is logged in is
//! not asked there: there is one login, and whoever runs moon made it. Where each person has a
//! Unix user - see [`crate::unix_users`] - both are that person's: an agent is installed for
//! them when it is on their login `PATH`, and logged in when their home holds the file the
//! agent keeps its login in.

use std::{ffi::OsString, path::Path};

use anyhow::Result;

use super::AgentHome;
use crate::{
    api::{AgentAvailability, AgentKind, AgentOption, AppState},
    unix_users::Person,
};

/// How one agent's login is told from the home of whoever runs it, and how it is made.
struct AgentLogin {
    agent: AgentKind,
    /// The file the agent keeps its login in, from the home directory - on Linux, the one
    /// system a person has a Unix user on.
    credentials: &'static str,
    /// What a person types in a shell of theirs to log in.
    log_in_command: &'static str,
}

/// Every agent moon starts. One set up to keep its login somewhere else - a keyring - has no
/// such file and works all the same, so a login that is missing is something to tell the
/// person before they start a task, never a reason to refuse the start.
const AGENT_LOGINS: &[AgentLogin] = &[
    AgentLogin {
        agent: AgentKind::Claude,
        credentials: ".claude/.credentials.json",
        log_in_command: "claude auth login",
    },
    AgentLogin {
        agent: AgentKind::Codex,
        credentials: ".codex/auth.json",
        log_in_command: "codex login",
    },
    AgentLogin {
        agent: AgentKind::OpenCode,
        credentials: ".local/share/opencode/auth.json",
        log_in_command: "opencode auth login",
    },
    AgentLogin {
        agent: AgentKind::Pi,
        credentials: ".pi/agent/auth.json",
        // Pi has no command that logs in: it is asked from inside, with `/login`.
        log_in_command: "pi",
    },
];

/// Which agents the owner of this home has installed.
pub(crate) fn availability_in(state: &AppState, home: &AgentHome) -> Result<AgentAvailability> {
    match home {
        AgentHome::OfTheServer => Ok(state.agent_availability),
        AgentHome::Of(person) => {
            let path = login_path_of(person)?;
            // What a person installed is in their home, which the asker may not be the one
            // to look into.
            home.reading(|| crate::agent::detect_agent_availability_on(&path))
        }
    }
}

/// Which agents the person a session works for has installed.
pub(crate) fn availability_for(state: &AppState, session_id: &str) -> Result<AgentAvailability> {
    availability_in(state, &AgentHome::of_session(state, session_id)?)
}

/// The agents the person a session works for is offered: which are installed for them and,
/// where each person has a Unix user, whether they are logged in to each.
pub(crate) fn agent_options_for(state: &AppState, session_id: &str) -> Result<Vec<AgentOption>> {
    let home = AgentHome::of_session(state, session_id)?;
    let mut options = crate::agent::agent_options(availability_in(state, &home)?);
    if let AgentHome::Of(person) = &home {
        home.reading(|| {
            for option in &mut options {
                say_whether_logged_in(option, &person.unix_user.home);
            }
        })?;
    }
    Ok(options)
}

/// Whether the agent is logged in for the person whose home this is, and how they log in.
/// Said of an agent they have installed: there is no logging in to one they have not.
fn say_whether_logged_in(option: &mut AgentOption, home_dir: &Path) {
    let login = AGENT_LOGINS
        .iter()
        .find(|login| login.agent == option.kind)
        .filter(|_| option.available);
    // "No agent" is the other option with no login to speak of.
    let Some(login) = login else {
        return;
    };
    option.logged_in = Some(home_dir.join(login.credentials).is_file());
    option.log_in_command = Some(login.log_in_command.to_string());
}

/// The login `PATH` of a person's Unix user, which is where what they installed is found.
fn login_path_of(person: &Person) -> Result<OsString> {
    Ok(OsString::from(person.unix_user.login_path()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_agent_is_logged_in_when_the_home_holds_its_login() {
        let home = std::env::temp_dir().join(format!(
            "moon-agent-logins-{}",
            crate::moontasks::store::new_uuid()
        ));
        std::fs::create_dir_all(home.join(".codex")).expect("expected the folder");
        std::fs::write(home.join(".codex/auth.json"), "{}").expect("expected the login");
        let mut options = crate::agent::agent_options(AgentAvailability {
            claude: true,
            codex: true,
            opencode: true,
            pi: false,
        });

        for option in &mut options {
            say_whether_logged_in(option, &home);
        }

        let said: Vec<(AgentKind, Option<bool>, Option<&str>)> = options
            .iter()
            .map(|option| {
                (
                    option.kind,
                    option.logged_in,
                    option.log_in_command.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                (AgentKind::None, None, None),
                (AgentKind::Claude, Some(false), Some("claude auth login")),
                (AgentKind::Codex, Some(true), Some("codex login")),
                (
                    AgentKind::OpenCode,
                    Some(false),
                    Some("opencode auth login")
                ),
                (AgentKind::Pi, None, None),
            ]
        );

        std::fs::remove_dir_all(home).expect("failed to remove the test home");
    }
}
