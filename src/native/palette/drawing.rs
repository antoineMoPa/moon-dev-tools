//! Drawing the palette: the query box over the list, the switch between its two searches
//! and the scope they search. The box and its rows are `egui_command_palette`'s; this is
//! what moon puts in it.

use egui::{Color32, Key, RichText, vec2};
#[cfg(not(target_arch = "wasm32"))]
use egui_command_palette::Choice;
use egui_command_palette::{LineAccessory, Row, Section, SectionHeader, View};

pub(in crate::native) use egui_command_palette::{Kept, cut_to_width, pressed_outside};

use crate::{
    api::SearchScope,
    native::{
        app::App,
        bindings::{self},
        theme::{Palette, SMALL_SIZE, UI_SIZE},
    },
};

use super::rows::{
    empty_message, footnote, hint_of, refresh_content_matches, refresh_file_matches,
};
use super::sections::{
    Listed, SectionKind, command_sections, keep_shown_matches, opened_by_header,
    refresh_section_searches, remember_highlight, restore_highlight, section_to_open,
};
use super::{Command, PaletteMode, history, rows_for};

/// The gap between the query line's edge and what is typed in it.
const LINE_PADDING: i8 = 7;

/// The two searches as the switch at the end of the line words them, in the order they sit
/// there from the left: what the finder reads by, and what the content search reads.
const SEARCH_SWITCH: [(PaletteMode, &str); 2] = [
    (PaletteMode::Files, "files"),
    (PaletteMode::Contents, "content"),
];

/// Which end of a row's second line is kept. A sentence about a command is read from its
/// start. A path is read from its end - the file, and the line in it - and its first folders
/// are what a row can do without.
fn description_kept(mode: PaletteMode) -> Kept {
    match mode {
        PaletteMode::Commands | PaletteMode::Rename | PaletteMode::CodeActions => Kept::Start,
        PaletteMode::Files | PaletteMode::Contents | PaletteMode::Places => Kept::End,
    }
}

/// The palette's colours and row sizes, out of the window's.
fn style_of(palette: &Palette) -> egui_command_palette::PaletteStyle {
    egui_command_palette::PaletteStyle {
        panel: palette.panel,
        line: palette.line,
        selected: palette.control_active_bg,
        accent: palette.accent,
        text: palette.ink,
        muted: palette.muted,
        shadow: Color32::from_black_alpha(60),
        row_height: 42.0,
        title_size: UI_SIZE,
        detail_size: SMALL_SIZE - 1.0,
        title_top: 8.0,
        detail_top: 24.0,
        header_height: 32.0,
    }
}

/// The row of the list a command is.
pub(super) fn row_of(command: &Command) -> Row {
    Row {
        title: command.title.clone(),
        description: command.description.clone(),
        shortcut: command.shortcut.map(bindings::describe).unwrap_or_default(),
        ..Row::default()
    }
}

/// The colour each agent is written in on a quick task's row, so the row says which agent it
/// starts before it is read.
#[cfg(not(target_arch = "wasm32"))]
const AGENT_INKS: &[(crate::api::AgentKind, Color32)] = &[
    (crate::api::AgentKind::Claude, Color32::from_rgb(0xd9, 0x77, 0x57)),
    (crate::api::AgentKind::Codex, Color32::from_rgb(0x7f, 0xc8, 0xf8)),
    (crate::api::AgentKind::OpenCode, Color32::from_rgb(0x5b, 0x8d, 0xef)),
    (crate::api::AgentKind::Pi, Color32::from_rgb(0xea, 0xb6, 0x5d)),
];

/// An agent's colour.
#[cfg(not(target_arch = "wasm32"))]
fn ink_of(agent: crate::api::AgentKind) -> Color32 {
    AGENT_INKS
        .iter()
        .find(|(of, _)| *of == agent)
        .map(|(_, ink)| *ink)
        .expect("every agent a quick task is offered for has an ink")
}

