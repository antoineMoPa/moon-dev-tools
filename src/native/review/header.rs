//! The strip at the top of a review: what is being reviewed and the review-wide actions.
//! The agent selector lives in the comment composer, but is defined here with the batch
//! send that shares its choice.

use egui::{Align, Layout, RichText, Ui};

use crate::{
    api::{AgentKind, CommentDispatchStatus},
    native::{
        app::App,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// What the review is currently pointed at: the working tree, or one commit.
fn review_label(payload: &crate::api::SessionPayload) -> String {
    let Some(active) = payload.active_commit.as_deref() else {
        return "local changes".to_string();
    };
    payload
        .commits
        .iter()
        .chain(payload.history_commits.iter())
        .find(|commit| commit.sha == active)
        .map(|commit| format!("{} {}", commit.short_sha, commit.subject))
        .unwrap_or_else(|| active.chars().take(7).collect())
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, session_id: &str, palette: &Palette) {
    let Some(payload) = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.payload.clone())
    else {
        return;
    };

    let repo_name = &payload.repo_name;
    let repo_path = &payload.repo_path;
    let label = review_label(&payload);
    let read_only = payload.read_only;
    let selected_agent = payload.selected_agent;
    // Only comments actually held for a batch can be batch-sent. Counting every unresolved
    // comment here would offer to send ones that were already dispatched, and the send would
    // then quietly do nothing.
    let batched = payload
        .review_comments
        .iter()
        .filter(|comment| comment.dispatch.status == CommentDispatchStatus::Batched)
        .count();
    let full_file_path = payload.full_file_path.as_deref();

    ui.horizontal(|ui| {
        ui.label(RichText::new(repo_name).strong())
            .on_hover_text(repo_path);
        ui.label(RichText::new("·").color(palette.line));
        // The header is one line: a long subject is cut short here and read in full on hover.
        ui.add(egui::Label::new(RichText::new(label.as_str()).color(palette.accent)).truncate())
            .on_hover_text(format!("{label}\n\nwhat this review is pointed at"));

        if read_only {
            widgets::pill(ui, "read only", palette.muted, palette.status_neutral_bg)
                .on_hover_text("nothing here can be staged or discarded");
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if batched > 0 {
                let has_agent = selected_agent != AgentKind::None;
                let label = format!("send {batched} to {}", selected_agent.label());
                let button =
                    widgets::clickable(ui.add_enabled(has_agent, egui::Button::new(label)));
                let button = if has_agent {
                    button.on_hover_text("hand every held comment to the agent now")
                } else {
                    button.on_disabled_hover_text("pick an agent first")
                };

                if button.clicked() {
                    let for_call = session_id.to_string();
                    let note = format!(
                        "sent {batched} comment{} to {}",
                        if batched == 1 { "" } else { "s" },
                        selected_agent.label()
                    );
                    app.tasks.act_and_say(
                        session_id,
                        "could not send the comments",
                        note,
                        move |backend| backend.send_comment_batch(&for_call),
                    );
                }
            }

            if let Some(path) = full_file_path
                && widgets::quiet_button(ui, "view file")
                    .on_hover_text(format!("show all of {path}"))
                    .clicked()
            {
                app.open_file_pane(session_id, path);
            }
        });
    });
}

/// The picker for where comments go. It lives in the comment composer, beside the button
/// that sends the comment — and more than one composer can be open, so each brings a salt
/// of its own to keep the drop-downs apart.
pub(crate) fn draw_agent_select(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    salt: u64,
    selected: AgentKind,
    agents: &[crate::api::AgentOption],
    palette: &Palette,
) {
    let selected_label = agents
        .iter()
        .find(|agent| agent.kind == selected)
        .map(|agent| agent.label.clone())
        .unwrap_or_else(|| selected.label().to_string());

    egui::ComboBox::from_id_salt(("agent-select", session_id, salt))
        .selected_text(RichText::new(selected_label).size(SMALL_SIZE))
        .width(112.0)
        .show_ui(ui, |ui| {
            for agent in agents {
                // An agent that is not installed is listed but cannot be picked, so the
                // menu explains the gap rather than hiding it.
                let response = ui
                    .add_enabled_ui(agent.available, |ui| {
                        ui.selectable_label(agent.kind == selected, agent.label.as_str())
                    })
                    .inner;
                if !agent.available {
                    response.on_hover_text(format!("{} is not on PATH", agent.label));
                    continue;
                }
                if response.clicked() && agent.kind != selected {
                    let kind = agent.kind;
                    let for_call = session_id.to_string();
                    app.tasks
                        .act(session_id, "could not switch agent", move |backend| {
                            backend.set_agent(&for_call, kind)
                        });
                }
            }
        })
        .response
        .on_hover_text("where comments get sent");

    let _ = palette;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CommitReviewStatus, CommitView, LocalChangeSummary, SessionPayload};

    fn payload(active_commit: Option<&str>, commits: Vec<CommitView>) -> SessionPayload {
        SessionPayload {
            repo_name: "repo".to_string(),
            branch_name: Some("main".to_string()),
            commit_base: None,
            commits,
            history_commits: Vec::new(),
            history_has_more: false,
            local_change_summary: LocalChangeSummary::default(),
            active_commit: active_commit.map(ToOwned::to_owned),
            repo_path: "/repo".to_string(),
            read_only: false,
            patch_preview_line_limit: 500,
            available_agents: Vec::new(),
            selected_agent: AgentKind::None,
            full_file_path: None,
            hunks: Vec::new(),
            review_comments: Vec::new(),
            export_text: String::new(),
        }
    }

    fn commit(sha: &str, subject: &str) -> CommitView {
        CommitView {
            sha: sha.to_string(),
            short_sha: sha.chars().take(7).collect(),
            subject: subject.to_string(),
            author: "someone".to_string(),
            review_status: CommitReviewStatus::Unreviewed,
        }
    }

    #[test]
    fn a_working_tree_review_says_local_changes() {
        assert_eq!(review_label(&payload(None, Vec::new())), "local changes");
    }

    #[test]
    fn a_commit_review_names_the_commit() {
        let commits = vec![commit("4542abed00", "Add the thing")];

        assert_eq!(
            review_label(&payload(Some("4542abed00"), commits)),
            "4542abe Add the thing"
        );
    }

    #[test]
    fn a_commit_that_is_not_listed_falls_back_to_its_short_sha() {
        assert_eq!(
            review_label(&payload(Some("deadbeefcafe"), Vec::new())),
            "deadbee"
        );
    }
}
