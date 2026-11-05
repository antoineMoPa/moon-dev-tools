//! The inspect pane: a Minerve run's conversation, read off its control socket rather than its
//! screen - every prompt, reply and tool call, with what each tool call was answered - see
//! [`crate::agent_conversation`].
//!
//! Opened from the `inspect` button on the agent's shell, beside it. The pane asks the server
//! for the conversation on the poll clock for as long as it is drawn.

use std::collections::HashMap;

use egui::{CornerRadius, RichText, Ui};
use egui_frames::DropSide;
use web_time::Instant;

use crate::{
    agent_conversation::{
        AgentConversation, AgentMessage, AgentMessageRole, AgentState, AgentToolCall,
    },
    native::{
        app::App,
        model::Model,
        panes::Pane,
        theme::{CODE_SIZE, Palette, SMALL_SIZE, UI_SIZE},
    },
};

/// The argument a tool call's header shows, by tool: what the call is about at a glance.
/// A tool missing here shows its name alone.
const HEADLINE_ARGUMENT_OF_TOOL: &[(&str, &str)] = &[
    ("run_shell_command", "command"),
    ("read", "path"),
    ("write", "path"),
    ("edit", "path"),
    ("delete_file", "path"),
    ("extract_structure", "path"),
    ("list_files", "dir"),
    ("get_url", "url"),
    ("search_for_string", "search_string"),
    ("search_for_path_pattern", "path_pattern"),
];

/// How many lines of a tool's output are drawn. The rest is counted, not shown: a whole file
/// read in a single label makes the pane crawl.
const SHOWN_OUTPUT_LINES: usize = 200;

/// What the window knows of the conversations its inspect panes show, by terminal.
#[derive(Default)]
pub(crate) struct Conversations {
    by_terminal: HashMap<String, Inspected>,
}

#[cfg(test)]
impl Conversations {
    /// A conversation as if the server had just answered it, for a terminal no server has.
    pub(crate) fn heard(&mut self, terminal_id: &str, conversation: AgentConversation) {
        self.by_terminal.insert(
            terminal_id.to_string(),
            Inspected {
                last_poll: Some(Instant::now()),
                conversation: Some(conversation),
                error: None,
                draft: String::new(),
            },
        );
    }
}

#[derive(Default)]
struct Inspected {
    last_poll: Option<Instant>,
    conversation: Option<AgentConversation>,
    error: Option<String>,
    /// What is being typed into the pane's box, sent to the agent on Enter.
    draft: String,
}

/// Bring the conversation of the agent in `terminal_id` forward: its pane if it has one, or a
/// new one beside the shell.
pub(crate) fn open(model: &mut Model, terminal_id: &str) {
    let open_pane = model
        .layout
        .find_pane(
            |pane| matches!(pane, Pane::Conversation { terminal_id: open } if open == terminal_id),
        )
        .map(|(pane, _)| pane);
    if let Some(pane) = open_pane {
        model.layout.focus_pane(pane);
        return;
    }
    let pane = Pane::Conversation {
        terminal_id: terminal_id.to_string(),
    };
    let terminal_frame = model
        .layout
        .find_pane(
            |pane| matches!(pane, Pane::Terminal { terminal_id: open, .. } if open == terminal_id),
        )
        .and_then(|(pane, _)| model.layout.frame_of(pane));
    match terminal_frame {
        Some(frame) => {
            model.layout.add_pane_beside(frame, DropSide::Right, pane);
        }
        None => {
            model.layout.add_pane_against_edge(
                DropSide::Right,
                egui_frames::DEFAULT_EDGE_SHARE,
                pane,
            );
        }
    }
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, terminal_id: &str) {
    poll(app, terminal_id);
    ui.ctx().request_repaint_after(super::app::POLL_INTERVAL);
    let palette = app.palette_of();
    let inspected = app
        .model
        .conversations
        .by_terminal
        .get_mut(terminal_id)
        .expect("polled just above");
    if let Some(error) = &inspected.error {
        ui.label(RichText::new(error).color(palette.warn));
    }
    let Some(conversation) = &inspected.conversation else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("reading the conversation…").color(palette.muted));
        });
        return;
    };
    let waiting = conversation.state == Some(AgentState::WaitingForInput);
    let mut sent = None;
    egui::Panel::bottom(ui.id().with(("composer", terminal_id)))
        .frame(egui::Frame::new().inner_margin(8))
        .show(ui, |ui| {
            sent = draw_composer(ui, &mut inspected.draft, waiting);
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(8))
        .show(ui, |ui| {
            draw_header(ui, conversation, &palette);
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| draw_messages(ui, &conversation.messages, &palette));
        });
    if let Some(text) = sent {
        submit(app, terminal_id, text);
    }
}