/// The row of a quick task made out of what was typed - see
/// [`crate::native::board::quick_task`]: the agent it starts with named in its colour, so it
/// reads as something other than the window's commands, and a chip per agent at its end.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn quick_task_row(
    task: &crate::native::board::quick_task::QuickTask,
    command: &Command,
) -> Row {
    Row {
        inked: vec![(task.agent_in_title(&command.title), ink_of(task.agent))],
        choices: task
            .agents
            .iter()
            .map(|agent| Choice {
                label: crate::native::board::agent_label(*agent),
                ink: ink_of(*agent),
            })
            .collect(),
        picked: task
            .agents
            .iter()
            .position(|agent| *agent == task.agent)
            .expect("the agent a quick task starts with is one of its agents"),
        ..row_of(command)
    }
}

/// A quick task's row in the palette's history - see [`super::history`]: the agent's dot, the
/// title, and the agent named in its colour.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn quick_task_entry_row(title: &str, agent: crate::api::AgentKind) -> Row {
    let label = crate::native::board::agent_label(agent);
    let title = format!("\"{title}\" · {label}");
    Row {
        dot: Some(ink_of(agent)),
        inked: vec![(title.len() - label.len()..title.len(), ink_of(agent))],
        title,
        ..Row::default()
    }
}

/// What the line carries besides what is typed: the switch between the two searches at its
/// end, when it has one, and under it the box for what files they read.
struct SearchLine<'a> {
    mode: PaletteMode,
    palette: &'a Palette,
    switch: bool,
    scope: &'a mut SearchScope,
    /// The search the switch was clicked over to.
    switched_to: Option<PaletteMode>,
}

impl LineAccessory for SearchLine<'_> {
    fn room_at_end(&self, ui: &egui::Ui) -> f32 {
        if self.switch {
            search_switch_width(ui, self.palette)
        } else {
            0.0
        }
    }

    fn show(&mut self, ui: &mut egui::Ui, line: egui::Rect) {
        if self.switch {
            self.switched_to = draw_search_switch(ui, line, self.mode, self.palette);
        }
        if matches!(self.mode, PaletteMode::Files | PaletteMode::Contents) {
            draw_scope_box(ui, self.scope, self.palette);
        }
    }
}

