//! Notes box - the box a task's notes are written in, on the task's own pane and on the pane a
//! new task is written on, and what the find bar does to it while it is over that pane.
//!
//! With no bar over the pane it is a plain `TextEdit`. With one, the same box lays its text
//! out with the matches marked in it, the way an open file marks them, and the box takes the
//! keyboard back when the bar is put away. Marking a text and putting the caret on a mark are
//! [`egui_moon_editor`]'s, which is where both are written for the editor a file is open in.

use std::ops::Range;

use egui::{Ui, text_edit::TextEditOutput};
use egui_frames::PaneId;
use egui_moon_editor::Marks;

use crate::native::{
    find::{Find, TextSearch},
    theme::Palette,
};

/// How many lines of notes the box stands open at. Enough for a paragraph, and it grows with
/// what is written into it.
const NOTES_ROWS: usize = 8;

/// What a notes box drawn under the find bar leaves for the frame after, in egui's memory
/// under the pane's name. The bar is put away after the panes have drawn, so a pane only ever
/// sees it gone on the frame after - by which time the search it was running is gone with it.
#[derive(Clone)]
struct UnderTheBar {
    /// The frame the notes were drawn under the bar on.
    frame: u64,
    /// The match the search was on, as a character range of the notes. `None` while the query
    /// is not in them.
    stopped_on: Option<Range<usize>>,
}

/// Draw the notes box of the pane `pane_id`, searched by `find` while that is over the pane,
/// and hand back what the box said about itself.
///
/// The box has no id of its own, so it must stay the first thing drawn after whatever stands
/// above it whether or not it is being searched: the caret a search leaves on a match is kept
/// under the same id the typing is.
pub(super) fn show(
    ui: &mut Ui,
    pane_id: PaneId,
    notes: &mut String,
    hint: Option<&'static str>,
    find: &mut Option<Find>,
    palette: &Palette,
) -> TextEditOutput {
    let under_the_bar = egui::Id::new(("notes-box-under-find-bar", pane_id));
    let frame = ui.ctx().cumulative_frame_nr();
    // Looked for here, before the box borrows the notes to write into them.
    let searched = TextSearch::over(find, pane_id).map(|search| {
        let found = egui_moon_editor::matches_in(notes, &search.query);
        (search, found)
    });

    let mut notes_box = egui::TextEdit::multiline(notes)
        .desired_width(f32::INFINITY)
        .desired_rows(NOTES_ROWS)
        .margin(egui::Margin::symmetric(6, 4));
    if let Some(hint) = hint {
        notes_box = notes_box.hint_text(hint);
    }

    let Some((search, found)) = searched else {
        let mut written = notes_box.show(ui);
        // The bar that was over these notes on the frame before has been put away, by Escape
        // or its own `×`: no bar is left anywhere. One that ⌘F took to another pane has not
        // been - the keyboard is in it, over there.
        let left: Option<UnderTheBar> = ui.data(|data| data.get_temp(under_the_bar));
        let put_away = left.filter(|left| find.is_none() && frame - left.frame <= 1);
        // The bar held the keyboard over these notes, so the notes are where it goes back to.
        // Not when something has the keyboard already - the title box of a task just made asks
        // for it above, and a shell in the next split keeps what it has.
        if let Some(left) = put_away
            && ui.memory(|memory| memory.focused().is_none())
        {
            written.response.request_focus();
            // With the match the search stopped on selected, so the eye finds where it landed
            // and the next letter is about it. Selected again here rather than left as the
            // search selected it: a `TextEdit` without the keyboard folds its selection down
            // to the caret on every frame it is drawn, and this box had none while the bar did.
            if let Some(stopped_on) = &left.stopped_on {
                let the_match = Marks {
                    ranges: std::slice::from_ref(stopped_on),
                    current: 0,
                    select_current: true,
                };
                egui_moon_editor::select_current_mark(ui, &the_match, &mut written);
            }
        }
        return written;
    };

    // The matches are marks laid into the text rather than the box's selection: the bar holds
    // the keyboard while it is open, and a box without it paints no selection at all.
    let marks = search.marks(&found);
    let style = palette.editor_style();
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        let job =
            egui_moon_editor::marked_plain_text(ui, text.as_str(), &marks, &style, wrap_width);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let mut written = notes_box.layouter(&mut layouter).show(ui);
    // Asked of this `ui`, which is the pane's scroll area's: a match under the fold of a long
    // page of notes is brought up to be read.
    egui_moon_editor::select_current_mark(ui, &marks, &mut written);
    ui.data_mut(|data| {
        data.insert_temp(
            under_the_bar,
            UnderTheBar {
                frame,
                stopped_on: marks.ranges.get(marks.current).cloned(),
            },
        );
    });
    find.as_mut()
        .expect("a search was read out of the bar, so the bar is open")
        .found(found.len());
    written
}
