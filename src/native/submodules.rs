//! The submodule hub: the reviewed repo and every submodule of it, each with how many of its
//! files have changed, and a way into a review of the ones that have changes.
//!
//! The list itself is `model.root_repo_status` and `model.submodules`, which
//! `App::poll_submodules` keeps up to date for the palette; this pane draws the same answer
//! whole rather than only its changed rows.

use egui::{Align, CornerRadius, Key, Layout, Modifiers, RichText, Sense, Stroke, Ui, vec2};

use crate::native::{
    app::App,
    palette::CommandAction,
    panes::OpenPaneRequest,
    theme::{Palette, SMALL_SIZE},
    widgets,
};

/// The gap between the change count and the path that follows it. The count's column is as
/// wide as the longest count in the list, so every path starts at one x without the column
/// being wider than what it holds.
const CHANGES_COLUMN_GAP: f32 = 14.0;
/// The padding inside a row, either side. A folder's label is indented by it too, so the
/// label and the counts under it start at one x.
const ROW_PADDING: f32 = 8.0;
/// How long the fill and border take to come up under the pointer, and to go again.
const HOVER_FADE: f32 = 0.1;
/// How wide the query box is. Enough for a path of a folder and part of a name, which is
/// what is typed into it.
const FILTER_BOX_WIDTH: f32 = 200.0;

/// The gap between a row's right edge and the shell button beside it.
const SHELL_BUTTON_GAP: f32 = 6.0;
/// The shell button's box.
const SHELL_BUTTON_SIZE: f32 = 18.0;

/// The query the hub is being narrowed by, ready to match paths against: trimmed and folded
/// to lowercase once here rather than once per submodule.
struct Filter(String);

impl Filter {
    fn of(query: &str) -> Self {
        Self(query.trim().to_lowercase())
    }

    /// Whether the query is leaving anything out at all.
    fn is_on(&self) -> bool {
        !self.0.is_empty()
    }

    /// Whether this submodule is one of the ones the query asks for. The whole path under the
    /// repo is looked through, so `gen` finds `generator` as well as every submodule inside
    /// `generator/` - the folder in the heading is part of what a row is found by.
    fn matches(&self, path_under_repo: &str) -> bool {
        !self.is_on() || path_under_repo.to_lowercase().contains(&self.0)
    }
}

/// One row of the hub: what it says, and the review it opens.
struct Row {
    /// The repo's own directory name, which is all a row says: the folder it sits in is the
    /// heading above it.
    name: String,
    /// The same with its folder in front, which is what the row's hover offers to review.
    path_under_repo: String,
    opens: OpenPaneRequest,
    /// Where the row's shell button opens a shell.
    repo_path: String,
    changes: String,
    changed: bool,
    /// What the row says under its name when its repo has commits its remote has not got,
    /// and nothing when it has none.
    unpushed: Option<String>,
}

