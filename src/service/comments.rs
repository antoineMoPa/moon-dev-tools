//! The comments on a review: resolved, changed, and sent to the agent that is to act on them.

use anyhow::{Result, bail};

use crate::{
    api::{AgentLogPayload, AppState, RepoSession},
    comments::{
        agent_dispatch_log, anchored_comment_key, build_anchored_comment_value,
        cancel_comment_dispatch, parse_anchored_comments, plan_batched_comment_dispatches,
        plan_comment_dispatches, spawn_comment_dispatch,
    },
};

pub(crate) fn resolve_comment(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
    comment_index: usize,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        let Some(existing) = session.comments.get(hunk_id).cloned() else {
            bail!("comment no longer exists");
        };

        let mut anchored = parse_anchored_comments(&existing);
        let Some(entry) = anchored.get_mut(comment_index) else {
            bail!("comment index is out of bounds");
        };
        entry.resolved = true;

        store_anchored_comments(session, hunk_id, &anchored);
        Ok(())
    })
}

pub(crate) fn resolve_comment_by_key(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
    comment_key: &str,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        let Some(existing) = session.comments.get(hunk_id).cloned() else {
            bail!("comment no longer exists");
        };

        let mut anchored = parse_anchored_comments(&existing);
        let Some(index) = anchored
            .iter()
            .position(|entry| anchored_comment_key(entry) == comment_key)
        else {
            bail!("comment no longer exists");
        };
        anchored[index].resolved = true;

        store_anchored_comments(session, hunk_id, &anchored);
        Ok(())
    })
}

fn store_anchored_comments(
    session: &mut RepoSession,
    hunk_id: &str,
    anchored: &[crate::comments::AnchoredComment],
) {
    let next = build_anchored_comment_value(anchored);
    if next.trim().is_empty() {
        session.comments.remove(hunk_id);
    } else {
        session.comments.insert(hunk_id.to_string(), next);
    }
}

pub(crate) fn update_comment(
    state: &AppState,
    session_id: &str,
    request: &crate::api::CommentRequest,
) -> Result<()> {
    let dispatch_jobs = crate::api::with_session(state, session_id, |session| {
        plan_comment_dispatches(session, session_id, request)
    })?;

    for job in dispatch_jobs {
        spawn_comment_dispatch(state.clone(), job);
    }

    Ok(())
}

pub(crate) fn send_comment_batch(state: &AppState, session_id: &str) -> Result<()> {
    let dispatch_jobs = crate::api::with_session(state, session_id, |session| {
        plan_batched_comment_dispatches(session, session_id)
    })?;

    for job in dispatch_jobs {
        spawn_comment_dispatch(state.clone(), job);
    }

    Ok(())
}

pub(crate) fn cancel_dispatch(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
    comment_index: usize,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        cancel_comment_dispatch(session, hunk_id, comment_index)
    })
}

pub(crate) fn dispatch_log(
    state: &AppState,
    session_id: &str,
    dispatch_key: &str,
) -> Result<AgentLogPayload> {
    crate::api::with_session(state, session_id, |session| {
        Ok(AgentLogPayload {
            dispatch_key: dispatch_key.to_string(),
            text: agent_dispatch_log(session, dispatch_key)?,
        })
    })
}
