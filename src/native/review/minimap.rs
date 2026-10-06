//! The review's minimap: the whole review down a strip at the right of the diff, drawn by the
//! map a file tab has down its own right edge - [`egui_moon_editor::Minimap`] - so the two
//! look and answer alike: dimmed until the pointer is on it, the stretch on screen marked, a
//! press or a drag on it scrolling the diff there.
//!
//! What is the review's own is what the map is of. A file is a line to a row; a review is
//! cards of rows with headings between them and comments in among them. So the diff pane
//! reports where every card landed as it draws - see [`Minimap::card`] - and a card that is
//! really drawn reports where its rows and its comments are inside it - see
//! [`CardBeingDrawn`]. The map is folded from that layout, an added or a removed line in the
//! ink the diff draws it in and a comment in the accent, so the strip and the diff cannot
//! disagree about where a line is.

use std::{collections::HashMap, ops::Range};

use egui::{Color32, Rangef, Rect, Ui};
use egui_moon_editor::{EditorStyle, MinimapSpan};

use crate::{
    api::HunkView,
    native::{
        app::App,
        model::hash_of,
        review::{
            diff::{DiffLine, LineKind},
            hunks::row_pitch,
        },
        theme::Palette,
    },
};

/// How wide a review pane has to be to spare the strip its width.
const ROOM_FOR_THE_MINIMAP: f32 = 900.0;
/// How far two measures of the same thing may be apart and still be the same. Positions are
/// snapped to whole pixels wherever the diff happens to be scrolled to, so a card's place in
/// the review is not the same number on every frame.
const SAME_PLACE: f32 = 1.0;

/// Whether a review pane this wide draws the strip beside its diff.
pub(crate) fn fits_beside(pane_width: f32) -> bool {
    pane_width >= ROOM_FOR_THE_MINIMAP
}

/// The strip's widget id. Derived from the review rather than from the enclosing `Ui`, which
/// is what lets the tests find the strip and press it.
pub(crate) fn strip_id(session_id: &str) -> egui::Id {
    egui::Id::new(("moonreview-minimap", session_id))
}

/// Where the diff's scroll area was on its last draw.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Scrolled {
    /// How far down the review the top of the pane is.
    pub(crate) offset: f32,
    /// How tall the whole review is laid out.
    pub(crate) content_height: f32,
    /// How much of it the pane shows at once.
    pub(crate) view_height: f32,
}

/// One hunk's card in the review as it was last laid out, measured from the top of the
/// review rather than of the window.
#[derive(Clone, Copy, Debug)]
struct Card {
    /// [`hash_of`] the hunk's id.
    hunk: u64,
    top: f32,
    height: f32,
}

fn same_cards(one: &[Card], other: &[Card]) -> bool {
    one.len() == other.len()
        && one.iter().zip(other).all(|(one, other)| {
            one.hunk == other.hunk
                && (one.top - other.top).abs() < SAME_PLACE
                && (one.height - other.height).abs() < SAME_PLACE
        })
}

/// A run of a hunk's lines drawn one under the other with nothing between them.
#[derive(Clone, Debug)]
struct Stretch {
    /// Where the first of them starts, from the top of the card.
    top: f32,
    /// From the top of one row to the top of the next.
    pitch: f32,
    /// Which lines, indexed the way the hunk's parsed lines are.
    lines: Range<usize>,
}

/// Where a card's rows and its comments are inside it, measured from the card's top - what
/// only drawing the card can say, and what stays true of it while it is scrolled out of
/// sight and skipped.
#[derive(Clone, Default, Debug)]
pub(crate) struct CardShape {
    stretches: Vec<Stretch>,
    /// The comments and the composers between the rows, each as its top and its bottom.
    notes: Vec<Rangef>,
}

impl CardShape {
    /// Whether this is the shape the card already had, give or take the pixel a row moves
    /// by as the diff scrolls.
    pub(crate) fn reads_as(&self, other: &Self) -> bool {
        self.stretches.len() == other.stretches.len()
            && self.notes.len() == other.notes.len()
            && self
                .stretches
                .iter()
                .zip(&other.stretches)
                .all(|(one, other)| {
                    one.lines == other.lines && (one.top - other.top).abs() < SAME_PLACE
                })
            && self.notes.iter().zip(&other.notes).all(|(one, other)| {
                (one.min - other.min).abs() < SAME_PLACE && (one.max - other.max).abs() < SAME_PLACE
            })
    }
}

/// A card while it is being drawn: where on the screen it starts, and the shape it reports
/// as its rows go down.
pub(crate) struct CardBeingDrawn {
    top: f32,
    shape: CardShape,
}

