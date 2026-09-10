//! A pane an extension draws - see [`crate::extensions`].
//!
//! The script runs on a thread of its own. This is the window's side of it: starting it when
//! its pane appears, drawing the last view it sent, handing it the clicks and keys the pane
//! gets, and doing what it asks of the window.

use std::collections::HashMap;

use egui::{Align, Color32, Key, Layout, Margin, RichText, Sense, TextStyle, Ui};
use egui_extras::{Column, TableBuilder};
use egui_frames::PaneId;
use serde_json::Value;

use crate::{
    extensions::{self, Effect, Element, Ink, Input, Output, Running, TableRow, host::Host},
    native::{
        app::App,
        model::ToastKind,
        panes::{OpenAt, OpenPaneRequest, Pane},
        theme::{Palette, SMALL_SIZE, UI_SIZE},
        widgets,
    },
};

/// The keys that type nothing, which a script hears by name - `Enter`, `Up`. Every other key
/// reaches it as the character it typed, so `^` is `^` whatever the keyboard.
const NAMED_KEYS: &[Key] = &[
    Key::Enter,
    Key::Escape,
    Key::Backspace,
    Key::Delete,
    Key::Tab,
    Key::ArrowUp,
    Key::ArrowDown,
    Key::ArrowLeft,
    Key::ArrowRight,
    Key::Home,
    Key::End,
    Key::PageUp,
    Key::PageDown,
];

/// The space between two columns of a table: enough that a size and a date beside it read as
/// two things.
const COLUMN_GAP: f32 = 14.0;

/// How narrow a table's column may be, for a column with nothing in it yet - a mark nothing is
/// marked with - which would otherwise take no room and leave its heading nowhere.
const MIN_COLUMN_WIDTH: f32 = 14.0;

/// How wide an input is drawn: room for a filter's few words, in a pane that is often a narrow
/// column.
const INPUT_WIDTH: f32 = 280.0;

/// One extension pane: its script, and what it last said.
pub(crate) struct ExtensionPane {
    /// `None` for one that could not be started, which `error` says why.
    running: Option<Running>,
    view: Option<Element>,
    error: Option<String>,
    /// The selected row the table was last scrolled to. The table is scrolled when the
    /// selection moves, and left where the wheel put it otherwise.
    scrolled_to: Option<usize>,
    /// Whether the script has an `on_key`. One without leaves every key to the window.
    takes_keys: bool,
    /// Whether the pane is in front in its frame, as the script was last told.
    visible: bool,
    /// What is typed in each of the view's inputs, by the input's id. Kept here rather than
    /// read back from the view, so a keystroke is on screen at once, not a round trip later.
    inputs: HashMap<String, String>,
}

impl ExtensionPane {
    fn failed(error: String) -> Self {
        Self {
            running: None,
            view: None,
            error: Some(error),
            scrolled_to: None,
            takes_keys: false,
            visible: true,
            inputs: HashMap::new(),
        }
    }
}

impl App {
    /// Start the script of an extension pane that has none - one just opened, or put back by
    /// a restored layout - take in what each script has said, and do what they asked.
    ///
    /// Before the workspace is drawn, so what an effect opens is not opened into the tree
    /// being drawn.
    pub(crate) fn follow_extensions(&mut self, ctx: &egui::Context) {
        let open: Vec<(PaneId, String)> = self
            .model
            .layout
            .panes()
            .filter_map(|(pane_id, pane)| match pane {
                Pane::Extension { name } => Some((pane_id, name.clone())),
                _ => None,
            })
            .collect();
        // A pane closed any way at all takes its script with it.
        self.model
            .extension_panes
            .retain(|pane_id, _| open.iter().any(|(open, _)| open == pane_id));
        for (pane_id, name) in open {
            if self.model.extension_panes.contains_key(&pane_id) {
                continue;
            }
            if let Some(pane) = self.start_extension(&name, ctx) {
                self.model.extension_panes.insert(pane_id, pane);
            }
        }

        // The tab in front of each frame is what can be seen; a script behind another tab is
        // not ticked - see `Input::Visible`.
        let in_front: Vec<PaneId> = self
            .model
            .layout
            .frame_ids()
            .into_iter()
            .filter_map(|frame| self.model.layout.frame(frame)?.active_pane())
            .collect();

        let mut effects = Vec::new();
        for (pane_id, pane) in &mut self.model.extension_panes {
            let Some(running) = &pane.running else {
                continue;
            };
            let visible = in_front.contains(pane_id);
            if visible != pane.visible {
                pane.visible = visible;
                running.send(Input::Visible(visible));
            }
            for output in running.take_outputs() {
                match output {
                    Output::Drawn { view, error } => {
                        if let Some(view) = view {
                            pane.view = Some(view);
                        }
                        pane.error = error;
                    }
                    Output::Loaded { takes_keys } => pane.takes_keys = takes_keys,
                    Output::Effect(effect) => effects.push(effect),
                }
            }
        }
        for effect in effects {
            self.apply_extension_effect(effect, ctx);
        }
    }

