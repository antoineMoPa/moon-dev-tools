//! What a card is marked with: the tags at the foot of the card, and the box they are edited
//! in.
//!
//! A tag is the person's word for a card - `bug`, `needs-tests`, a client's name - and it is
//! kept in one spelling by the store (see [`crate::moontasks::store::tag_of`]), so a tag typed
//! twice lands as one. The board's filter looks through them the way it looks through a title,
//! since a tag is on the card to be found by.
//!
//! The box is the one a mail client puts addresses in: what the card already has stands in it
//! as a pill apiece, each with the mark that takes it off, and the next one is typed at the
//! end of them. Enter or a space puts the word typed on, so several go on in one breath, and
//! backspace with nothing typed takes the last pill off, which is what that key does in every
//! field that holds things rather than letters. Under the box are the tags the rest of the
//! board uses, because the second card to be called `bug` should be a click.
//!
//! On a card the box opens in place of the card's pills, from `[tags]`, and shuts on a press
//! anywhere off the card. On the task's pane it stands open.

use egui::{CornerRadius, Rect, Response, RichText, Sense, Ui, vec2};

use crate::{
    moontasks::{TaskView, store},
    native::{
        app::App,
        board::{BoardAction, gesture::Controls},
        theme::{Palette, SMALL_SIZE, TAG_HUES},
        widgets,
    },
};

/// How wide the place a tag is typed is. About a dozen letters, which is a tag, and short
/// enough to sit on the end of a line of pills rather than pushing itself onto its own.
const ENTRY_WIDTH: f32 = 96.0;

/// The inside margin of a pill, and the gap between the tag and the mark that takes it off.
const PILL_PADDING: egui::Vec2 = vec2(5.0, 2.0);
const PILL_SPACING: f32 = 4.0;

/// The tags that are always drawn on a background of their own, whatever their letters would
/// land on. A card marked `bug` should read as broken from across the board.
const TAG_BACKGROUNDS: &[(&str, fn(&Palette) -> egui::Color32)] =
    &[("bug", |palette| palette.bug_tag_bg)];

/// Which background this tag is drawn on.
///
/// A tag of [`TAG_BACKGROUNDS`] is drawn on its own. Any other is settled by the tag's own
/// letters, added up into one byte and folded onto the palette's hues, so the same tag is the
/// same color on every card, on every board, and after every restart - with no color chosen or
/// written down anywhere. Two tags a letter apart usually come out apart too, which is what the
/// eye wants from `needs-test` and `needs-tests`.
pub(crate) fn background_of(tag: &str, palette: &Palette) -> egui::Color32 {
    if let Some((_, background)) = TAG_BACKGROUNDS.iter().find(|(named, _)| *named == tag) {
        return background(palette);
    }
    let checksum = tag.bytes().fold(0u8, |sum, byte| sum.wrapping_add(byte));
    palette.tag_bgs[usize::from(checksum) % TAG_HUES]
}

/// The tags the rest of the board uses that the box does not hold, narrowed to what is being
/// typed. What somebody is halfway through writing is usually a tag they have written before,
/// so the list under the box is the rest of that word.
fn suggestions(app: &App, held: &[String], typed: &str) -> Vec<String> {
    let typed = typed.trim().to_lowercase();
    let mut offered: Vec<String> = Vec::new();
    for tag in app
        .model
        .board
        .tasks
        .iter()
        .flat_map(|other| other.tags.iter())
    {
        if held.contains(tag) || offered.contains(tag) || !tag.contains(&typed) {
            continue;
        }
        offered.push(tag.clone());
    }
    offered
}

/// The pills at the foot of a card, one per tag. Nothing at all on a card with none: a card
/// at rest is its title and its description.
///
/// A press on any of them opens the card's box, the way `[tags]` does: a tag is pressed
/// because it is the thing about the card to be changed.
fn draw_pills(app: &mut App, ui: &mut Ui, task: &TaskView, palette: &Palette, card: &mut Controls) {
    if task.tags.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
        for tag in &task.tags {
            let (pill, _) = paint_pill(ui, tag, None, palette, Sense::click());
            pill.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tag));
            let pill = widgets::clickable(pill).on_hover_text("Edit this card's tags");
            if card.pressed(&pill) {
                open_box(app, task);
            }
        }
    });
    ui.add_space(3.0);
}

/// Open the card's tag box with the keyboard in it, shutting whichever card's box was open.
fn open_box(app: &mut App, task: &TaskView) {
    app.model.board.tagging_card = Some(task.id.clone());
    app.model
        .board
        .tagging
        .entry(task.id.clone())
        .or_default()
        .focus = true;
}

/// What a card shows of its tags: the box, while its `[tags]` has it open, and the pills
/// otherwise.
pub(crate) fn draw_on_card(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    palette: &Palette,
    card: &mut Controls,
    actions: &mut Vec<BoardAction>,
) {
    if app.model.board.tagging_card.as_deref() != Some(task.id.as_str()) {
        draw_pills(app, ui, task, palette, card);
        return;
    }
    draw_field(app, ui, task, palette, card, actions);
    ui.add_space(3.0);
}