/// The box a prompt for the agent is typed in. Enter sends it, Shift+Enter is a new line.
/// Returns the prompt when one was sent this frame.
fn draw_composer(ui: &mut Ui, draft: &mut String, waiting: bool) -> Option<String> {
    let id = ui.id().with("draft");
    // Taken before the box sees it, which would otherwise put a line break in the draft.
    let enter = ui.memory(|memory| memory.has_focus(id))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let hint = match waiting {
        true => "Message minerve - Enter sends, Shift+Enter for a new line",
        false => "minerve is working…",
    };
    ui.add_enabled(
        waiting,
        egui::TextEdit::multiline(draft)
            .id(id)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    );
    (enter && waiting && !draft.trim().is_empty()).then(|| draft.clone())
}

/// Sends a prompt to the agent. The draft is cleared once the agent has it, and kept with
/// the reason when it was refused.
fn submit(app: &mut App, terminal_id: &str, text: String) {
    let session_id = app.model.root_session_id.clone();
    let asked_for = terminal_id.to_string();
    let answered = terminal_id.to_string();
    let prompt = text.clone();
    app.tasks.spawn_keyed(
        None,
        move |backend| backend.submit_to_agent(&session_id, &asked_for, &prompt),
        move |model, result| {
            let inspected = model.conversations.by_terminal.entry(answered).or_default();
            match result {
                Ok(()) => {
                    if inspected.draft == text {
                        inspected.draft.clear();
                    }
                    inspected.error = None;
                    // Asked again straight away, so the prompt shows without a poll's wait.
                    inspected.last_poll = None;
                }
                Err(error) => inspected.error = Some(format!("could not send the prompt: {error}")),
            }
        },
    );
}

fn poll(app: &mut App, terminal_id: &str) {
    let inspected = app
        .model
        .conversations
        .by_terminal
        .entry(terminal_id.to_string())
        .or_default();
    if inspected
        .last_poll
        .is_some_and(|last| last.elapsed() < super::app::POLL_INTERVAL)
    {
        return;
    }
    inspected.last_poll = Some(Instant::now());
    let session_id = app.model.root_session_id.clone();
    let asked_for = terminal_id.to_string();
    let answered = terminal_id.to_string();
    app.tasks.spawn_keyed(
        Some(format!("conversation-{terminal_id}")),
        move |backend| backend.terminal_conversation(&session_id, &asked_for),
        move |model, result| {
            let inspected = model.conversations.by_terminal.entry(answered).or_default();
            match result {
                Ok(conversation) => {
                    inspected.conversation = Some(conversation);
                    inspected.error = None;
                }
                Err(error) => {
                    inspected.error = Some(format!("could not read the conversation: {error}"))
                }
            }
        },
    );
}