    /// The pane for an extension, or `None` while the window does not know its project yet -
    /// the review is still loading, which is where the project's path comes from - so a later
    /// frame starts it.
    fn start_extension(&self, name: &str, ctx: &egui::Context) -> Option<ExtensionPane> {
        let Some(extension) = extensions::named(name) else {
            return Some(ExtensionPane::failed(format!(
                "there is no extension called {name}"
            )));
        };
        // A script reads folders and runs programs on the machine the window is on, which is
        // only the project's when the review is local.
        if !self.backend().reads_this_machine() {
            return Some(ExtensionPane::failed(
                "extensions run on this machine, and this window's project is on another one"
                    .to_string(),
            ));
        }
        let project_root = self.repo_root()?;
        let host = Host {
            project_root,
            path: crate::shell_path::installed_tools_path().to_string(),
        };
        let repaint = ctx.clone();
        Some(ExtensionPane {
            running: Some(Running::start(extension, host, move || {
                repaint.request_repaint();
            })),
            view: None,
            error: None,
            scrolled_to: None,
            takes_keys: false,
            visible: true,
            inputs: HashMap::new(),
        })
    }

    /// Start an open extension over: its script read again and run from `init`. One that
    /// could not be started at all is started afresh on the next frame.
    pub(crate) fn restart_extension(&mut self, name: &str) {
        let Some((pane_id, _)) = self
            .model
            .layout
            .find_pane(|pane| pane.runs_extension(name))
        else {
            return;
        };
        match self
            .model
            .extension_panes
            .get(&pane_id)
            .and_then(|pane| pane.running.as_ref())
        {
            Some(running) => running.send(Input::Restart),
            None => {
                self.model.extension_panes.remove(&pane_id);
            }
        }
    }

    fn apply_extension_effect(&mut self, effect: Effect, ctx: &egui::Context) {
        match effect {
            Effect::OpenFile { path, line } => {
                let root = self
                    .repo_root()
                    .expect("an extension only runs on a project on this machine");
                // A file tab names its file by its path in the repo, so a file anywhere else
                // has no name to be opened under.
                match path.strip_prefix(&root) {
                    Ok(inside) => {
                        let session_id = self.model.root_session_id.clone();
                        self.open_pane(OpenPaneRequest::File {
                            session_id,
                            file_path: inside.to_string_lossy().to_string(),
                            // A line to bring on screen, and nothing searched for to mark.
                            at: line.map(|line| OpenAt {
                                line,
                                query: String::new(),
                            }),
                        });
                    }
                    Err(_) => self.model.error(format!(
                        "{} is outside {}, and only files of the project open in a tab",
                        path.display(),
                        root.display()
                    )),
                }
            }
            Effect::OpenShell(command) => self.run_in_shell(command),
            Effect::Copy(text) => ctx.copy_text(text),
            Effect::Notify(said) => self.model.info(said),
            Effect::Log(said) => self.model.messages.record(
                ToastKind::Info,
                said,
                crate::native::messages::now_unix(),
            ),
            Effect::Failed(said) => self.model.messages.record(
                ToastKind::Error,
                said,
                crate::native::messages::now_unix(),
            ),
        }
    }