/// The submodules of one folder of the repo, under that folder's name. A submodule at the top
/// of the repo sits in no folder, and its group has no heading.
struct Group {
    folder: String,
    rows: Vec<Row>,
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui) {
    let palette = app.palette_of();
    let filter = Filter::of(&app.model.submodule_filter);
    let groups = groups_of(app, &filter);
    let rows = || groups.iter().flat_map(|group| group.rows.iter());
    let (total, changed) = (rows().count(), rows().filter(|row| row.changed).count());
    // The rows are as wide as the longest name in them rather than as wide as the pane: a
    // list of short names in a wide window reads as a list, not as a table of empty space.
    let body = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::FontId::proportional(SMALL_SIZE);
    // The unpushed note is written under the name, so the name's column is as wide as
    // whichever of the two is wider.
    let widest_name = widest(ui, rows().map(|row| &row.name), &body, &palette).max(widest(
        ui,
        rows().filter_map(|row| row.unpushed.as_ref()),
        &small,
        &palette,
    ));
    let changes_column =
        widest(ui, rows().map(|row| &row.changes), &small, &palette) + CHANGES_COLUMN_GAP;
    // The shell button sits outside the row, to its right, so a narrow pane leaves room for it.
    let row_width = (changes_column + widest_name + ROW_PADDING * 2.0)
        .min(ui.available_width() - 2.0 - SHELL_BUTTON_GAP - SHELL_BUTTON_SIZE);

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(9, 7))
        .show(ui, |ui| {
            // No title: the tab is called submodules, so the header is the box the list is
            // narrowed with and the count of what it is showing.
            draw_filter_box(app, ui, &palette);
            ui.label(
                RichText::new(format!("{changed} of {total} changed"))
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
            widgets::divider(ui, &palette);
            ui.add_space(5.0);

            // The repo itself heads the list, so a query that leaves it out leaves the list
            // empty; without a query the repo is always there, and only its submodules can
            // be missing.
            if groups.is_empty() {
                ui.label(RichText::new("no submodule matches").color(palette.muted));
                return;
            }
            let no_submodules = !filter.is_on() && app.model.submodules.is_empty();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("submodule-hub-list")
                .show(ui, |ui| {
                    // Centered, so the list sits in the pane rather than against its edge.
                    ui.vertical_centered(|ui| {
                        for group in &groups {
                            ui.allocate_ui(vec2(row_width, 0.0), |ui| {
                                ui.vertical(|ui| {
                                    if !group.folder.is_empty() {
                                        ui.horizontal(|ui| {
                                            ui.add_space(ROW_PADDING);
                                            // With the trailing slash the heading reads as
                                            // the folder it is rather than as another name
                                            // in the list of submodule names under it.
                                            ui.label(
                                                RichText::new(format!("{}/", group.folder))
                                                    .size(SMALL_SIZE)
                                                    .color(palette.muted),
                                            );
                                        });
                                        ui.add_space(2.0);
                                    }
                                    for row in &group.rows {
                                        draw_row(app, ui, row, changes_column, row_width, &palette);
                                    }
                                });
                            });
                            ui.add_space(8.0);
                        }
                        if no_submodules {
                            ui.label(
                                RichText::new("this repo has no submodules").color(palette.muted),
                            );
                        }
                    });
                });
        });
}

/// The box the list is narrowed with, and the way back to the whole list.
///
/// It takes the keyboard when the hub is opened, so a hub brought up to find one submodule
/// among many is already waiting for the name.
fn draw_filter_box(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let mut cleared = false;

    ui.horizontal(|ui| {
        let entry = ui.add(
            egui::TextEdit::singleline(&mut app.model.submodule_filter)
                .hint_text("Filter submodules")
                .desired_width(FILTER_BOX_WIDTH)
                .margin(egui::Margin::symmetric(6, 3)),
        );
        if std::mem::take(&mut app.model.submodule_filter_focus) {
            entry.request_focus();
        }

        // Escape empties the box rather than leaving a query on that nobody is asking about
        // any more. egui takes the keyboard off the box when Escape is pressed, which is what
        // `lost_focus` is here; the key itself is taken with it so nothing further down the
        // window acts on the same press.
        if entry.lost_focus()
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            cleared = true;
        }

        if !Filter::of(&app.model.submodule_filter).is_on() {
            return;
        }
        if widgets::close_button(ui, palette)
            .on_hover_text("Show every submodule again")
            .clicked()
        {
            cleared = true;
        }
    });

    if cleared {
        app.model.submodule_filter.clear();
    }
    ui.add_space(4.0);
}

/// The width the widest of these texts draws at, which is what a column is sized to.
fn widest<'a>(
    ui: &Ui,
    texts: impl Iterator<Item = &'a String>,
    font: &egui::FontId,
    palette: &Palette,
) -> f32 {
    texts
        .map(|text| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(text.clone(), font.clone(), palette.ink)
                    .size()
                    .x
            })
        })
        .fold(0.0f32, f32::max)
}