/// The `[tags]` button at the foot of a card, which opens the box in the card and shuts it
/// again. Answers whether the box is open, which keeps the card's offers out for as long as
/// it is.
pub(crate) fn draw_button(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    card: &mut Controls,
) -> bool {
    let open = app.model.board.tagging_card.as_deref() == Some(task.id.as_str());
    let button = widgets::clickable(ui.add(egui::Button::new("[tags]").frame(false)))
        .on_hover_text(
            "What this card is marked with. The board's filter finds a card by its tags too",
        );
    if card.pressed(&button) {
        if open {
            app.model.board.tagging_card = None;
        } else {
            open_box(app, task);
        }
    }
    app.model.board.tagging_card.as_deref() == Some(task.id.as_str())
}

/// Shut the card's box on a press that lands off the card - the way a menu is shut, since
/// the box stands in for one. `card` is where the card was drawn this frame.
pub(crate) fn shut_on_press_elsewhere(app: &mut App, ui: &Ui, task: &TaskView, card: Rect) {
    if app.model.board.tagging_card.as_deref() != Some(task.id.as_str()) {
        return;
    }
    let pressed_elsewhere = ui.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .press_origin()
                .is_some_and(|origin| !card.contains(origin))
    });
    if pressed_elsewhere {
        app.model.board.tagging_card = None;
    }
}

/// The box the card's tags are edited in, and the board's own tags under it.
///
/// It takes the keyboard only when its composer says to - on the frame a card's box opened,
/// and after each tag put on or taken off: the pane's box stands there all the time and must
/// not take the keyboard off whatever the person is really typing in.
pub(crate) fn draw_field(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    palette: &Palette,
    controls: &mut Controls,
    actions: &mut Vec<BoardAction>,
) {
    let composer = app.model.board.tagging.entry(task.id.clone()).or_default();
    let held = composer.tags_over(&task.tags);
    let offered = suggestions(app, &held, &app.model.board.tagging[&task.id].text);

    // What the card should end up marked with, if this frame changed it at all. The whole
    // list every time: the box knows what it wants the card to say, and sending that is one
    // write rather than a diff.
    let mut wanted: Option<Vec<String>> = None;

    let entry_id = ui.id().with(("tag-entry", &task.id));
    let focused = ui.memory(|memory| memory.has_focus(entry_id));
    // Both taken before the box is drawn, so the box never sees them. Enter would otherwise
    // take the keyboard out of a box that is about to be typed the next tag into; backspace
    // with nothing typed takes the last tag off rather than deleting a letter that is not
    // there.
    let entered =
        focused && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let backspaced = focused
        && app.model.board.tagging[&task.id].text.is_empty()
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace));
    if backspaced && !held.is_empty() {
        let mut tags = held.clone();
        tags.pop();
        wanted = Some(tags);
    }

    // The whole box answers a press, so a press between the pills puts the keyboard in it
    // rather than landing on the card behind.
    let field = ui.scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
        egui::Frame::new()
            .fill(palette.composer_bg)
            .stroke(egui::Stroke::new(1.0, palette.line))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::symmetric(5, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                    for tag in &held {
                        if controls.pressed(&draw_removable_pill(ui, tag, palette)) {
                            let mut tags = held.clone();
                            tags.retain(|kept| kept != tag);
                            wanted = Some(tags);
                        }
                    }

                    let composer = app
                        .model
                        .board
                        .tagging
                        .get_mut(&task.id)
                        .expect("the composer was made at the top of the box");
                    let entry = ui.add(
                        egui::TextEdit::singleline(&mut composer.text)
                            .id(entry_id)
                            // Nothing to say once there are pills beside it: what the box is
                            // for is being shown rather than described.
                            .hint_text(if held.is_empty() { "add a tag" } else { "" })
                            .desired_width(ENTRY_WIDTH)
                            // No frame of its own: the box around the whole row is the field,
                            // and a second outline inside it would read as a box in a box.
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::symmetric(2, 2)),
                    );
                    controls.pressed(&entry);
                    if std::mem::take(&mut composer.focus) {
                        entry.request_focus();
                    }

                    // Every word finished - by a space after it, or by Enter - goes on, and
                    // the one still being typed stays in the box. Words rather than keys, so
                    // a line of tags pasted in goes on whole.
                    let finished = entered || composer.text.ends_with(char::is_whitespace);
                    let mut words: Vec<String> = composer
                        .text
                        .split_whitespace()
                        .map(str::to_string)
                        .collect();
                    let unfinished = if finished { None } else { words.pop() };
                    if !words.is_empty() {
                        composer.text = unfinished.unwrap_or_default();
                        wanted = Some(store::tags_of(
                            held.iter().chain(words.iter()).map(String::as_str),
                        ));
                    } else if finished {
                        composer.text.clear();
                    }

                    // Escape empties the box, and shuts a card's box that was already empty.
                    if entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape))
                    {
                        if composer.text.is_empty() {
                            if app.model.board.tagging_card.as_deref() == Some(task.id.as_str()) {
                                app.model.board.tagging_card = None;
                            }
                        } else {
                            composer.text.clear();
                        }
                    }
                });
            });
    });
    if controls.pressed(&field.response) {
        ui.memory_mut(|memory| memory.request_focus(entry_id));
    }

    if !offered.is_empty() {
        ui.add_space(5.0);
        ui.label(
            RichText::new("Add tags:")
                .size(SMALL_SIZE)
                .color(palette.muted),
        );
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
            for tag in offered {
                let pressed = draw_offered_pill(ui, &tag, palette)
                    .on_hover_text(format!("Mark this card {tag}"));
                if controls.pressed(&pressed) {
                    let mut tags = held.clone();
                    tags.push(tag);
                    wanted = Some(tags);
                }
            }
        });
    }

    let Some(tags) = wanted else {
        return;
    };
    // The keyboard goes back into the box after every press, whichever of them it was: a
    // pill pressed off, a tag pressed on, a tag entered. Tags are put on a card several at a
    // time, and the press before is no reason to reach for the box again.
    let composer = app
        .model
        .board
        .tagging
        .get_mut(&task.id)
        .expect("the composer was made at the top of the box");
    composer.focus = true;
    if tags == held {
        return;
    }
    composer.sent = Some(tags.clone());
    actions.push(BoardAction::SetTags(task.id.clone(), tags));
}

