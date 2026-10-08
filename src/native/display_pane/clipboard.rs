//! Clipboard results belong to this pane and this browser only. A denied asynchronous web
//! write keeps the text available in a small dialog with a fresh user action to retry.
use crate::api::display::{CLIPBOARD_LIMIT, DisplayClipboard};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct Clipboard {
    fallback: Arc<Mutex<Option<String>>>,
    error: Option<String>,
    pending: Option<(u64, web_time::Instant)>,
    sequence: u64,
}
impl Clipboard {
    pub(super) fn begin(&mut self) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.sequence += 1;
        self.pending = Some((self.sequence, web_time::Instant::now()));
        self.error = None;
        Some(self.sequence)
    }
    pub(super) fn pasted(&mut self, text: &str) -> Option<u64> {
        if text.len() > CLIPBOARD_LIMIT {
            self.error = Some("Clipboard text exceeds 64 KiB".into());
            return None;
        }
        self.begin()
    }
    pub(super) fn received(&mut self, ctx: &egui::Context, reply: DisplayClipboard) {
        if self.pending.as_ref().is_none_or(|(id, _)| *id != reply.id) {
            return;
        }
        self.pending = None;
        self.error = reply.error;
        if let Some(text) = reply.text {
            self.write(ctx, text, false);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn write(&mut self, ctx: &egui::Context, text: String, _manual: bool) {
        ctx.copy_text(text);
    }

    #[cfg(target_arch = "wasm32")]
    fn write(&mut self, ctx: &egui::Context, text: String, manual: bool) {
        use wasm_bindgen_futures::{JsFuture, spawn_local};
        if manual && copy_during_click(&text).unwrap_or(false) {
            *self.fallback.lock().expect("clipboard fallback lock") = None;
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        if !window.is_secure_context() {
            *self.fallback.lock().expect("clipboard fallback lock") = Some(text);
            return;
        }
        // Calling write_text before spawning preserves activation when retry is clicked.
        let promise = window.navigator().clipboard().write_text(&text);
        let fallback = Arc::clone(&self.fallback);
        let ctx = ctx.clone();
        spawn_local(async move {
            let failed = JsFuture::from(promise).await.is_err();
            let mut fallback = fallback.lock().expect("clipboard fallback lock");
            if failed {
                *fallback = Some(text);
            } else if fallback.as_ref() == Some(&text) {
                *fallback = None;
            }
            ctx.request_repaint();
        });
    }
    fn expire(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, started)| started.elapsed() > std::time::Duration::from_secs(4))
        {
            self.pending = None;
            self.error = Some("Desktop clipboard transfer timed out. Try again.".into());
        }
    }
    pub(super) fn draw(&mut self, ctx: &egui::Context) {
        self.expire();
        let width = (ctx.content_rect().width() - 48.0).clamp(120.0, 400.0);
        if let Some(error) = self.error.clone() {
            egui::Window::new("Desktop clipboard")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(error);
                    if ui.button("Close").clicked() {
                        self.error = None;
                    }
                });
        }
        let text = self
            .fallback
            .lock()
            .expect("clipboard fallback lock")
            .clone();
        if let Some(text) = text {
            egui::Window::new("Copy from desktop")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Your browser needs a click to copy this text.");
                    ui.horizontal(|ui| {
                        if ui.button("Copy text").clicked() {
                            self.write(ctx, text.clone(), true);
                        }
                        if ui.button("Close").clicked() {
                            *self.fallback.lock().expect("clipboard fallback lock") = None;
                        }
                    });
                    let mut shown = text.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut shown)
                            .desired_width(width)
                            .desired_rows(5),
                    );
                    ui.small("If copying is blocked, the text stays here for you to select.");
                });
        }
    }
}

/// Legacy browser copy is limited to this explicit button. It also works on HTTP where
/// navigator.clipboard is unavailable. The temporary node and focus/DOM selection are
/// restored even when the browser refuses the command.
#[cfg(target_arch = "wasm32")]
fn copy_during_click(text: &str) -> Result<bool, wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or_else(|| wasm_bindgen::JsValue::from_str("No window"))?;
    let document = window
        .document()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("No document"))?;
    let active = document.active_element();
    let selection = window.get_selection()?;
    let ranges: Vec<_> = selection
        .as_ref()
        .map(|selection| {
            (0..selection.range_count())
                .filter_map(|i| selection.get_range_at(i).ok())
                .collect()
        })
        .unwrap_or_default();
    let textarea = document
        .create_element("textarea")?
        .dyn_into::<web_sys::HtmlTextAreaElement>()?;
    textarea.set_value(text);
    textarea.set_attribute(
        "style",
        "position:fixed;left:-10000px;top:0;width:1px;height:1px;opacity:0",
    )?;
    let body = document
        .body()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("No body"))?;
    body.append_child(&textarea)?;
    textarea.select();
    // eframe listens for copy on document and would prevent this DOM copy's default.
    // Stop only this temporary node's event from bubbling; retain the browser default.
    let copy = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        |event: web_sys::Event| event.stop_propagation(),
    );
    textarea.add_event_listener_with_callback("copy", copy.as_ref().unchecked_ref())?;
    let result = document
        .unchecked_ref::<web_sys::HtmlDocument>()
        .exec_command("copy");
    let _ = textarea.remove_event_listener_with_callback("copy", copy.as_ref().unchecked_ref());
    textarea.remove();
    if let Some(active) = active.and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = active.focus();
    }
    if let Some(selection) = selection {
        let _ = selection.remove_all_ranges();
        for range in ranges {
            let _ = selection.add_range(&range);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_replies_cannot_replace_a_new_copy_and_timeouts_allow_retry() {
        let mut clipboard = Clipboard::default();
        let first = clipboard.begin().unwrap();
        assert!(clipboard.begin().is_none());
        clipboard.pending.as_mut().unwrap().1 -= std::time::Duration::from_secs(5);
        let ctx = egui::Context::default();
        clipboard.expire();
        let second = clipboard.begin().unwrap();
        assert_ne!(first, second);
        clipboard.received(
            &ctx,
            DisplayClipboard {
                id: first,
                text: None,
                error: Some("late".into()),
            },
        );
        assert_eq!(clipboard.pending.as_ref().unwrap().0, second);
        assert!(clipboard.error.is_none());
        clipboard.received(
            &ctx,
            DisplayClipboard {
                id: second,
                text: None,
                error: None,
            },
        );
        assert!(clipboard.pending.is_none());
    }
    #[test]
    fn oversized_paste_is_rejected_before_sending() {
        let mut clipboard = Clipboard::default();
        assert!(clipboard.pasted(&"x".repeat(CLIPBOARD_LIMIT + 1)).is_none());
        assert!(clipboard.pending.is_none());
        assert!(clipboard.error.is_some());
    }
}