pub(crate) fn draw(app: &mut App, ctx: &egui::Context) {
    if !app.model.palette.is_open() {
        // Put away by its own box - a press elsewhere - which knows nothing of the line of
        // shell under it.
        #[cfg(not(target_arch = "wasm32"))]
        {
            app.model.palette.shell = None;
        }
        return;
    }
    history::take_escape(app, ctx);
    // A press anywhere else - a shell, a tab, a pane in the next frame over - puts the palette
    // away and belongs to whatever was pressed.
    if app.model.palette.input.dismissal_requested(ctx) {
        app.model.palette.dismiss();
        return;
    }
    // Tab flips the switch at the end of the line, when there is one. Taken out of the
    // input, so the text box does not hand the keyboard on - it would only ask for it back.
    if has_search_switch(app)
        && ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, Key::Tab)
                | input.consume_key(egui::Modifiers::SHIFT, Key::Tab)
        })
    {
        app.model
            .palette
            .switch_search(other_search(app.model.palette.mode));
    }
    // Tab on a row of the files or the lines the command list found opens the whole list
    // of them, on the same query.
    if app.model.palette.mode == PaletteMode::Commands
        && let Some(to) = section_to_open(app)
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Tab))
    {
        app.model.palette.open_section(to);
    }
    // Only the command list takes a line of shell: the searches look for what is typed.
    #[cfg(not(target_arch = "wasm32"))]
    let shell_line = super::shell::typed_line(app)
        .filter(|_| app.model.palette.mode == PaletteMode::Commands)
        .map(ToOwned::to_owned);
    #[cfg(target_arch = "wasm32")]
    let shell_line: Option<String> = None;
    #[cfg(not(target_arch = "wasm32"))]
    super::shell::follow_query(app);
    match app.model.palette.mode {
        PaletteMode::Rename | PaletteMode::Places | PaletteMode::CodeActions => {}
        // A line of shell is not looked for in the repo.
        PaletteMode::Commands if shell_line.is_some() => app.model.palette.stop_searches(),
        PaletteMode::Commands => {
            refresh_section_searches(app, ctx);
            keep_shown_matches(app);
        }
        PaletteMode::Files => refresh_file_matches(app),
        PaletteMode::Contents => refresh_content_matches(app),
    }
    let palette = app.palette_of();
    *app.model.palette.input.style_mut() = style_of(&palette);
    let listed = match app.model.palette.mode {
        PaletteMode::Commands => command_sections(app),
        _ => vec![Listed::bare(rows_for(app))],
    };
    let row_count: usize = listed.iter().map(|part| part.commands.len()).sum();
    let hint = hint_of(app);
    let empty = empty_message(app);
    let note = footnote(app, row_count);
    let sections = sections_of(&listed, description_kept(app.model.palette.mode));
    restore_highlight(app, &listed);
    let mut entries = history::shown_entries(app);
    let on_first_row = app.model.palette.input.highlighted() == 0;
    let entry_run = history::take_keys(app, ctx, &entries, on_first_row);
    // ⌘Enter on a line of shell opens it in a shell of its own.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(line) = &shell_line
        && entry_run.is_none()
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, Key::Enter))
    {
        super::shell::open_in_terminal(app, line);
        return;
    }
    let cursor = &app.model.palette.history.cursor;
    let view = View {
        hint: &hint,
        empty: &empty,
        footnote: note.as_deref(),
        label: cursor.searching().then_some(history::SEARCH_LABEL),
        highlight_shown: !cursor.active(),
    };
    let mut line = SearchLine {
        mode: app.model.palette.mode,
        palette: &palette,
        switch: has_search_switch(app),
        scope: &mut app.model.palette.search_scope,
        switched_to: None,
    };

    let shown = app
        .model
        .palette
        .input
        .show_sections(ctx, &sections, &view, &mut line);
    let switched_to = line.switched_to;
    remember_highlight(app, &listed);
    let entry_clicked = history::draw(app, ctx, &entries);
    #[cfg(not(target_arch = "wasm32"))]
    super::shell::draw_output(app, ctx);

    if let Some(index) = entry_run.or(entry_clicked) {
        history::run(app, ctx, entries.swap_remove(index));
        return;
    }

    if let Some(to) = switched_to {
        app.model.palette.switch_search(to);
    }
    if let Some(index) = shown.more_clicked {
        app.model
            .palette
            .open_section(opened_by_header(&listed, index));
        return;
    }
    // A name typed from `/` or `~` is a path of the disk, which the file picker browses and
    // completes - the finder only searches the repo. A file for a task's card stays a file
    // of the repo.
    #[cfg(not(target_arch = "wasm32"))]
    if app.model.palette.mode == PaletteMode::Files
        && app.model.palette.files_link_to_task.is_none()
        && crate::native::file_picker::names_the_disk(app.model.palette.query())
    {
        let typed = app.model.palette.query().to_string();
        app.pick_file_to_edit_from(&typed);
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some((row, choice)) = shown.choice_clicked {
        app.model.palette.dismiss();
        app.pending_action = Some(super::sections::task_started_from_chip(
            app, &listed, row, choice,
        ));
        return;
    }
    // Enter on a line of shell runs it under the palette, which its row keeps open.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(line) = &shell_line
        && shown.chosen.is_some()
    {
        super::shell::run(app, ctx, line);
        return;
    }
    if let Some(index) = shown.chosen
        && let Some(command) = listed.into_iter().flat_map(|part| part.commands).nth(index)
    {
        if let Some(thing) = history::thing_of(app, app.model.palette.mode, &command) {
            app.record_in_history(thing);
        }
        app.model.palette.dismiss();
        app.pending_action = Some(command.action);
    }
}

/// The parts of the list as the palette draws them. The searches' sections keep the end of
/// a path, whatever the mode keeps of a row.
fn sections_of(listed: &[Listed], mode_kept: Kept) -> Vec<Section<'_>> {
    listed
        .iter()
        .map(|part| Section {
            header: part.header.as_ref().map(|header| SectionHeader {
                label: header.label,
                count: &header.count,
                busy: header.busy,
                more: header.more.as_deref(),
            }),
            rows: &part.rows,
            description_kept: match part.kind {
                Some(SectionKind::Files | SectionKind::Contents) => Kept::End,
                Some(SectionKind::Commands | SectionKind::Task) | None => mode_kept,
            },
            placeholders: part.placeholders,
        })
        .collect()
}