/// A pill as the box draws it: the tag on its background, and after it `mark` if it has one,
/// painted rather than laid out so every pill's words sit on the same line whatever letters
/// are in them. Answers the pill and, if it has a mark, where that was drawn.
fn paint_pill(
    ui: &mut Ui,
    tag: &str,
    mark: Option<&str>,
    palette: &Palette,
    sense: Sense,
) -> (Response, Option<Rect>) {
    let font = egui::FontId::proportional(SMALL_SIZE);
    let word = ui
        .painter()
        .layout_no_wrap(tag.to_string(), font.clone(), palette.ink);
    let mark = mark.map(|mark| {
        ui.painter()
            .layout_no_wrap(mark.to_string(), font, palette.muted)
    });
    let mark_width = mark
        .as_ref()
        .map_or(0.0, |mark| PILL_SPACING + mark.size().x);
    let size = vec2(word.size().x + mark_width, word.size().y) + PILL_PADDING * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, sense);

    let mark_rect = mark.as_ref().map(|_| {
        Rect::from_min_max(
            egui::pos2(
                rect.left() + PILL_PADDING.x + word.size().x + PILL_SPACING / 2.0,
                rect.top(),
            ),
            rect.max,
        )
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(3), background_of(tag, palette));
        painter.galley(rect.min + PILL_PADDING, word, palette.ink);
        if let Some(mark) = mark {
            let at = egui::pos2(
                rect.right() - PILL_PADDING.x - mark.size().x,
                rect.top() + PILL_PADDING.y,
            );
            painter.galley(at, mark, palette.muted);
        }
    }
    (response, mark_rect)
}

/// One tag in the box: the word, and the mark that takes it off. Answers the mark.
fn draw_removable_pill(ui: &mut Ui, tag: &str, palette: &Palette) -> Response {
    let (pill, mark) = paint_pill(ui, tag, Some("×"), palette, Sense::hover());
    pill.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, tag));
    let mark = mark.expect("the pill was painted with a mark");
    let mark = ui.interact(mark, pill.id.with("take-off"), Sense::click());
    mark.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "×"));
    widgets::clickable(mark).on_hover_text(format!("Take {tag} off this card"))
}

/// One of the board's tags under the box, pressed to put it on.
fn draw_offered_pill(ui: &mut Ui, tag: &str, palette: &Palette) -> Response {
    let (pill, _) = paint_pill(ui, tag, None, palette, Sense::click());
    pill.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tag));
    widgets::clickable(pill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::theme::ThemeMode;

    /// The same tag is the same color wherever and whenever it is drawn, and the color is one
    /// the palette has - which is all that can be asked of a color nobody chose.
    #[test]
    fn a_tag_is_always_drawn_on_the_same_background() {
        let palette = Palette::of(ThemeMode::Dark);

        let once = background_of("needs-tests", &palette);
        let again = background_of("needs-tests", &palette);
        assert_eq!(once, again);
        assert!(palette.tag_bgs.contains(&once));
    }

    /// Adding a letter moves the checksum, so a tag and its plural come out apart.
    #[test]
    fn tags_a_letter_apart_are_told_apart() {
        let palette = Palette::of(ThemeMode::Dark);

        assert_ne!(
            background_of("needs-test", &palette),
            background_of("needs-tests", &palette)
        );
    }

    /// `bug` is red in both themes, not whichever hue its letters would have picked.
    #[test]
    fn a_bug_is_always_drawn_in_red() {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let palette = Palette::of(mode);

            assert_eq!(background_of("bug", &palette), palette.bug_tag_bg);
        }
    }
}
