//! What a card carries besides its title: the tags it is marked with, and the checkout it
//! works in.

use egui::{Ui, vec2};

use crate::{
    moontasks::{AUTOPILOT_TAG, TaskView},
    native::{
        app::App,
        board::BoardAction,
        theme::Palette,
        widgets::{self},
    },
};

/// Every tag anything on the board is marked with, in the order they read best: the ones this
/// card already has first, then the rest of the board's, so the menu opens on what is nearest.
fn tags_to_offer(app: &App, task: &TaskView) -> Vec<String> {
    let mut offered = task.tags.clone();
    let in_use = app
        .model
        .board
        .tasks
        .iter()
        .flat_map(|other| other.tags.iter().cloned())
        .chain(std::iter::once(AUTOPILOT_TAG.to_string()));
    for tag in in_use {
        if !offered.contains(&tag) {
            offered.push(tag);
        }
    }
    offered
}

/// The pills under a card's title: what it is marked with, and anything about its checkout
/// that stands in the way.
///
/// The branch a card works on is not a pill. Every card with a checkout has one, it is always
/// the same as the card's id, and a row of them says nothing that differs between two cards —
/// so it is in the `[worktree]` menu, where it is read when it is wanted.
pub(super) fn draw_marks(ui: &mut Ui, task: &TaskView, palette: &Palette) {
    let uncommitted = task
        .worktree
        .as_ref()
        .is_some_and(|worktree| !worktree.is_clean);
    if task.tags.is_empty() && !uncommitted {
        return;
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
        for tag in &task.tags {
            widgets::pill(ui, tag, palette.ink, palette.status_neutral_bg);
        }
        // Only committed work can be checked out in the repo, so a card that cannot be
        // reviewed yet says why before the button is pressed.
        if uncommitted {
            widgets::pill(ui, "uncommitted", palette.warn, palette.status_failed_bg).on_hover_text(
                "Commit in the worktree before reviewing — the review checks this branch \
                     out in the repo",
            );
        }
    });
    ui.add_space(3.0);
}

/// The `[tags]` menu: every tag on the board, ticked where this card has it, and a box to
/// write a new one in.
pub(super) fn draw_tag_menu(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    actions: &mut Vec<BoardAction>,
) {
    let offered = tags_to_offer(app, task);
    let composing = app
        .model
        .board
        .tagging
        .as_ref()
        .is_some_and(|composer| composer.task_id == task.id);

    let (button, _) =
        egui::containers::menu::MenuButton::from_button(egui::Button::new("[tags]").frame(false))
            .ui(ui, |ui| {
                for tag in offered {
                    let held = task.tags.contains(&tag);
                    let label = if held {
                        format!("✓ {tag}")
                    } else {
                        format!("   {tag}")
                    };
                    if widgets::clickable(ui.button(label)).clicked() {
                        // The whole list every time: the menu knows what the card should end up
                        // marked with, and sending that is one write rather than a diff.
                        let mut tags = task.tags.clone();
                        if held {
                            tags.retain(|held| *held != tag);
                        } else {
                            tags.push(tag);
                        }
                        actions.push(BoardAction::SetTags(task.id.clone(), tags));
                        ui.close();
                    }
                }

                ui.separator();
                if composing {
                    draw_tag_composer(app, ui, task, actions);
                } else if widgets::clickable(ui.button("new tag…")).clicked() {
                    actions.push(BoardAction::OpenTagComposer(task.id.clone()));
                }
            });
    widgets::clickable(button)
        .on_hover_text("What this card is marked with — see the autopilot window");
}

/// The box a new tag is typed into, standing in the menu where the tag will appear.
fn draw_tag_composer(app: &mut App, ui: &mut Ui, task: &TaskView, actions: &mut Vec<BoardAction>) {
    let Some(composer) = app.model.board.tagging.as_mut() else {
        return;
    };
    let response = ui.add(
        egui::TextEdit::singleline(&mut composer.text)
            .hint_text("tag")
            .desired_width(120.0),
    );
    if std::mem::take(&mut composer.focus) {
        response.request_focus();
    }

    if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
        let typed = composer.text.clone();
        let mut tags = task.tags.clone();
        tags.push(typed);
        actions.push(BoardAction::SetTags(task.id.clone(), tags));
        actions.push(BoardAction::CloseTagComposer);
        ui.close();
    } else if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        actions.push(BoardAction::CloseTagComposer);
    }
}

/// The worktree half of the actions row: make one, or give the one there is back.
pub(super) fn draw_worktree_actions(
    app: &App,
    ui: &mut Ui,
    task: &TaskView,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let Some(worktree) = &task.worktree else {
        if widgets::quiet_button(ui, "[worktree]")
            .on_hover_text(
                "Give this task a checkout of its own, so its agents work off your own tree",
            )
            .clicked()
        {
            actions.push(BoardAction::CreateWorktree(task.id.clone()));
        }
        return;
    };

    // The directory goes, and whatever was never committed in it goes with it, so the press
    // asks first — the same two-press shape deleting a card has.
    if app.model.board.pending_worktree_discard.as_deref() == Some(task.id.as_str()) {
        match widgets::confirm(
            ui,
            palette,
            "[really discard]",
            "Also throws away anything uncommitted in the worktree",
        ) {
            widgets::Confirmed::Yes => {
                actions.push(BoardAction::DiscardWorktree(task.id.clone(), true))
            }
            widgets::Confirmed::No => actions.push(BoardAction::CancelWorktreeDiscard),
            widgets::Confirmed::Waiting => {}
        }
        return;
    }

    if widgets::quiet_button(ui, "[discard worktree]")
        .on_hover_text(format!(
            "Remove {} — {} stays",
            worktree.path, worktree.branch
        ))
        .clicked()
    {
        actions.push(BoardAction::ArmWorktreeDiscard(task.id.clone()));
    }
}