impl CardBeingDrawn {
    /// A card about to be drawn with its top at `top`, a place on the screen.
    pub(crate) fn at(top: f32) -> Self {
        Self {
            top,
            shape: CardShape::default(),
        }
    }

    /// The row of line `index` is about to be drawn at `top`, a place on the screen, and
    /// the row under it would start `pitch` further down.
    pub(crate) fn line(&mut self, index: usize, top: f32, pitch: f32) {
        let top = top - self.top;
        if let Some(stretch) = self.shape.stretches.last_mut()
            && stretch.lines.end == index
            && (stretch.top + stretch.lines.len() as f32 * stretch.pitch - top).abs() < SAME_PLACE
        {
            stretch.lines.end += 1;
            return;
        }
        self.shape.stretches.push(Stretch {
            top,
            pitch,
            lines: index..index + 1,
        });
    }

    /// A comment or a composer was drawn between two places on the screen.
    pub(crate) fn note(&mut self, top: f32, bottom: f32) {
        self.shape
            .notes
            .push(Rangef::new(top - self.top, bottom - self.top));
    }

    pub(crate) fn shape(self) -> CardShape {
        self.shape
    }
}

/// The inks of the map's layers, in the order they are drawn: the grounds first, then the
/// lines over them, a change over the context around it.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Inks {
    card: Color32,
    removed_ground: Color32,
    added_ground: Color32,
    note: Color32,
    context: Color32,
    removed: Color32,
    added: Color32,
}

impl Inks {
    /// A card in the ground the diff draws one on, a changed line in the tint and the ink
    /// the diff gives it, a comment in the accent, and context in the ink a file tab's map
    /// draws a line in.
    fn of(palette: &Palette, style: &EditorStyle) -> Self {
        Self {
            card: palette.code_bg,
            removed_ground: palette.diff_removed_bg,
            added_ground: palette.diff_added_bg,
            note: palette.accent,
            context: style.minimap_ink,
            removed: palette.removed,
            added: palette.added,
        }
    }
}

/// What the map is folded from, a layer to a field: where everything of the review is, from
/// the review's top.
#[derive(Default)]
struct Layers {
    cards: Vec<Range<f32>>,
    removed_grounds: Vec<Range<f32>>,
    added_grounds: Vec<Range<f32>>,
    notes: Vec<Range<f32>>,
    context: Vec<(f32, MinimapSpan)>,
    removed: Vec<(f32, MinimapSpan)>,
    added: Vec<(f32, MinimapSpan)>,
}

impl Layers {
    /// Add one card: its ground, then its lines and its comments where the card's shape
    /// says they are. `lines` are the hunk's parsed lines, the ones the shape's stretches
    /// index, and `max_chars` how many characters of one the map has room for.
    fn card(&mut self, card: &Card, shape: &CardShape, lines: &[DiffLine], max_chars: usize) {
        self.cards.push(card.top..card.top + card.height);
        self.notes.extend(
            shape
                .notes
                .iter()
                .map(|note| card.top + note.min..card.top + note.max),
        );
        for stretch in &shape.stretches {
            for (place, index) in stretch.lines.clone().enumerate() {
                // The shape is what the card was last drawn as, and the lines are what the
                // hunk was last parsed as: a hunk whose patch changed while it was out of
                // sight has new lines under an old shape until it is drawn again.
                let Some(line) = lines.get(index) else {
                    continue;
                };
                let top = card.top + stretch.top + place as f32 * stretch.pitch;
                let (inked, ground) = match line.kind {
                    LineKind::Added => (&mut self.added, Some(&mut self.added_grounds)),
                    LineKind::Removed => (&mut self.removed, Some(&mut self.removed_grounds)),
                    LineKind::Context | LineKind::Header | LineKind::Other => {
                        (&mut self.context, None)
                    }
                };
                inked.push((top, MinimapSpan::of(line.body(), max_chars)));
                if let Some(ground) = ground {
                    ground.push(top..top + stretch.pitch);
                }
            }
        }
    }
}

/// The strip of one review: the map, and the layout the diff pane reported on its last draw
/// that the map is folded from.
#[derive(Default)]
pub(crate) struct Minimap {
    map: egui_moon_editor::Minimap,
    /// How far a press on the strip asks the diff to be scrolled, until the diff has been -
    /// see [`take_press`].
    pub(crate) asked: Option<f32>,
    pub(crate) scrolled: Scrolled,
    /// Where on the screen the top of the review is on the draw now reporting its cards.
    content_top: f32,
    /// Every card of the review that is not folded away, top to bottom.
    cards: Vec<Card>,
    /// What the map was last folded from, which is what says when to fold it again.
    folded_cards: Vec<Card>,
    folded_inks: Inks,
    folded_hunks_drawn_differently: u64,
    /// How many times the map has been folded. A review of a thousand hunks is tens of
    /// thousands of lines to go through, which is work for when the layout changes and not
    /// for every frame of a scroll - and this is what a test of that counts.
    #[cfg(test)]
    pub(crate) times_folded: usize,
}

