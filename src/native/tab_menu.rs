//! Tab menu - lists every tab of the window behind the hamburger button of a phone's tab strip,
//! the most recently shown first, so the tab just left - the board, an agent's shell - is the
//! first row.

use crate::native::{app::App, menu::bar::TabEntry};

impl App {
    /// Keep `tabs_most_recent_first` true to the layout: the tab in front of the frame the
    /// keyboard is in goes to the head, tabs that have closed go, and tabs that have not been
    /// in front since they opened are kept at the tail, in the order they were found.
    pub(crate) fn remember_tab_order(&mut self) {
        let layout = &self.model.layout;
        self.tabs_most_recent_first.retain(|pane| layout.contains(*pane));
        for (pane_id, _) in layout.panes() {
            if !self.tabs_most_recent_first.contains(&pane_id) {
                self.tabs_most_recent_first.push(pane_id);
            }
        }
        if let Some((front, _)) = layout.active_pane() {
            self.tabs_most_recent_first.retain(|pane| *pane != front);
            self.tabs_most_recent_first.insert(0, front);
        }
    }

    pub(crate) fn tab_menu_entries(&mut self) -> Vec<TabEntry> {
        let front = self.model.layout.active_pane().map(|(pane_id, _)| pane_id);
        let mut entries = Vec::new();
        for pane_id in self.tabs_most_recent_first.clone() {
            let pane = self
                .model
                .layout
                .pane(pane_id)
                .expect("the order is kept to the open tabs")
                .clone();
            let title = egui_frames::PaneView::tab(self, pane_id, &pane).title;
            entries.push(TabEntry {
                pane_id,
                title,
                in_front: Some(pane_id) == front,
            });
        }
        entries
    }
}
