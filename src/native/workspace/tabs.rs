//! The tab in front: which pane the keyboard is talking to, the chords that raise tabs, and
//! closing one.

use egui_frames::PaneId;

use crate::native::{
    app::App,
    bindings,
    panes::{Pane, PaneKind},
};

impl App {
    pub(crate) fn close_pane(&mut self, pane_id: PaneId) {
        let was_editing = self.model.file_editors.remove(&pane_id);
        let closed = self.model.layout.close_pane(pane_id);

        // The language server hears the file close, if this tab was the last one on it - the
        // editor is out of the map already, so what is left in it is the tabs still showing
        // the file. A file open in two tabs stays open in the server for the other one.
        if let (Some(editing), Some(Pane::File { session_id, .. })) = (&was_editing, &closed) {
            let session_id = session_id.clone();
            self.close_document(editing, &session_id);
        }

        // A task's pane takes its boxes with it, writing whatever was typed into the notes and
        // not yet written: the tab closing is the last chance those words get.
        if let Some(Pane::Start { task_id, .. }) = &closed
            && let Some(editor) = self.model.board.task_editors.remove(task_id)
            && editor.notes_typed_at.is_some()
        {
            crate::native::board::actions::apply(
                self,
                crate::native::board::actions::BoardAction::SaveNotes {
                    task_id: task_id.clone(),
                    notes: editor.notes,
                },
            );
        }

        // A new-task pane takes its writing with it and makes nothing: `[create]` is what makes
        // a task, and closing the tab without pressing it is saying no to the task. The empty
        // card it was standing for goes off the board with it.
        if let Some(Pane::NewTask { draft_id, .. }) = &closed {
            self.model.board.drafts.remove(draft_id);
            self.model.board.card_being_written = None;
        }

        // Closing a shell's tab ends the shell: the tab is the only window it had.
        //
        // A task's shell is the exception. It belongs to the task rather than to the tab, and
        // keeps running with nothing attached until the task reaches DONE, so the user can come
        // back to the agent they closed.
        if let Some(Pane::Terminal {
            terminal_id,
            task_id,
            ..
        }) = closed
        {
            // Closing the build shell by hand is calling its restart off.
            if self.model.restart_on_shell_exit.as_deref() == Some(terminal_id.as_str()) {
                self.model.restart_on_shell_exit = None;
            }
            self.terminals.remove(&terminal_id);
            self.model.terminal_names.remove(&terminal_id);
            self.terminal_errors.remove(&terminal_id);
            if self
                .model
                .renaming_tab
                .as_ref()
                .is_some_and(|rename| rename.pane_id == pane_id)
            {
                self.model.renaming_tab = None;
            }
            if task_id.is_some() {
                return;
            }
            let session_id = self.model.root_session_id.clone();
            self.tasks.spawn(
                move |backend| backend.close_terminal(&session_id, &terminal_id),
                |model, result| model.report(result, "could not close the shell"),
            );
        }
    }

    /// The pane in front of the active frame, which is what the keyboard is talking to.
    pub(crate) fn active_pane(&self) -> Option<(PaneId, &Pane)> {
        self.model.layout.active_pane()
    }

    pub(crate) fn active_pane_kind(&self) -> Option<PaneKind> {
        self.active_pane().map(|(_, pane)| pane.kind())
    }

    pub(crate) fn active_pane_id(&self) -> Option<PaneId> {
        self.active_pane().map(|(pane_id, _)| pane_id)
    }

    /// Where a pane was drawn this frame: the body of the frame holding it, below the tabs.
    /// Anything that floats over a pane - the find bar - is placed against this.
    pub(crate) fn pane_rect(&self, pane_id: PaneId) -> Option<egui::Rect> {
        self.frames.pane_rect(pane_id)
    }

    /// Let the board's mark follow the tab in front, before the frame is drawn.
    ///
    /// A task's own tab coming forward - its page, a shell started in it, a file opened off its
    /// card - is that task being worked in, and the board marks its card for it: the same mark
    /// a click on the card makes, because it means the same thing.
    ///
    /// Only when the tab in front changes, so the marks made on the board afterwards are not
    /// undone frame by frame by the tab that opened the first of them.
    ///
    /// The task is read off the pane here rather than by the board, because the arrangement is
    /// lent out to the workspace widget for the length of the draw - see [`Self::draw_workspace`]
    /// - and the board could not ask it anything while it is out.
    pub(super) fn follow_task_in_front(&mut self) {
        let front = self.active_pane_id();
        if front == self.front_pane {
            return;
        }
        self.front_pane = front;

        // Something that is nobody's task came forward - the board itself, a review. The marks
        // are left alone: they are read on the board, and clicking onto the board to look at
        // them would otherwise be what took them off.
        let Some(task_id) = front
            .and_then(|pane| self.model.layout.pane(pane))
            .and_then(Pane::task_id)
            .map(str::to_string)
        else {
            return;
        };
        crate::native::board::selection::mark_only(&mut self.model.board, task_id);
    }