impl Minimap {
    /// The diff is about to report its cards, with the top of the review at `content_top`
    /// on the screen.
    pub(crate) fn start(&mut self, content_top: f32) {
        self.content_top = content_top;
        self.cards.clear();
    }

    /// A hunk's card takes `card` of the screen on this draw, drawn or skipped.
    pub(crate) fn card(&mut self, hunk_id: &str, card: Rect) {
        self.cards.push(Card {
            hunk: hash_of(hunk_id),
            top: card.top() - self.content_top,
            height: card.height(),
        });
    }

    /// Fold the map again, from the cards the diff just reported.
    fn fold(&mut self, app: &App, session_id: &str, strip: Rect, row_pitch: f32, inks: Inks) {
        let hunks: HashMap<u64, &HunkView> = app
            .model
            .review_ref(session_id)
            .map(|review| review.hunks())
            .unwrap_or_default()
            .iter()
            .map(|hunk| (hash_of(&hunk.id), hunk))
            .collect();

        let mut layers = Layers::default();
        let max_chars = egui_moon_editor::Minimap::columns(strip.width());
        for card in &self.cards {
            let hunk = hunks
                .get(&card.hunk)
                .expect("the diff reports a card for a hunk of the review it is drawing");
            let shape = app
                .hunk_shapes
                .get(&hunk.id)
                .expect("a card that was laid out was drawn once, which is what shaped it");
            // An image's card has no lines to it, only the card itself.
            let lines = app.built_diff_lines(&hunk.id).unwrap_or_default();
            layers.card(card, shape, lines, max_chars);
        }

        let mut fold = self
            .map
            .fold(self.scrolled.content_height, row_pitch, strip.height());
        fold.bands(inks.card, layers.cards);
        fold.bands(inks.removed_ground, layers.removed_grounds);
        fold.bands(inks.added_ground, layers.added_grounds);
        fold.bands(inks.note, layers.notes);
        fold.lines(inks.context, layers.context);
        fold.lines(inks.removed, layers.removed);
        fold.lines(inks.added, layers.added);

        self.folded_cards.clone_from(&self.cards);
        self.folded_inks = inks;
        self.folded_hunks_drawn_differently = app.hunks_drawn_differently;
        #[cfg(test)]
        {
            self.times_folded += 1;
        }
    }
}

/// Take a press or a drag on the strip, in the panel the strip has to itself, before the
/// diff is drawn: what it asks for is where the diff then scrolls to, on this same frame.
/// Answers where the strip is, for [`paint`].
pub(crate) fn take_press(app: &mut App, ui: &Ui, session_id: &str) -> Rect {
    let strip = ui.max_rect();
    let minimap = &mut app.model.review(session_id).minimap;
    minimap.asked = minimap.map.scroll_asked(ui, strip_id(session_id), strip);
    strip
}

/// Paint the strip, after the diff has been drawn and has reported where everything is.
/// `style` is a file tab's, whose map this is drawn as.
pub(crate) fn paint(
    app: &mut App,
    ui: &Ui,
    session_id: &str,
    strip: Rect,
    style: &EditorStyle,
    palette: &Palette,
) {
    // Taken out while it is folded: that reads the app's own caches, which cannot be read
    // while the review that holds this is borrowed to change it.
    let mut minimap = std::mem::take(&mut app.model.review(session_id).minimap);
    let inks = Inks::of(palette, style);
    let scrolled = minimap.scrolled;
    if !same_cards(&minimap.folded_cards, &minimap.cards)
        || minimap.folded_inks != inks
        || minimap.folded_hunks_drawn_differently != app.hunks_drawn_differently
        || !minimap
            .map
            .is_folded_for(scrolled.content_height, strip.height())
    {
        minimap.fold(app, session_id, strip, row_pitch(ui), inks);
    }
    minimap.map.paint(
        ui,
        strip_id(session_id),
        strip,
        style.minimap_slider_ink,
        scrolled.offset,
        scrolled.view_height,
    );
    app.model.review(session_id).minimap = minimap;
}

#[cfg(test)]
#[path = "minimap_tests.rs"]
mod tests;