    /// Hand the extension in front this frame's key presses, unless a text box has the
    /// keyboard. Taken out of the input, so nothing under the pane acts on them as well.
    pub(crate) fn forward_keys_to_extension(&mut self, ctx: &egui::Context) {
        let Some(pane_id) = self
            .active_pane()
            .and_then(|(pane_id, pane)| matches!(pane, Pane::Extension { .. }).then_some(pane_id))
        else {
            return;
        };
        // The palette and the find bar are drawn over the pane and answer Escape themselves,
        // whether or not their box has the keyboard yet - a key taken here would never reach them.
        if ctx.egui_wants_keyboard_input() || self.model.palette.open || self.model.find.is_some() {
            return;
        }
        let Some(running) = self
            .model
            .extension_panes
            .get(&pane_id)
            .filter(|pane| pane.takes_keys)
            .and_then(|pane| pane.running.as_ref())
        else {
            return;
        };

        let keys = ctx.input_mut(|input| {
            // A character typed with a chord held - ⌥ on macOS types one - is the window's,
            // like the chord itself.
            let chorded = input.modifiers.command || input.modifiers.ctrl || input.modifiers.alt;
            let mut keys = Vec::new();
            input.events.retain(|event| match event {
                egui::Event::Text(typed) if !chorded => {
                    keys.extend(typed.chars().map(String::from));
                    false
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !(modifiers.command || modifiers.ctrl || modifiers.alt)
                    && NAMED_KEYS.contains(key) =>
                {
                    // Shift is heard on the keys that type nothing - `Shift+Tab` is not `Tab`.
                    // On the ones that type, it is already in the character.
                    keys.push(match modifiers.shift {
                        true => format!("Shift+{}", key.name()),
                        false => key.name().to_string(),
                    });
                    false
                }
                _ => true,
            });
            keys
        });
        for key in keys {
            running.send(Input::Key(key));
        }
    }
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, pane_id: PaneId) {
    let palette = app.palette_of();
    let Some(pane) = app.model.extension_panes.get_mut(&pane_id) else {
        // Not started yet: opened this very frame, or waiting on the review for the project's
        // path - see `App::start_extension`.
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("waiting for the project…").color(palette.muted));
        });
        return;
    };

    let selected_row = pane.view.as_ref().and_then(Element::selected_row);
    let scroll_to = selected_row.filter(|_| selected_row != pane.scrolled_to);
    pane.scrolled_to = selected_row;

    let mut drawing = Drawing {
        palette: &palette,
        clicked: None,
        scroll_to,
        ids: 0,
        inputs: &mut pane.inputs,
    };
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            if let Some(error) = &pane.error {
                ui.add(
                    egui::Label::new(RichText::new(error).monospace().color(palette.warn)).wrap(),
                );
                widgets::divider(ui, &palette);
            }
            match &pane.view {
                Some(view) => drawing.element(ui, view, false),
                None if pane.error.is_none() => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("starting…").color(palette.muted));
                    });
                }
                None => {}
            }
        });

    if let (Some(event), Some(running)) = (drawing.clicked, &pane.running) {
        running.send(Input::Event(event));
    }
}

/// One pass over a view.
struct Drawing<'a> {
    palette: &'a Palette,
    /// What the click or keystroke this frame sends to `update`: what the button, row or menu
    /// entry carried, or an input's `on_change` with what is now typed.
    clicked: Option<Value>,
    /// The row the first table is to bring into sight.
    scroll_to: Option<usize>,
    /// Counts the scrolled elements drawn so far, which is what tells their ids apart.
    ids: usize,
    /// The pane's inputs - see [`ExtensionPane::inputs`].
    inputs: &'a mut HashMap<String, String>,
}