/// The reviewed repo first, in a group of its own, then the submodules the query asks for,
/// gathered under the folder of the repo each sits in. They arrive sorted by path, so a
/// folder's submodules are already together and the groups keep that order. A folder whose
/// submodules all fall to the query loses its heading with them, because its group is never
/// made.
fn groups_of(app: &App, filter: &Filter) -> Vec<Group> {
    let root_repo_path = app
        .model
        .review_ref(&app.model.root_session_id)
        .and_then(|review| review.payload.as_ref())
        .map(|payload| payload.repo_path.clone());

    let mut groups: Vec<Group> = Vec::new();
    if let Some(root) = app
        .model
        .root_repo_status
        .as_ref()
        .filter(|root| filter.matches(&root.name))
    {
        groups.push(Group {
            folder: String::new(),
            rows: vec![Row {
                name: root.name.clone(),
                path_under_repo: root.name.clone(),
                // The review of the repo itself is the one the window opened on, so the row
                // brings that forward rather than opening a second one.
                opens: OpenPaneRequest::Review {
                    session_id: app.model.root_session_id.clone(),
                    title: "review".to_string(),
                },
                repo_path: root.repo_path.clone(),
                changes: changes_label(root.changed_files),
                changed: root.changed_files > 0,
                unpushed: unpushed_label(root.unpushed_commits),
            }],
        });
    }
    for submodule in &app.model.submodules {
        let path = path_under_repo(&submodule.repo_path, root_repo_path.as_deref());
        if !filter.matches(path) {
            continue;
        }
        let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
        let row = Row {
            name: submodule.name.clone(),
            path_under_repo: path.to_string(),
            opens: OpenPaneRequest::ReviewRepo {
                repo_path: submodule.repo_path.clone(),
                title: submodule.name.clone(),
            },
            repo_path: submodule.repo_path.clone(),
            changes: changes_label(submodule.changed_files),
            changed: submodule.changed_files > 0,
            unpushed: unpushed_label(submodule.unpushed_commits),
        };
        match groups.last_mut() {
            Some(group) if group.folder == folder => group.rows.push(row),
            _ => groups.push(Group {
                folder: folder.to_string(),
                rows: vec![row],
            }),
        }
    }
    groups
}

pub(crate) fn changes_label(changed_files: usize) -> String {
    match changed_files {
        0 => "no changes".to_string(),
        1 => "1 change".to_string(),
        count => format!("{} changes", widgets::grouped(count)),
    }
}

fn unpushed_label(unpushed_commits: usize) -> Option<String> {
    match unpushed_commits {
        0 => None,
        1 => Some("1 commit not pushed".to_string()),
        count => Some(format!("{} commits not pushed", widgets::grouped(count))),
    }
}