    /// The review in the frontmost pane of the active frame, if that pane is a review.
    pub(crate) fn focused_review_session(&self) -> Option<String> {
        match self.active_pane()?.1 {
            Pane::Review { session_id, .. } => Some(session_id.clone()),
            _ => None,
        }
    }

    /// The review a command aimed at "this review" means: the one the pane in front belongs to,
    /// whether that is a review, a file of it, or its own commit pane. Else the review the
    /// window was launched on.
    ///
    /// A window can have several reviews open at once: every changed submodule is a review of
    /// its own repo, with its own branch to commit and push. Committing while reading one of
    /// them is committing that repo, not the one the window was launched on.
    pub(crate) fn review_in_front(&self) -> String {
        match self.active_pane().map(|(_, pane)| pane) {
            Some(
                Pane::Review { session_id, .. }
                | Pane::File { session_id, .. }
                | Pane::Commit { session_id },
            ) => session_id.clone(),
            _ => self.model.root_session_id.clone(),
        }
    }

    /// `C-x o`: hand the keyboard to the next frame of the workspace, wrapping round at the end.
    pub(crate) fn focus_next_frame(&mut self) {
        if self.model.layout.frame_count() < 2 {
            return;
        }
        self.model.layout.focus_next_frame();
    }

    /// cmd+1 through cmd+9: bring the active frame's nth tab to the front. A digit past the
    /// last tab does nothing.
    pub(crate) fn select_tab(&mut self, index: usize) {
        let frame = self.model.layout.active_frame();
        let Some(pane) = self
            .model
            .layout
            .frame(frame)
            .and_then(|open| open.panes().get(index).copied())
        else {
            return;
        };
        self.model.layout.focus_pane(pane);
    }

    /// The chords that raise tabs reach the active frame, so its tabs and only its tabs wear
    /// them at the right of their titles. A frame with a single tab wears none: cmd+1 there
    /// would change nothing worth signposting.
    pub(super) fn stamp_tab_shortcuts(&mut self) {
        self.tab_shortcuts.clear();
        let frame = self.model.layout.active_frame();
        let Some(open) = self.model.layout.frame(frame) else {
            return;
        };
        if open.panes().len() < 2 {
            return;
        }
        for (index, pane) in open.panes().iter().take(9).enumerate() {
            if let Some(label) = bindings::tab_shortcut_label(index) {
                self.tab_shortcuts.insert(*pane, label);
            }
        }
    }

    /// The tab in front of the active frame is the one being worked in, so it is the one with
    /// the keyboard - whatever put it there: a click on its tab or in its frame, cmd+1, `C-x o`,
    /// a tab dragged across, a pane opened or closed. Watching the arrangement rather than
    /// each of those means none of them can be the one that forgets.
    ///
    /// The keyboard itself moves in two steps, because egui's focus can only be taken by the
    /// widget that would hold it: the pane left behind lets go here, and the pane arriving takes
    /// it while it draws - in [`App::draw_terminal`] for a shell, and in `file_pane::drawing::draw_editor`
    /// for a file or a task's notes.
    pub(super) fn follow_front_tab(&mut self, ctx: &egui::Context) {
        let front = self.active_pane_id();
        if front == self.keyboard_pane {
            return;
        }
        self.keyboard_pane = front;
        // A widget drawn in the arriving pane is not being left behind: the click that brought
        // the pane forward may be the very one that put the keyboard in that widget - the third
        // click of a triple on a card's title, say, with the rename box the first two opened
        // already holding it. The keyboard is where it was reached for, so it stays.
        if let Some(focused) = ctx.memory(|memory| memory.focused()) {
            let in_front_pane = front
                .and_then(|pane| self.frames.pane_rect(pane))
                .zip(ctx.read_response(focused))
                .is_some_and(|(pane, widget)| pane.intersects(widget.rect));
            if in_front_pane {
                return;
            }
            // A shell that keeps the keyboard keeps every key sent to the window, and a pane
            // with nothing to type into never takes it off it, so letting go is not the
            // arriving pane's job to do.
            ctx.memory_mut(|memory| memory.surrender_focus(focused));
        }
        self.pane_taking_keyboard = front;
    }
}
