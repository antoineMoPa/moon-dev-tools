//! The menu a card is right-clicked for - or held under a finger for, on a touch screen - see
//! [`super::gesture::Ended::Held`]: what is done *to* a task, rather than started in it.
//!
//! The `[start]` menu is the work a card offers - a review, a shell, an agent. This one is the
//! task folder itself - where it is, and a shell standing in it - and the card's place on the
//! board: sending it to DONE without a drag across the board. They are apart because they
//! answer different questions, and because a right click reaches this one from anywhere on the
//! card while `[start]` is a button at the foot of it.
//!
//! It is drawn from the board rather than from the card, over the whole board, because the
//! card that opened it is redrawn every frame and may be moving under it - the menu stands
//! where the click was made and stays there until it is answered or let go of.

use egui::{Align, Context, Id, Layout};

use crate::{
    api::AgentKind,
    moontasks::{ColumnId, StartFolder, StartResourceRequest, TaskResourceKind, store},
    native::{app::App, board::BoardAction, widgets},
};

/// The card menu, while a right click has one up.
pub(super) fn draw(app: &mut App, ctx: &Context, actions: &mut Vec<BoardAction>) {
    let Some(menu) = &app.model.board.card_menu else {
        return;
    };
    let at = menu.at;
    let Some(task) = app
        .model
        .board
        .tasks
        .iter()
        .find(|task| task.id == menu.task_id)
    else {
        // The card went while its menu was up - deleted here, or in `.moontasks` by hand.
        app.model.board.card_menu = None;
        return;
    };
    let (task_id, dir_path) = (task.id.clone(), task.dir_path.clone());
    // The column the menu can finish the card in, by the name the board shows it under - if
    // the board has it, and the card is not in it already.
    let finished = ColumnId::new(store::MENU_FINISHES_IN);
    let finish_label = app
        .model
        .board
        .columns
        .iter()
        .find(|column| column.id == finished && task.status != finished)
        .map(|column| column.label.clone());

    let mut up = true;
    let mut finish = false;
    let id = Id::new(("moontask-card-menu", &task_id));
    egui::Popup::new(
        id,
        ctx.clone(),
        egui::PopupAnchor::Position(at),
        egui::LayerId::new(egui::Order::Foreground, id),
    )
    .kind(egui::PopupKind::Menu)
    .layout(Layout::top_down_justified(Align::Min))
    .style(egui::containers::menu::menu_style)
    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
    .open_bool(&mut up)
    .show(|ui| {
        items(ui, &task_id, &dir_path, actions);
        if let Some(label) = &finish_label
            && widgets::clickable(ui.button(format!("move to {label}")))
                .on_hover_text("Send the card to the column finished tasks go to")
                .clicked()
        {
            finish = true;
            ui.close();
        }
    });

    if finish {
        super::column::arrive_in_column(app, &finished, std::slice::from_ref(&task_id), actions);
    }
    if !up {
        app.model.board.card_menu = None;
    }
}

/// What the menu offers: the task's folder, said and stood in.
fn items(ui: &mut egui::Ui, task_id: &str, dir_path: &str, actions: &mut Vec<BoardAction>) {
    if widgets::clickable(ui.button("copy task path"))
        .on_hover_text(format!("Copy {dir_path} to the clipboard"))
        .clicked()
    {
        ui.ctx().copy_text(dir_path.to_string());
        ui.close();
    }

    if widgets::clickable(ui.button("open shell in the task folder"))
        .on_hover_text("Open a shell in the task's own folder, where its notes and brief are")
        .clicked()
    {
        actions.push(BoardAction::Start(
            task_id.to_string(),
            StartResourceRequest {
                kind: TaskResourceKind::Shell,
                agent: AgentKind::None,
                opens_in: StartFolder::TaskFolder,
            },
        ));
        ui.close();
    }
}