/// A row is one target: anywhere on it opens the review of that repo. The shell button to its
/// right opens a shell in that repo instead.
fn draw_row(
    app: &mut App,
    ui: &mut Ui,
    row: &Row,
    changes_column: f32,
    row_width: f32,
    palette: &Palette,
) {
    // The id the row is interacted with below, made here so what is drawn and what is
    // clicked are the same target rather than two ids that happen to look alike.
    let id = ui.id().with(&row.path_under_repo);
    let shell_id = id.with("shell");
    // Where the name was drawn, which the shell button beside the row is levelled with.
    let mut name_rect = egui::Rect::NOTHING;
    let response = ui.allocate_ui(vec2(row_width, 0.0), |ui| {
        let hovered = ui
            .ctx()
            .read_response(id)
            .is_some_and(|response| response.hovered());
        // Nothing at all until the pointer is on it: at rest the hub is a list of names, and
        // the fill and border fading up are what say the whole block is one target.
        let shown = ui
            .ctx()
            .animate_bool_with_time(id.with("hover"), hovered, HOVER_FADE);

        // A repo with commits its remote has not got keeps its border at rest: that is the
        // one thing on the hub that wants doing before anything built on the repo is shared.
        let border = if row.unpushed.is_some() {
            palette.unpushed
        } else {
            palette.accent.gamma_multiply(shown)
        };
        egui::Frame::new()
            .fill(palette.control_active_bg.gamma_multiply(shown))
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(ROW_PADDING as i8, 6))
            .show(ui, |ui| {
                ui.set_min_width(row_width - ROW_PADDING * 2.0);
                ui.horizontal(|ui| {
                    // The count's cell is as tall as the name beside it, so the two sit on
                    // one line however much smaller the count is written.
                    let name_height = ui.text_style_height(&egui::TextStyle::Body);
                    ui.allocate_ui_with_layout(
                        vec2(changes_column, name_height),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_min_size(vec2(changes_column, name_height));
                            ui.label(RichText::new(&row.changes).size(SMALL_SIZE).color(
                                if row.changed {
                                    palette.accent
                                } else {
                                    palette.muted
                                },
                            ));
                        },
                    );
                    name_rect = ui.label(RichText::new(&row.name).strong()).rect;
                });
                if let Some(unpushed) = &row.unpushed {
                    ui.horizontal(|ui| {
                        // The name's line has a gap between its count and its name that a
                        // space does not: without it the note starts short of the name.
                        ui.add_space(changes_column + ui.spacing().item_spacing.x);
                        ui.label(
                            RichText::new(unpushed)
                                .size(SMALL_SIZE)
                                .color(palette.unpushed),
                        );
                    });
                }
            })
            .response
    });

    let clicked = widgets::clickable(ui.interact(response.response.rect, id, Sense::click()))
        .on_hover_text(format!("Review {}", row.path_under_repo))
        .clicked();
    // Deferred: the arrangement is lent out while its panes draw, this one included, so a
    // request made here would find no panes to bring forward. The app acts on it after.
    if clicked {
        app.pending_action = Some(CommandAction::OpenPane(row.opens.clone()));
    }

    // Level with the name rather than centred on the row, so it stays put on a row that has
    // a note under its name.
    let shell_rect = egui::Rect::from_center_size(
        egui::pos2(
            // From the row's left edge and the width every row is given, rather than the row's
            // own right edge: a row holding a short name can come out narrower than the rest,
            // and the buttons would step in with it.
            response.response.rect.left() + row_width + SHELL_BUTTON_GAP + SHELL_BUTTON_SIZE / 2.0,
            name_rect.center().y,
        ),
        vec2(SHELL_BUTTON_SIZE, SHELL_BUTTON_SIZE),
    );
    let shell_hint = format!("Open a shell in {}", row.path_under_repo);
    if draw_shell_button(ui, shell_id, shell_rect, &shell_hint, palette)
        .on_hover_text(&shell_hint)
        .clicked()
    {
        app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::TerminalInRepo {
            repo_path: row.repo_path.clone(),
        }));
    }
}

/// A prompt, `>_`, as a button: quiet until the pointer is on it.
///
/// Drawn rather than typeset: a font puts `>` above its baseline and `_` below it, which
/// leaves the pair off the middle of whatever line it is set beside.
fn draw_shell_button(
    ui: &mut Ui,
    id: egui::Id,
    rect: egui::Rect,
    hint: &str,
    palette: &Palette,
) -> egui::Response {
    let response = widgets::clickable(ui.interact(rect, id, Sense::click()));
    // Painted rather than a widget with text, so what it is has to be said for it: the hint
    // is its name to a screen reader, and to a test looking for it.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hint));
    if ui.is_rect_visible(rect) {
        let ink = if response.hovered() {
            palette.ink
        } else {
            palette.muted
        };
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(3), palette.control_bg);
        }
        let stroke = Stroke::new(1.2, ink);
        let center = rect.center();
        let half = SHELL_BUTTON_SIZE * 0.2;
        // The chevron on the left half, the underscore on the right, both over the same
        // height so the mark as a whole is centred on the box.
        let chevron_x = center.x - half * 1.1;
        ui.painter().line(
            vec![
                egui::pos2(chevron_x - half * 0.5, center.y - half),
                egui::pos2(chevron_x + half * 0.5, center.y),
                egui::pos2(chevron_x - half * 0.5, center.y + half),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x + half * 0.2, center.y + half),
                egui::pos2(center.x + half * 1.6, center.y + half),
            ],
            stroke,
        );
    }
    response
}

fn path_under_repo<'a>(submodule_path: &'a str, root_repo_path: Option<&str>) -> &'a str {
    let Some(root) = root_repo_path else {
        return submodule_path;
    };
    submodule_path
        .strip_prefix(root)
        .map(|rest| rest.trim_start_matches('/'))
        .unwrap_or(submodule_path)
}
