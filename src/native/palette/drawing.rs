//! Drawing the palette: the query box over the list, the scope it searches, and a row.

use egui::{Align2, Color32, CornerRadius, Key, RichText, Stroke, StrokeKind, vec2};

use crate::{
    api::SearchScope,
    native::{
        app::App,
        bindings::{self},
        theme::{Palette, SMALL_SIZE},
    },
};

use super::rows::{
    empty_message, footnote, hint_of, refresh_content_matches, refresh_file_matches,
};
use super::{Command, PaletteMode, rows_for};

/// How much of the window's height the palette's rows may take before they scroll. With the
/// palette hung 12% of the way down, this keeps the whole of it on screen.
const ROWS_HEIGHT_OF_SCREEN: f32 = 0.55;

/// The height of one row of the list.
pub(super) const ROW_HEIGHT: f32 = 34.0;

pub(crate) fn draw(app: &mut App, ctx: &egui::Context) {
    if !app.model.palette.open {
        return;
    }
    // A press anywhere else - a shell, a tab, a pane in the next frame over - puts the palette
    // away and belongs to whatever was pressed. It is answered before anything is drawn: the
    // search box asks for the keyboard every frame it exists, so a palette still on screen
    // would take it straight back off the shell that was just clicked.
    if pressed_outside(ctx, app.model.palette.rect) {
        app.model.palette.dismiss();
        return;
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
    let matches = rows_for(app);

    let (dismiss, move_down, move_up, accept) = ctx.input_mut(|input| {
        (
            input.key_pressed(Key::Escape),
            input.key_pressed(Key::ArrowDown),
            input.key_pressed(Key::ArrowUp),
            input.key_pressed(Key::Enter),
        )
    });

    if dismiss {
        app.model.palette.dismiss();
        return;
    }
    // Typed since the highlight was picked: the list underneath it is a different list, and
    // the first match of the new one is what Enter runs.
    let retyped = app.model.palette.highlight_query != app.model.palette.query;
    if retyped {
        app.model.palette.highlighted = 0;
        app.model.palette.highlight_query = app.model.palette.query.clone();
    }
    if !matches.is_empty() {
        let last = matches.len() - 1;
        if move_down {
            app.model.palette.highlighted = (app.model.palette.highlighted + 1).min(last);
        }
        if move_up {
            app.model.palette.highlighted = app.model.palette.highlighted.saturating_sub(1);
        }
        app.model.palette.highlighted = app.model.palette.highlighted.min(last);
    }

    let mut chosen: Option<usize> = None;
    if accept && !matches.is_empty() {
        chosen = Some(app.model.palette.highlighted);
    }

    let screen = ctx.viewport_rect();
    let area = egui::Area::new("moonreview-palette".into())
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, screen.height() * 0.12))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.line))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(9))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 10],
                    blur: 28,
                    spread: 0,
                    color: Color32::from_black_alpha(60),
                })
                .show(ui, |ui| {
                    ui.set_width((screen.width() * 0.5).clamp(360.0, 560.0));

                    let hint = hint_of(app);
                    let entry = ui.add(
                        egui::TextEdit::singleline(&mut app.model.palette.query)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(7, 5)),
                    );
                    entry.request_focus();
                    // Opened on a name to type over, which is selected so the first key typed
                    // replaces it - see `PaletteState::show_rename`.
                    if std::mem::take(&mut app.model.palette.select_query) {
                        select_all(ui.ctx(), entry.id, &app.model.palette.query);
                    }
                    if matches!(
                        app.model.palette.mode,
                        PaletteMode::Files | PaletteMode::Contents
                    ) {
                        draw_scope_box(ui, &mut app.model.palette.search_scope, &palette);
                    }

                    ui.add_space(6.0);
                    if matches.is_empty() {
                        ui.label(RichText::new(empty_message(app)).color(palette.muted));
                        return;
                    }

                    // The rows scroll rather than the box growing: a list taller than the
                    // window would be pushed up to fit, search line and all, every time the
                    // matches came in. The height is the rows' own, capped - an area hands
                    // its contents last frame's rect to fit in, so a scroll area left to size
                    // itself would stay as short as the empty palette was. The highlight is
                    // kept in view when the keyboard moves it; a pointer over a row is
                    // already looking at it.
                    let keep_highlight_in_view = move_down || move_up || retyped;
                    let rows_height = rows_height_of(matches.len(), ui.spacing().item_spacing.y)
                        .min(screen.height() * ROWS_HEIGHT_OF_SCREEN);
                    egui::ScrollArea::vertical()
                        .max_height(rows_height)
                        .min_scrolled_height(rows_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (index, command) in matches.iter().enumerate() {
                                let highlighted = index == app.model.palette.highlighted;
                                let row = draw_row(ui, command, highlighted, &palette);
                                if highlighted && keep_highlight_in_view {
                                    row.scroll_to_me(None);
                                }
                                if row.clicked() {
                                    chosen = Some(index);
                                }
                                if row.hovered() {
                                    app.model.palette.highlighted = index;
                                }
                            }
                        });
                    if let Some(footnote) = footnote(app, matches.len()) {
                        ui.label(
                            RichText::new(footnote)
                                .size(SMALL_SIZE - 1.0)
                                .color(palette.muted),
                        );
                    }
                });
        });
    app.model.palette.rect = Some(area.response.rect);

    if let Some(index) = chosen
        && let Some(command) = matches.into_iter().nth(index)
    {
        app.model.palette.dismiss();
        app.pending_action = Some(command.action);
    }
}

