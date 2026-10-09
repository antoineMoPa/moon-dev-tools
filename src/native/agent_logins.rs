//! Agent logins - which agents whoever is at this window is not logged in to, and the shell
//! that logs them in.
//!
//! Only a server that gives each person a Unix user says: an agent there is installed once
//! and logged in to by each person, in a home of their own - see `AgentOption::logged_in`.
//! What it says is a hint, read off the files a login leaves and not by trying one. So an
//! agent marked here is offered all the same, with the way to log in beside it.

use crate::{
    api::{AgentKind, AgentOption},
    native::app::App,
};

/// What a menu says after the name of an agent the person is not logged in to.
pub(crate) const NOT_LOGGED_IN: &str = "not logged in";

/// What the person types in a shell of theirs to log in to this agent, when the server says
/// they are not logged in to it. `None` for an agent they are logged in to, and wherever the
/// server does not ask.
pub(crate) fn log_in_command_of(option: &AgentOption) -> Option<&str> {
    match option.logged_in {
        Some(false) => Some(option.log_in_command.as_deref().unwrap_or_else(|| {
            panic!(
                "the server says {} is not logged in, and not how to log in",
                option.label
            )
        })),
        Some(true) | None => None,
    }
}

/// The same for each agent of the review this window is on, by which agent it is.
pub(crate) fn log_in_commands(app: &App) -> Vec<(AgentKind, String)> {
    let Some(payload) = app
        .model
        .review_ref(&app.model.root_session_id)
        .and_then(|review| review.payload.as_ref())
    else {
        return Vec::new();
    };
    payload
        .available_agents
        .iter()
        .filter_map(|option| Some((option.kind, log_in_command_of(option)?.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_agent_the_server_says_is_not_logged_in_has_a_way_to_log_in() {
        let claude = |logged_in: Option<bool>| AgentOption {
            kind: AgentKind::Claude,
            label: "Claude".to_string(),
            available: true,
            logged_in,
            log_in_command: logged_in.map(|_| "claude auth login".to_string()),
        };

        assert_eq!(log_in_command_of(&claude(Some(false))), Some("claude auth login"));
        assert_eq!(log_in_command_of(&claude(Some(true))), None);
        // A server that runs everyone as itself does not ask.
        assert_eq!(log_in_command_of(&claude(None)), None);
    }
}