fn draw_header(ui: &mut Ui, conversation: &AgentConversation, palette: &Palette) {
    ui.horizontal(|ui| {
        match conversation.state {
            Some(AgentState::Working) => {
                ui.spinner();
                ui.label(RichText::new("working").color(palette.accent));
            }
            Some(AgentState::WaitingForInput) => {
                ui.label(RichText::new("waiting for input").color(palette.muted));
            }
            None => {
                ui.label(RichText::new("not connected yet").color(palette.muted));
            }
        }
        if let Some(session_id) = &conversation.session_id {
            ui.label(
                RichText::new(format!("session {session_id}"))
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
        }
    });
    if let Some(broken) = &conversation.broken {
        ui.label(RichText::new(format!("stopped reading: {broken}")).color(palette.warn));
    }
}

fn draw_messages(ui: &mut Ui, messages: &[AgentMessage], palette: &Palette) {
    // A tool's output is drawn under the call it answers, so it is looked up by the call's id.
    let outputs: HashMap<&str, &str> = messages
        .iter()
        .filter(|message| message.role == AgentMessageRole::Tool)
        .filter_map(|message| {
            Some((
                message.tool_call_id.as_deref()?,
                message.content.as_deref()?,
            ))
        })
        .collect();
    // One `minerve` heading over a run of replies and tool calls, which is what a turn is.
    let mut previous_role = None;
    for (index, message) in messages.iter().enumerate() {
        let continues_reply = previous_role == Some(AgentMessageRole::Assistant)
            || previous_role == Some(AgentMessageRole::Tool);
        previous_role = Some(message.role);
        ui.push_id(index, |ui| match message.role {
            AgentMessageRole::System => {
                egui::CollapsingHeader::new(RichText::new("system prompt").color(palette.muted))
                    .show(ui, |ui| draw_text(ui, message, palette));
            }
            AgentMessageRole::User => {
                ui.add_space(8.0);
                ui.label(RichText::new("you").strong().color(palette.accent));
                draw_text(ui, message, palette);
            }
            AgentMessageRole::Assistant => {
                if !continues_reply {
                    ui.add_space(8.0);
                    ui.label(RichText::new("minerve").strong().color(palette.accent_2));
                }
                draw_text(ui, message, palette);
                for call in &message.tool_calls {
                    let output = outputs.get(call.id.as_str()).copied();
                    draw_tool_call(ui, call, output, palette);
                }
            }
            // Drawn under its call - see above.
            AgentMessageRole::Tool => {}
        });
    }
}

fn draw_text(ui: &mut Ui, message: &AgentMessage, palette: &Palette) {
    if let Some(content) = message
        .content
        .as_deref()
        .filter(|content| !content.is_empty())
    {
        ui.label(RichText::new(content).size(UI_SIZE).color(palette.ink));
    }
}

fn draw_tool_call(ui: &mut Ui, call: &AgentToolCall, output: Option<&str>, palette: &Palette) {
    // The model wrote these, so they may be anything: what does not parse is shown as written.
    let arguments: Option<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_str(&call.function.arguments).ok();
    let headline = HEADLINE_ARGUMENT_OF_TOOL
        .iter()
        .find(|(tool, _)| *tool == call.function.name)
        .and_then(|(_, argument)| arguments.as_ref()?.get(*argument)?.as_str());
    let mut title = RichText::new(match headline {
        Some(headline) => format!("{}  {}", call.function.name, first_line(headline)),
        None => call.function.name.clone(),
    })
    .monospace()
    .size(CODE_SIZE);
    title = match output {
        Some(_) => title.color(palette.ink),
        None => title.color(palette.muted),
    };
    egui::CollapsingHeader::new(title)
        .id_salt(&call.id)
        .show(ui, |ui| {
            match (call.function.name.as_str(), &arguments) {
                ("edit", Some(arguments)) => draw_edits(ui, arguments, palette),
                ("write", Some(arguments)) => {
                    let content = arguments
                        .get("content")
                        .and_then(|content| content.as_str());
                    draw_code(
                        ui,
                        content.unwrap_or_default(),
                        palette.diff_added_bg,
                        palette,
                    );
                }
                // The headline already says all there is to say about these.
                ("run_shell_command" | "read" | "delete_file" | "list_files", Some(_)) => {}
                (_, Some(arguments)) => draw_code(
                    ui,
                    &serde_json::to_string_pretty(arguments).expect("a JSON map serializes"),
                    palette.code_bg,
                    palette,
                ),
                (_, None) => draw_code(ui, &call.function.arguments, palette.code_bg, palette),
            }
            match output {
                Some(output) => {
                    ui.label(
                        RichText::new("output")
                            .size(SMALL_SIZE)
                            .color(palette.muted),
                    );
                    draw_code(ui, &first_lines(output), palette.code_bg, palette);
                }
                None => {
                    ui.label(
                        RichText::new("no output yet")
                            .size(SMALL_SIZE)
                            .color(palette.muted),
                    );
                }
            }
        });
}

/// Each replacement of an `edit` call: the text it takes out over the text it puts in.
fn draw_edits(
    ui: &mut Ui,
    arguments: &serde_json::Map<String, serde_json::Value>,
    palette: &Palette,
) {
    let Some(edits) = arguments.get("edits").and_then(|edits| edits.as_array()) else {
        return;
    };
    for edit in edits {
        let text_of = |key: &str| {
            edit.get(key)
                .and_then(|text| text.as_str())
                .unwrap_or_default()
        };
        draw_code(ui, text_of("oldText"), palette.diff_removed_bg, palette);
        draw_code(ui, text_of("newText"), palette.diff_added_bg, palette);
        ui.add_space(4.0);
    }
}

fn draw_code(ui: &mut Ui, text: &str, fill: egui::Color32, palette: &Palette) {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .monospace()
                        .size(CODE_SIZE)
                        .color(palette.ink),
                )
                .wrap(),
            );
        });
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// The first [`SHOWN_OUTPUT_LINES`] lines of `text`, and how many more there are.
fn first_lines(text: &str) -> String {
    let total = text.lines().count();
    if total <= SHOWN_OUTPUT_LINES {
        return text.to_string();
    }
    let shown: Vec<&str> = text.lines().take(SHOWN_OUTPUT_LINES).collect();
    format!(
        "{}\n… {} more lines",
        shown.join("\n"),
        total - SHOWN_OUTPUT_LINES
    )
}