/// How tall `rows` rows are when laid out one under the other: the rows themselves, and the
/// layout's gap between each pair. Counting the rows alone left the area a gap short per row,
/// which put a scrollbar on a list of three rows that all fit.
pub(super) fn rows_height_of(rows: usize, gap: f32) -> f32 {
    rows as f32 * ROW_HEIGHT + rows.saturating_sub(1) as f32 * gap
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

/// Select the whole of the palette's line.
fn select_all(ctx: &egui::Context, id: egui::Id, query: &str) {
    let Some(mut state) = egui::TextEdit::load_state(ctx, id) else {
        return;
    };
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(query.chars().count()),
        )));
    state.store(ctx, id);
}

/// Whether a pointer button went down this frame away from where the palette drew last frame.
/// Before it has drawn once there is nowhere to be outside of, and the press is somebody
/// else's business.
fn pressed_outside(ctx: &egui::Context, drawn_at: Option<egui::Rect>) -> bool {
    let Some(drawn_at) = drawn_at else {
        return false;
    };
    ctx.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|at| !drawn_at.contains(at))
    })
}

fn draw_row(
    ui: &mut egui::Ui,
    command: &Command,
    highlighted: bool,
    palette: &Palette,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), egui::Sense::click());
    let response = crate::native::widgets::clickable(response);

    if ui.is_rect_visible(rect) {
        if highlighted {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(5), palette.control_active_bg);
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(5),
                Stroke::new(1.0, palette.accent),
                StrokeKind::Inside,
            );
        }
        ui.painter().text(
            rect.min + vec2(9.0, 5.0),
            Align2::LEFT_TOP,
            &command.title,
            egui::FontId::proportional(crate::native::theme::UI_SIZE),
            palette.ink,
        );
        ui.painter().text(
            rect.min + vec2(9.0, 19.0),
            Align2::LEFT_TOP,
            &command.description,
            egui::FontId::proportional(SMALL_SIZE - 1.0),
            palette.muted,
        );
        // The keyboard's own way to the same command, against the right edge.
        if let Some(chord) = command.shortcut {
            ui.painter().text(
                egui::pos2(rect.max.x - 9.0, rect.center().y),
                Align2::RIGHT_CENTER,
                bindings::describe(chord),
                egui::FontId::proportional(SMALL_SIZE - 1.0),
                palette.muted,
            );
        }
    }
    response
}
