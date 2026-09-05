//! The project pane: the two commands the Project menu runs, and where they are set.
//!
//! What builds a repo is a fact about the repo, so it is kept in the repo - see
//! [`crate::project`]. This is the frame that writes that file, so a project is configured
//! from the window rather than by finding the file first.

use egui::{CornerRadius, RichText, Sense, Stroke, Ui, vec2};

use crate::{
    native::{
        app::App,
        theme::{Palette, SMALL_SIZE},
        widgets,
        workspace_color::{self, WorkspaceColor},
    },
    project::ProjectCommand,
};

/// What each box is for, in the order the menu has them. The pane is two of the same row, so
/// what differs between them is here rather than written out twice.
///
/// Every box is labelled above it as well as hinted inside it: the hint is gone the moment
/// anything is typed, and a filled-in box with nothing over it says nothing about which of
/// the two commands it holds.
const BOXES: &[(ProjectCommand, &str)] = &[
    (
        ProjectCommand::Build,
        "the command that builds this project",
    ),
    (
        ProjectCommand::Run,
        // The one word that is not a line of shell - see `crate::project::RESTART_RUN_COMMAND`.
        "the command that runs this project, or @restart to start this window again",
    ),
];

/// How wide a command box gets. A command line is longer than a name and shorter than a
/// paragraph, and the pane is often a narrow column beside a review.
const BOX_WIDTH: f32 = 420.0;

pub(crate) fn draw(app: &mut App, ui: &mut Ui) {
    let palette = app.palette_of();

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new("Project Settings")
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
            widgets::divider(ui, &palette);
            ui.add_space(8.0);

            draw_commands(app, ui, &palette);
            ui.add_space(16.0);
            draw_workspace_color(app, ui, &palette);
        });
}

/// The two boxes that write the repo's file.
fn draw_commands(app: &mut App, ui: &mut Ui, palette: &Palette) {
    // The file is read when the pane opens; until it is back there is nothing to put in the
    // boxes, and empty boxes would read as a project with no commands set.
    let Some(editor) = &mut app.model.project_editor else {
        ui.label(
            RichText::new("reading the project file")
                .size(SMALL_SIZE)
                .color(palette.muted),
        );
        return;
    };

    // The pane was opened to type in, so the first box has the keyboard from the moment the
    // file is back and there is something to type into.
    let mut takes_keyboard = std::mem::take(&mut app.model.project_focus);
    let mut edited = false;
    for (which, hint) in BOXES {
        ui.label(
            RichText::new(format!("{} command", which.label()))
                .size(SMALL_SIZE)
                .strong(),
        );
        ui.add_space(2.0);
        let entry = ui.add(
            egui::TextEdit::singleline(editor.text_mut(*which))
                .hint_text(*hint)
                .desired_width(BOX_WIDTH)
                .margin(egui::Margin::symmetric(6, 4)),
        );
        if std::mem::take(&mut takes_keyboard) {
            entry.request_focus();
        }
        edited |= entry.changed();
        ui.add_space(10.0);
    }

    // Saved as it is typed rather than behind a button: there are two boxes and one file,
    // and a command left unsaved is a menu item that does not do what the pane says it does.
    // The write itself is `App::save_project`, one at a time.
    app.model.project_unsaved |= edited;
}

/// How big one color's swatch is.
const SWATCH: f32 = 22.0;

/// The row of swatches this window's ground is picked from.
///
/// Unlike the two boxes above, this writes nothing to the repo: which color a window is
/// belongs to whoever is looking at it, so it is kept in `~/.moonreview/settings.json`
/// against the project's path - see `App::set_workspace_color`. The row is here because this
/// is the pane about this project.
fn draw_workspace_color(app: &mut App, ui: &mut Ui, palette: &Palette) {
    ui.label(RichText::new("workspace color").size(SMALL_SIZE).strong());
    ui.add_space(4.0);

    let current = app.model.workspace_color;
    let mode = app.model.theme;
    // The pick is taken out of the loop: drawing borrows the app, and marking the workspace
    // wants it back.
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        for color in workspace_color::ALL {
            if swatch(ui, palette, color, mode, color == current)
                .on_hover_text(color.label())
                .clicked()
            {
                picked = Some(color);
            }
        }
    });

    ui.add_space(4.0);
    ui.label(
        RichText::new(current.label())
            .size(SMALL_SIZE)
            .color(palette.muted),
    );

    if let Some(color) = picked {
        app.set_workspace_color(color);
    }
}

/// One color, as the window would be painted in it. The swatch is the ground itself rather
/// than a bright sample of the hue: what it has to answer is what the window will look like.
fn swatch(
    ui: &mut Ui,
    palette: &Palette,
    color: WorkspaceColor,
    mode: crate::native::theme::ThemeMode,
    current: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    let response = widgets::clickable(response);
    if !ui.is_rect_visible(rect) {
        return response;
    }

    // The grounds are near-neutral by design, so which one is picked is said by the border
    // rather than by the fill - the fills are too close together to carry it alone.
    let border = if current {
        Stroke::new(2.0, palette.accent)
    } else if response.hovered() {
        Stroke::new(1.0, palette.ink)
    } else {
        Stroke::new(1.0, palette.line)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        color.bg(mode),
        border,
        egui::StrokeKind::Inside,
    );
    response
}