/// The box the palette is drawn in, floating over the window. The file picker is the same
/// kind of box - see [`crate::native::file_picker`].
pub(in crate::native) fn floating_frame(palette: &Palette) -> egui::Frame {
    egui_command_palette::frame(&style_of(palette))
}

/// How wide that box is.
pub(in crate::native) fn floating_width(screen: egui::Rect) -> f32 {
    egui_command_palette::frame_width(screen)
}

/// The ground and outline of the row the keyboard is on.
pub(in crate::native) fn paint_highlight(ui: &egui::Ui, rect: egui::Rect, palette: &Palette) {
    egui_command_palette::paint_highlight(ui, rect, &style_of(palette));
}

/// The box under the two searches' line that widens them to the files the repo's
/// `.gitignore` leaves out - the submodules of a repo that ignores them, say. The line keeps
/// the keyboard; this is for the pointer.
fn draw_scope_box(ui: &mut egui::Ui, scope: &mut SearchScope, palette: &Palette) {
    ui.add_space(3.0);
    let mut included = scope.includes_ignored();
    let label = RichText::new("include gitignored")
        .size(SMALL_SIZE - 1.0)
        .color(palette.muted);
    if ui.checkbox(&mut included, label).changed() {
        *scope = SearchScope::including_ignored(included);
    }
}

/// The search the switch is not on: the one Tab switches to.
fn other_search(mode: PaletteMode) -> PaletteMode {
    match mode {
        PaletteMode::Files => PaletteMode::Contents,
        PaletteMode::Contents => PaletteMode::Files,
        PaletteMode::Commands
        | PaletteMode::Rename
        | PaletteMode::Places
        | PaletteMode::CodeActions => {
            unreachable!("only the two searches have the switch")
        }
    }
}

/// Whether the line ends in the switch between the two searches: it does on either of them,
/// except when the finder is picking a file for a task's card, which a line of text is not.
fn has_search_switch(app: &App) -> bool {
    matches!(
        app.model.palette.mode,
        PaletteMode::Files | PaletteMode::Contents
    ) && app.model.palette.files_link_to_task.is_none()
}

/// The switch's lettering: a size under the small one, so the switch sits inside the line
/// rather than filling it.
const SWITCH_FONT_SIZE: f32 = SMALL_SIZE - 2.0;

/// The switch between the two searches, on `mode`, in the palette's colours: the track a
/// tinted ground and the thumb the panel's own, told apart on either theme.
fn search_switch(
    mode: PaletteMode,
    palette: &Palette,
) -> moon_toggle_switch::ToggleSwitch<'static, PaletteMode> {
    moon_toggle_switch::ToggleSwitch::new(&SEARCH_SWITCH, mode).style(
        moon_toggle_switch::ToggleSwitchStyle {
            track: palette.batch_bg,
            thumb: palette.panel,
            ink: palette.ink,
            muted: palette.muted,
            font: egui::FontId::proportional(SWITCH_FONT_SIZE),
            padding: vec2(4.0, 2.0),
        },
    )
}

/// How wide the switch is, for the line to leave it room.
fn search_switch_width(ui: &egui::Ui, palette: &Palette) -> f32 {
    search_switch(PaletteMode::Files, palette).size(ui).x
}

/// The switch at the end of the line, so a name typed into the finder can be looked for in
/// the files' text instead without typing it again - Tab flips it too. Drawn over the end
/// of the line, in the room the line's margin left it. The search clicked, if it is the
/// other one.
fn draw_search_switch(
    ui: &mut egui::Ui,
    line: egui::Rect,
    mode: PaletteMode,
    palette: &Palette,
) -> Option<PaletteMode> {
    let width = search_switch_width(ui, palette);
    let room = egui::Rect::from_min_max(
        egui::pos2(line.max.x - f32::from(LINE_PADDING) - width, line.min.y),
        line.max,
    );
    search_switch(mode, palette).show_in(ui, room)
}
