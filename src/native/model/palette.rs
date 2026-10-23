//! What the window holds of the command palette: the mode it is in, what was typed, and the
//! searches it has running.

/// The command palette, and the query typed into it.
pub(crate) struct PaletteState {
    /// The query line and the list under it: what is typed, the row the keyboard is on, and
    /// where the box drew - see `egui_command_palette`.
    pub(crate) input: egui_command_palette::CommandPalette,
    /// Whether the query is picking a command or naming a file of the repo.
    pub(crate) mode: crate::native::palette::PaletteMode,
    /// What the file finder has found for the query it last searched for.
    pub(crate) files: crate::native::palette::Search<String>,
    /// The same for the content search: the lines of the repo that hold what was typed.
    pub(crate) contents: crate::native::palette::Search<crate::api::ContentMatch>,
    /// Which files both searches read, as the "include gitignored" box under the query has
    /// it. Kept from one opening to the next: a repo that gitignores its submodules wants
    /// them read every time.
    pub(crate) search_scope: crate::api::SearchScope,
    /// The review whose repo both searches read, and whose session the file they open is
    /// opened in: the one in front when the palette was opened - see `App::review_in_front`.
    /// A search started from a file of a submodule is a search of that submodule, not of
    /// the repo the window was launched on around it.
    pub(crate) search_session_id: String,
    /// The ticket of the search started last. A running search holds the ticket it was
    /// started on and reads this from its thread: once it is no longer the latest - a key
    /// typed since, the palette put away - nobody wants what it finds, and it stops.
    pub(crate) latest_search: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// The task the file finder is picking a file for, while it is: the file chosen is put on
    /// that task's card and then opened, rather than only opened. `None` is the plain finder.
    pub(crate) files_link_to_task: Option<String>,
    /// The places the palette is listing, while it is - see [`crate::native::places`].
    pub(crate) places: Option<crate::native::places::FoundPlaces>,
}

impl PaletteState {
    pub(crate) fn is_open(&self) -> bool {
        self.input.is_open()
    }

    pub(crate) fn query(&self) -> &str {
        self.input.query()
    }

    #[cfg(test)]
    pub(crate) fn highlighted(&self) -> usize {
        self.input.highlighted()
    }

    #[cfg(test)]
    pub(crate) fn rect(&self) -> Option<egui::Rect> {
        self.input.rect()
    }

    /// Open it on an empty query, at the top of the list, and drawn nowhere yet.
    pub(crate) fn show(&mut self) {
        self.input.open();
        self.mode = crate::native::palette::PaletteMode::Commands;
        // Whatever the last search found belongs to the query that is being cleared - and
        // one still running is looking for it too.
        self.stop_searches();
        self.files = crate::native::palette::Search::default();
        self.contents = crate::native::palette::Search::default();
        self.files_link_to_task = None;
        self.places = None;
    }

    /// Open it on a name to rename, selected, so what is typed replaces it - see
    /// [`crate::native::renaming`].
    pub(crate) fn show_rename(&mut self, name: &str) {
        self.show();
        self.mode = crate::native::palette::PaletteMode::Rename;
        self.input.open_on(name);
    }

    /// Open it on the code actions a language server offered, to pick one from - see
    /// [`crate::native::code_actions`], which holds the list.
    pub(crate) fn show_code_actions(&mut self) {
        self.show();
        self.mode = crate::native::palette::PaletteMode::CodeActions;
    }

    /// Open it on a list of places a language server named, to pick one from.
    pub(crate) fn show_places(&mut self, found: crate::native::places::FoundPlaces) {
        self.show();
        self.mode = crate::native::palette::PaletteMode::Places;
        self.places = Some(found);
    }

    /// The same, on the file finder: what is typed names a file of the repo the given review
    /// is of, rather than a command.
    pub(crate) fn show_files(&mut self, search_session_id: String) {
        self.show();
        self.mode = crate::native::palette::PaletteMode::Files;
        self.search_session_id = search_session_id;
    }

    /// The file finder again, picking a file for a task's card: the one chosen is linked to
    /// the task before it is opened. The board is the root repo's, so its files are found
    /// there, whatever review is in front.
    pub(crate) fn show_files_for_task(&mut self, task_id: String, root_session_id: String) {
        self.show_files(root_session_id);
        self.files_link_to_task = Some(task_id);
    }

    /// The same, on the content search: what is typed is looked for in the text of the files
    /// of the repo the given review is of.
    pub(crate) fn show_contents(&mut self, search_session_id: String) {
        self.show();
        self.mode = crate::native::palette::PaletteMode::Contents;
        self.search_session_id = search_session_id;
    }

    /// Turn the file finder into the content search, or back, keeping what is typed: the
    /// switch at the end of the line, which Tab flips too. Both lists are dropped, the one being left too - a
    /// search it still had running stops once the other starts, and a list cut short that
    /// way would pass for the whole answer on the way back. The search of the mode switched
    /// to starts on the next frame, from the query as it stands.
    pub(crate) fn switch_search(&mut self, to: crate::native::palette::PaletteMode) {
        assert!(
            matches!(
                to,
                crate::native::palette::PaletteMode::Files
                    | crate::native::palette::PaletteMode::Contents
            ),
            "the line only switches between the two searches"
        );
        self.mode = to;
        self.stop_searches();
        self.files = crate::native::palette::Search::default();
        self.contents = crate::native::palette::Search::default();
        self.input.highlight_first();
    }

    /// Put it away. The rect goes with it so the next one it draws is the one clicks are
    /// measured against, and a search still out is nobody's to wait for.
    pub(crate) fn dismiss(&mut self) {
        self.input.dismiss();
        self.stop_searches();
    }

    /// The ticket for a search about to start, which makes it the latest: whichever search
    /// was running is no longer wanted.
    pub(crate) fn next_search_ticket(&self) -> u64 {
        self.latest_search
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1
    }

    /// Leave no search wanted, without starting one.
    fn stop_searches(&self) {
        self.latest_search
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            input: egui_command_palette::CommandPalette::default(),
            mode: crate::native::palette::PaletteMode::Commands,
            files: crate::native::palette::Search::default(),
            contents: crate::native::palette::Search::default(),
            search_scope: crate::api::SearchScope::default(),
            search_session_id: String::new(),
            latest_search: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            files_link_to_task: None,
            places: None,
        }
    }
}