impl Drawing<'_> {
    /// `in_cell` for an element in a table's cell, where text stays on the one line the row
    /// has height for, at its whole width: that width is what sizes the column, and the
    /// column clips it once it is dragged narrower. A label cut to fit would ask for no width
    /// at all, and every column would stay as narrow as it started.
    fn element(&mut self, ui: &mut Ui, element: &Element, in_cell: bool) {
        match element {
            Element::Text {
                text,
                ink,
                strong,
                mono,
                small,
            } => {
                let mut rich = RichText::new(text).color(ink_of(self.palette, *ink));
                if *strong {
                    rich = rich.strong();
                }
                if *mono {
                    rich = rich.monospace();
                }
                if *small {
                    rich = rich.size(SMALL_SIZE);
                }
                let label = egui::Label::new(rich);
                ui.add(match in_cell {
                    // Not selectable: a label that can be selected takes the click for itself,
                    // and a click on a row's words is a click on the row.
                    true => label.extend().selectable(false),
                    false => label.wrap(),
                });
            }
            Element::Heading { text } => {
                ui.label(
                    RichText::new(text)
                        .size(UI_SIZE + 3.0)
                        .strong()
                        .color(self.palette.ink),
                );
            }
            Element::Row { children } if in_cell => {
                ui.horizontal(|ui| {
                    for child in children {
                        self.element(ui, child, in_cell);
                    }
                });
            }
            Element::Row { children } => {
                ui.horizontal_wrapped(|ui| {
                    for child in children {
                        self.element(ui, child, in_cell);
                    }
                });
            }
            Element::Column { children } => {
                ui.vertical(|ui| {
                    for child in children {
                        self.element(ui, child, in_cell);
                    }
                });
            }
            Element::Button {
                label,
                on_click,
                disabled,
            } => {
                let button = egui::Button::new(RichText::new(label).size(SMALL_SIZE));
                if widgets::clickable(ui.add_enabled(!disabled, button)).clicked() {
                    self.clicked = Some(on_click.clone());
                }
            }
            Element::Code { text } => {
                self.ids += 1;
                egui::ScrollArea::vertical()
                    .id_salt(("extension-code", self.ids))
                    .stick_to_bottom(true)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(text)
                                .monospace()
                                .size(SMALL_SIZE)
                                .color(self.palette.ink),
                        );
                    });
            }
            Element::Separator => widgets::divider(ui, self.palette),
            Element::Table { columns, rows } => self.table(ui, columns, rows),
            Element::Input {
                id,
                value,
                hint,
                on_change,
            } => {
                let typed = self
                    .inputs
                    .entry(id.clone())
                    .or_insert_with(|| value.clone());
                let edit_id = ui.make_persistent_id(("extension-input", id));
                // The script's value stands while the box has not got the keyboard, so a script
                // that clears its filter clears the box. While it has, what is being typed
                // stands: the script is a keystroke behind it.
                if !ui.memory(|memory| memory.has_focus(edit_id)) && typed != value {
                    *typed = value.clone();
                }
                let response = ui.add(
                    egui::TextEdit::singleline(typed)
                        .id(edit_id)
                        .hint_text(hint.as_str())
                        .desired_width(INPUT_WIDTH),
                );
                if response.changed() {
                    let Value::Object(mut event) = on_change.clone() else {
                        unreachable!("an input's on_change is checked to be a map")
                    };
                    event.insert("value".to_string(), Value::String(typed.clone()));
                    self.clicked = Some(Value::Object(event));
                }
            }
        }
    }

    fn table(&mut self, ui: &mut Ui, columns: &[String], rows: &[TableRow]) {
        self.ids += 1;
        let row_height = ui.text_style_height(&TextStyle::Body) + 6.0;
        let height = ui.available_height();
        let palette = self.palette;
        // Only the first table follows the keyboard, which is the one `selected_row` read.
        let scroll_to = self.scroll_to.take();

        ui.push_id(("extension-table", self.ids), |ui| {
            // The window's own shades for the row under the keyboard and the one under the
            // pointer, rather than egui's selection color: that is the accent, which is the ink a
            // folder's name is written in and unreadable on itself.
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = palette.hunk_active_bg;
            visuals.selection.stroke.color = palette.ink;
            visuals.widgets.hovered.bg_fill = palette.row_hover_bg;
            ui.spacing_mut().item_spacing.x = COLUMN_GAP;

            let mut table = TableBuilder::new(ui)
                .sense(Sense::click())
                .auto_shrink([false, false])
                .min_scrolled_height(0.0)
                .max_scroll_height(height)
                .cell_layout(Layout::left_to_right(Align::Center));
            for index in 0..columns.len() {
                // Each column as wide as what is in it, and the last one the rest of the pane.
                let column = match index + 1 == columns.len() {
                    true => Column::remainder(),
                    false => Column::auto().at_least(MIN_COLUMN_WIDTH).resizable(true),
                };
                table = table.column(column.clip(true));
            }
            if let Some(row) = scroll_to {
                table = table.scroll_to_row(row, None);
            }

            table
                .header(row_height, |mut header| {
                    for name in columns {
                        header.col(|ui| {
                            ui.label(RichText::new(name).size(SMALL_SIZE).color(palette.muted));
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_height, rows.len(), |mut table_row| {
                        let row = &rows[table_row.index()];
                        table_row.set_selected(row.selected);
                        for cell in &row.cells {
                            table_row.col(|ui| self.element(ui, cell, true));
                        }
                        let response = table_row.response();
                        if !row.menu.is_empty() {
                            response.context_menu(|ui| {
                                for item in &row.menu {
                                    if ui.button(&item.label).clicked() {
                                        self.clicked = Some(item.event.clone());
                                        ui.close();
                                    }
                                }
                            });
                        }
                        let carried = match (response.double_clicked(), response.clicked()) {
                            (true, _) => &row.on_double_click,
                            (false, true) => &row.on_click,
                            (false, false) => &None,
                        };
                        if let Some(event) = carried {
                            self.clicked = Some(event.clone());
                        }
                    });
                });
        });
    }
}

fn ink_of(palette: &Palette, ink: Ink) -> Color32 {
    match ink {
        Ink::Normal => palette.ink,
        Ink::Muted => palette.muted,
        Ink::Accent => palette.accent,
        Ink::Warn => palette.warn,
        Ink::Added => palette.added,
        Ink::Removed => palette.removed,
    }
}
