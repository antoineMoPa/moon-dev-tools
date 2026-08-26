//! The attach-a-session modal: the sessions the agents themselves have for this repo,
//! offered to a task whose own record of one stopped pointing anywhere.

use egui::{Align, CornerRadius, Layout as UiLayout, RichText, Stroke, Ui};

use crate::{
    agent_sessions::AgentSessionView,
    api::AgentKind,
    native::{
        app::App,
        board::{BoardAction, agent_label, available_agents},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How wide the modal is. Room for a sentence of a title, a pill and a date on one row.
const PICKER_WIDTH: f32 = 460.0;

/// How tall the list may grow before it scrolls instead.
const LIST_HEIGHT: f32 = 320.0;

pub(super) fn draw(
    app: &mut App,
    ctx: &egui::Context,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let Some(picker) = &app.model.board.attach_picker else {
        return;
    };
    let task_id = picker.task_id.clone();
    let task_title = picker.task_title.clone();
    let error = picker.error.clone();
    let sessions = picker.sessions.clone();
    let agents: Vec<AgentKind> = available_agents(app)
        .into_iter()
        .filter(|agent| *agent != AgentKind::None)
        .collect();

    let modal = egui::Modal::new(egui::Id::new("moontasks-attach-picker")).show(ctx, |ui| {
        ui.set_width(PICKER_WIDTH);

        ui.label(RichText::new("attach a session").strong());
        ui.label(
            RichText::new(format!("onto \"{task_title}\""))
                .size(SMALL_SIZE)
                .color(palette.muted),
        );
        ui.add_space(6.0);
        widgets::divider(ui, palette);
        ui.add_space(6.0);

        if let Some(error) = &error {
            ui.label(RichText::new(error).color(palette.warn));
        } else {
            match &sessions {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("reading the agents' sessions…").color(palette.muted),
                        );
                    });
                }
                Some(sessions) if sessions.is_empty() => {
                    ui.label(
                        RichText::new("the agents have no past sessions for this repo")
                            .color(palette.muted),
                    );
                }
                Some(sessions) => {
                    egui::ScrollArea::vertical()
                        .id_salt("moontasks-attach-sessions")
                        .max_height(LIST_HEIGHT)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for session in sessions {
                                draw_row(ui, &task_id, session, palette, actions);
                                ui.add_space(4.0);
                            }
                        });
                }
            }
        }

        if !agents.is_empty() {
            draw_manual_entry(app, ui, &task_id, &agents, palette, actions);
        }

        ui.add_space(6.0);
        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            if widgets::quiet_button(ui, "cancel").clicked() {
                actions.push(BoardAction::CloseAttachPicker);
            }
        });
    });

    // Escape, or a click on the backdrop.
    if modal.should_close() {
        actions.push(BoardAction::CloseAttachPicker);
    }
}

/// A session id typed or pasted by hand, with the agent it belongs to: the way in for a
/// session the listing does not show - too old to make the newest few, or one nobody ever
/// spoke in. The id is passed through as given; an id the agent does not know is refused by
/// the agent itself, in the shell that opens.
fn draw_manual_entry(
    app: &mut App,
    ui: &mut Ui,
    task_id: &str,
    agents: &[AgentKind],
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let Some(picker) = app.model.board.attach_picker.as_mut() else {
        return;
    };

    ui.add_space(6.0);
    widgets::divider(ui, palette);
    ui.add_space(6.0);
    ui.label(
        RichText::new("or attach a session by its id")
            .size(SMALL_SIZE)
            .color(palette.muted),
    );
    ui.add_space(4.0);

    let entry = ui.add(
        egui::TextEdit::singleline(&mut picker.manual_id)
            .hint_text("session id")
            .desired_width(f32::INFINITY),
    );
    let submitted = entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    ui.add_space(4.0);

    // Claude first: it is the agent whose ids are pasted in practice, its listing being the
    // one that drops sessions nobody spoke in.
    let agent = picker
        .manual_agent
        .filter(|picked| agents.contains(picked))
        .or_else(|| agents.contains(&AgentKind::Claude).then_some(AgentKind::Claude))
        .unwrap_or(agents[0]);
    let ready = !picker.manual_id.trim().is_empty();
    let mut attach = false;

    ui.horizontal(|ui| {
        let mut picked = agent;
        egui::ComboBox::from_id_salt("moontasks-attach-manual-agent")
            .selected_text(agent_label(agent))
            .width(104.0)
            .show_ui(ui, |ui| {
                for option in agents {
                    ui.selectable_value(&mut picked, *option, agent_label(*option));
                }
            });
        if picked != agent {
            picker.manual_agent = Some(picked);
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            if widgets::clickable(ui.add_enabled(ready, egui::Button::new("attach")))
                .on_hover_text("Put this session on the task and open it")
                .clicked()
            {
                attach = true;
            }
        });
    });

    if (submitted || attach) && ready {
        actions.push(BoardAction::Attach {
            task_id: task_id.to_string(),
            agent,
            agent_session_id: picker.manual_id.trim().to_string(),
        });
    }
}

/// One session: which agent it belongs to, what it is about, and when it was last written to.
/// The whole row is the button that attaches it.
fn draw_row(
    ui: &mut Ui,
    task_id: &str,
    session: &AgentSessionView,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let laid_out = egui::Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.line))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(agent_label(session.agent))
                        .size(SMALL_SIZE)
                        .color(palette.accent_2),
                );
                ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(utc_date_label(session.updated_at_unix))
                            .size(SMALL_SIZE)
                            .color(palette.muted),
                    );
                });
            });
            let width = ui.available_width();
            let title = widgets::cut_to_fit(
                ui,
                &session.title,
                egui::TextStyle::Body.resolve(ui.style()),
                palette.ink,
                width,
                1,
            );
            ui.add(egui::Label::new(title).selectable(false));
        })
        .response;

    let row = widgets::clickable(ui.interact(
        laid_out.rect,
        ui.id().with(("attach-session", &session.id)),
        egui::Sense::click(),
    ))
    .on_hover_text(format!("{}\n\nsession {}", session.title, session.id));

    if row.hovered() {
        ui.painter().rect_stroke(
            laid_out.rect,
            CornerRadius::same(5),
            Stroke::new(1.0, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
    if row.clicked() {
        actions.push(BoardAction::Attach {
            task_id: task_id.to_string(),
            agent: session.agent,
            agent_session_id: session.id.clone(),
        });
    }
}

/// The day a session was last written to, as `2026-08-09`.
///
/// A date rather than a relative age: the list is sorted newest first, so recency is
/// already readable, and a date does not change from one frame to the next.
fn utc_date_label(unix: u64) -> String {
    // Civil-from-days, Howard Hinnant's algorithm: days since the epoch to y-m-d.
    let days = (unix / 86_400) as i64;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_s_day_reads_as_a_date() {
        assert_eq!(utc_date_label(0), "1970-01-01");
        // 2026-08-09 14:24:47 UTC.
        assert_eq!(utc_date_label(1_786_388_687), "2026-08-10");
        // Leap day.
        assert_eq!(utc_date_label(1_709_164_800), "2024-02-29");
    }
}
