//! Review comments handed to an agent already running on a task: pasted into its shell, with
//! the commented lines as a patch, rather than given to a run started for them.
//!
//! The comment is kept on the review like any other, and its dispatch says how the handing
//! over went. A paste can wait for as long as the agent is asking a person something - see
//! `told` in [`crate::terminal`] - so `Queued` is what the card reads until it has gone.

use std::sync::{Arc, atomic::AtomicBool};

use anyhow::{Result, anyhow};

use super::{
    CommentDispatchState, dispatch_key, parse_anchored_comments,
    server_side::{CommentUpdate, anchored_comment_key, apply_comment_update},
};
use crate::{
    api::{
        AgentKind, AppState, CancelToken, CommentDispatchStatus, CommentRequest, RepoSession,
        RunningAgent, with_session,
    },
    git::collect_session_hunks,
    terminal::PasteEnded,
};

/// What is to be pasted into an agent, and the dispatches that say how it went.
pub(crate) struct ToldComments {
    terminal_id: String,
    agent_name: String,
    message: String,
    dispatch_keys: Vec<String>,
    cancel_token: CancelToken,
}

/// One comment as the agent reads it. The repo is named because a task's review can span
/// several, and the paths in a patch start at the root of one of them.
fn comment_message(repo: &std::path::Path, patch_excerpt: &str, comment: &str) -> String {
    format!(
        "Review comment on {}\n{patch_excerpt}\n\n{comment}",
        repo.display()
    )
}

fn waiting(agent: &RunningAgent, cancel_token: &CancelToken) -> CommentDispatchState {
    CommentDispatchState {
        status: CommentDispatchStatus::Queued,
        detail: format!("Waiting for `{}`.", agent.name),
        agent: AgentKind::None,
        cancel_token: Some(Arc::clone(cancel_token)),
        log: None,
        patch_excerpt: String::new(),
    }
}

/// Store the comment the request adds, and put together the message that hands it over.
pub(crate) fn plan_told_comment(
    session: &mut RepoSession,
    request: &CommentRequest,
    agent: &RunningAgent,
) -> Result<ToldComments> {
    if request.patch_excerpt.trim().is_empty() {
        return Err(anyhow!("a comment for {} has no patch", agent.name));
    }
    let update = apply_comment_update(session, request);
    let added = added_comment(&update)?;
    let cancel_token: CancelToken = Arc::new(AtomicBool::new(false));
    let key = dispatch_key(&request.hunk_id, added);
    let message = comment_message(&session.repo_path, &request.patch_excerpt, &added.comment);
    session
        .comment_dispatches
        .insert(key.clone(), waiting(agent, &cancel_token));

    Ok(ToldComments {
        terminal_id: agent.terminal_id.clone(),
        agent_name: agent.name.clone(),
        message,
        dispatch_keys: vec![key],
        cancel_token,
    })
}

fn added_comment(update: &CommentUpdate) -> Result<&super::AnchoredComment> {
    let before = update
        .previous_anchored
        .iter()
        .map(anchored_comment_key)
        .collect::<std::collections::HashSet<_>>();
    update
        .next_anchored
        .iter()
        .find(|entry| !before.contains(&anchored_comment_key(entry)))
        .ok_or_else(|| anyhow!("that comment is already on the hunk"))
}

/// Move every held comment to waiting on the agent, in the order of the diff, and put together
/// the one message that hands them over.
pub(crate) fn plan_told_held_comments(
    session: &mut RepoSession,
    agent: &RunningAgent,
) -> Result<ToldComments> {
    let mut messages = Vec::new();
    let mut dispatch_keys = Vec::new();

    for hunk in collect_session_hunks(session)? {
        let stored = session.comments.get(&hunk.id).cloned().unwrap_or_default();
        for entry in parse_anchored_comments(&stored)
            .iter()
            .filter(|entry| !entry.resolved)
        {
            let key = dispatch_key(&hunk.id, entry);
            let Some(dispatch) = session.comment_dispatches.get(&key) else {
                continue;
            };
            if dispatch.status != CommentDispatchStatus::Batched {
                continue;
            }
            if dispatch.patch_excerpt.trim().is_empty() {
                return Err(anyhow!("a comment held for {} has no patch", agent.name));
            }
            messages.push(comment_message(
                &session.repo_path,
                &dispatch.patch_excerpt,
                &entry.comment,
            ));
            dispatch_keys.push(key);
        }
    }

    if messages.is_empty() {
        return Err(anyhow!("no comment is being held"));
    }
    // Only now that all of them can go: a refusal above leaves every one of them held.
    let cancel_token: CancelToken = Arc::new(AtomicBool::new(false));
    for key in &dispatch_keys {
        session
            .comment_dispatches
            .insert(key.clone(), waiting(agent, &cancel_token));
    }
    Ok(ToldComments {
        terminal_id: agent.terminal_id.clone(),
        agent_name: agent.name.clone(),
        message: messages.join("\n\n"),
        dispatch_keys,
        cancel_token,
    })
}

/// Paste the message into the agent's shell. Answers once it is waiting its turn there; the
/// dispatches say how it ended. A shell that cannot be told at all fails them here, so the
/// comment says so as well as whoever asked.
pub(crate) fn tell_running_agent(
    state: &AppState,
    session_id: &str,
    told: ToldComments,
) -> Result<()> {
    let ToldComments {
        terminal_id,
        agent_name,
        message,
        dispatch_keys,
        cancel_token,
    } = told;

    let on_ended = {
        let state = state.clone();
        let session_id = session_id.to_string();
        let dispatch_keys = dispatch_keys.clone();
        let agent_name = agent_name.clone();
        move |ended| record_how_it_ended(&state, &session_id, &dispatch_keys, &agent_name, ended)
    };

    let refused = state.terminals.tell_paste(
        &terminal_id,
        &message,
        Arc::clone(&cancel_token),
        Box::new(on_ended),
    );
    if let Err(error) = &refused {
        record_how_it_ended(
            state,
            session_id,
            &dispatch_keys,
            &agent_name,
            PasteEnded::Failed(error.to_string()),
        );
    }
    refused
}

fn record_how_it_ended(
    state: &AppState,
    session_id: &str,
    dispatch_keys: &[String],
    agent_name: &str,
    ended: PasteEnded,
) {
    let (status, detail) = match ended {
        PasteEnded::Sent => (
            CommentDispatchStatus::Completed,
            format!("Told `{agent_name}`."),
        ),
        PasteEnded::Canceled => (
            CommentDispatchStatus::Canceled,
            format!("Not sent to `{agent_name}`."),
        ),
        PasteEnded::Failed(reason) => (CommentDispatchStatus::Failed, reason),
    };
    // A session gone by now has no comment left to say this on.
    let _ = with_session(state, session_id, |session| {
        for key in dispatch_keys {
            if let Some(dispatch) = session.comment_dispatches.get_mut(key) {
                dispatch.status = status;
                dispatch.detail = detail.clone();
                dispatch.cancel_token = None;
            }
        }
        Ok(())
    });
}
