//! Drawing the palette: the query box over the list, the switch between its two searches
//! and the scope they search. The box and its rows are `egui_command_palette`'s; this is
//! what moon puts in it.

use egui::{Color32, Key, RichText, vec2};
use egui_command_palette::{LineAccessory, Row, View};

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
use super::{Command, CommandAction, PaletteMode, rows_for};

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
        row_height: 34.0,
        title_size: UI_SIZE,
        detail_size: SMALL_SIZE - 1.0,
        detail_top: 19.0,
    }
}

/// The row of the list a command is.
fn row_of(command: &Command) -> Row {
    Row {
        title: command.title.clone(),
        description: command.description.clone(),
        shortcut: command.shortcut.map(bindings::describe).unwrap_or_default(),
        title_color: title_color_of(&command.action),
    }
}

/// The colour each agent's quick task is written in, so the row says which agent it starts
/// before it is read.
#[cfg(not(target_arch = "wasm32"))]
const AGENT_INKS: &[(crate::api::AgentKind, Color32)] = &[
    (crate::api::AgentKind::Claude, Color32::from_rgb(0xd9, 0x77, 0x57)),
    (crate::api::AgentKind::Codex, Color32::from_rgb(0x7f, 0xc8, 0xf8)),
    (crate::api::AgentKind::OpenCode, Color32::from_rgb(0x5b, 0x8d, 0xef)),
    (crate::api::AgentKind::Pi, Color32::from_rgb(0xea, 0xb6, 0x5d)),
];

/// The colour a command's title is set apart in: a quick task made out of what was typed is
/// written in its agent's colour - see [`crate::native::board::quick_task`] - so it reads as
/// something other than the window's commands. `None` is the ordinary ink.
fn title_color_of(action: &CommandAction) -> Option<Color32> {
    match action {
        #[cfg(not(target_arch = "wasm32"))]
        CommandAction::StartQuickTask { agent, .. } => Some(
            AGENT_INKS
                .iter()
                .find(|(of, _)| of == agent)
                .map(|(_, ink)| *ink)
                .expect("every agent a quick task is offered for has an ink"),
        ),
        _ => None,
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
        return;
    }
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
    match app.model.palette.mode {
        PaletteMode::Commands
        | PaletteMode::Rename
        | PaletteMode::Places
        | PaletteMode::CodeActions => {}
        PaletteMode::Files => refresh_file_matches(app),
        PaletteMode::Contents => refresh_content_matches(app),
    }
    let palette = app.palette_of();
    *app.model.palette.input.style_mut() = style_of(&palette);
    let matches = rows_for(app);
    let rows: Vec<Row> = matches.iter().map(row_of).collect();
    let hint = hint_of(app);
    let empty = empty_message(app);
    let note = footnote(app, matches.len());
    let view = View {
        hint: &hint,
        empty: &empty,
        footnote: note.as_deref(),
        description_kept: description_kept(app.model.palette.mode),
    };
    let mut line = SearchLine {
        mode: app.model.palette.mode,
        palette: &palette,
        switch: has_search_switch(app),
        scope: &mut app.model.palette.search_scope,
        switched_to: None,
    };

    let chosen = app
        .model
        .palette
        .input
        .show_rows(ctx, &rows, &view, &mut line);
    let switched_to = line.switched_to;

    if let Some(to) = switched_to {
        app.model.palette.switch_search(to);
    }
    if let Some(index) = chosen
        && let Some(command) = matches.into_iter().nth(index)
    {
        app.model.palette.dismiss();
        app.pending_action = Some(command.action);
    }
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
