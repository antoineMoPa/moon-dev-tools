//! A JSON file shown as a tree whose objects and arrays fold, in place of the text of it.

use egui::{RichText, Ui};
use egui_frames::PaneId;
use serde_json::Value;

use crate::native::{
    app::App,
    theme::{Palette, SMALL_SIZE},
};

/// Whether the file is JSON, which is what decides if the pane offers `[tree]`.
pub(crate) fn is_json(file_path: &str) -> bool {
    std::path::Path::new(file_path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

/// The text of the pane drawn as a tree, or why it is not one: a file half-typed is not JSON,
/// and the pane says where the parser stopped rather than showing an empty tree.
pub(super) fn draw_tree(app: &mut App, ui: &mut Ui, pane_id: PaneId) {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    let palette = app.palette_of();
    let parsed: Result<Value, _> = serde_json::from_str(editor.code.text());
    match parsed {
        Err(error) => {
            ui.label(RichText::new(format!("not valid JSON: {error}")).color(palette.warn));
        }
        Ok(value) => {
            egui::ScrollArea::both()
                .id_salt(("file-pane-json-tree", pane_id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_value(ui, &palette, egui::Id::new(("json", pane_id)), 0, None, &value);
                });
        }
    }
}

/// One value: a leaf on a line of its own, or a header that folds the members under it.
fn draw_value(
    ui: &mut Ui,
    palette: &Palette,
    id: egui::Id,
    depth: usize,
    key: Option<&str>,
    value: &Value,
) {
    let label = key.map_or(String::new(), |key| format!("{key}: "));
    match value {
        Value::Object(members) => {
            let summary = format!("{label}{{ {} }}", members.len());
            fold(ui, palette, id, depth, &summary, |ui| {
                for (name, member) in members {
                    draw_value(ui, palette, id.with(name), depth + 1, Some(name), member);
                }
            });
        }
        Value::Array(items) => {
            let summary = format!("{label}[ {} ]", items.len());
            fold(ui, palette, id, depth, &summary, |ui| {
                for (index, item) in items.iter().enumerate() {
                    draw_value(
                        ui,
                        palette,
                        id.with(index),
                        depth + 1,
                        Some(&index.to_string()),
                        item,
                    );
                }
            });
        }
        leaf => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(palette.muted));
                let (text, colour) = match leaf {
                    Value::String(text) => (format!("{text:?}"), palette.added),
                    Value::Null => ("null".to_string(), palette.muted),
                    other => (other.to_string(), palette.ink),
                };
                ui.add(egui::Label::new(RichText::new(text).color(colour)).selectable(true));
            });
        }
    }
}

/// The first two levels are open and the rest folded, so a big file opens on its shape.
fn fold(
    ui: &mut Ui,
    palette: &Palette,
    id: egui::Id,
    depth: usize,
    summary: &str,
    body: impl FnOnce(&mut Ui),
) {
    let open_by_default = depth < 2;
    egui::CollapsingHeader::new(RichText::new(summary).size(SMALL_SIZE + 2.0).color(palette.ink))
        .id_salt(id)
        .default_open(open_by_default)
        .show(ui, body);
}

#[cfg(test)]
mod tests {
    use super::is_json;

    #[test]
    fn only_json_files_are_offered_the_tree() {
        assert!(is_json("package.json"));
        assert!(is_json("a/B.JSON"));
        assert!(!is_json("src/lib.rs"));
        assert!(!is_json("json"));
        assert!(!is_json("data.jsonl"));
    }
}
