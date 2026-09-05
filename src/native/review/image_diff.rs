//! The before/after view of a changed image.
//!
//! The server hands both sides over as `data:` URIs. egui has no loader for that scheme, so
//! each side is decoded here once and given
//! to egui as bytes under a `bytes://` URI derived from the image's own content - which is also
//! what makes egui drop the old texture when the picture changes.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use egui::{RichText, Ui, vec2};

use crate::{
    api::ImageDiffView,
    native::{
        app::App,
        model::hash_of,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// The share of the window an image may take vertically. Inside a scroll area a pane is as
/// tall as it likes, so the window is what the picture has to be measured against.
const TALLEST_IMAGE_FRACTION: f32 = 0.7;
/// Between the before and the after.
const IMAGE_GAP: f32 = 12.0;
/// Around the whole comparison. The hunk card has no inner margin of its own - a diff's lines
/// run edge to edge - so the pictures bring their own.
const IMAGE_PADDING: i8 = 10;
/// Between a side's label and its picture.
const LABEL_GAP: f32 = 4.0;

pub(crate) fn draw_image_diff(
    app: &mut App,
    ui: &mut Ui,
    file_path: &str,
    image: &ImageDiffView,
    palette: &Palette,
) {
    egui::Frame::new()
        .inner_margin(egui::Margin::same(IMAGE_PADDING))
        .show(ui, |ui| {
            draw_file_link(app, ui, file_path, palette);
            ui.add_space(8.0);

            // Each side takes half of what is going, scaled up to fill it - a favicon is
            // otherwise drawn at its own 16 points and is impossible to compare.
            let half = ((ui.available_width() - IMAGE_GAP) / 2.0).max(80.0);
            let tallest = ui.ctx().viewport_rect().height() * TALLEST_IMAGE_FRACTION;

            ui.horizontal_top(|ui| {
                for (label, source, missing) in [
                    ("before", &image.before_src, "(added)"),
                    ("after", &image.after_src, "(deleted)"),
                ] {
                    ui.vertical(|ui| {
                        ui.set_width(half);
                        ui.label(
                            RichText::new(label)
                                .size(SMALL_SIZE - 1.0)
                                .color(palette.muted),
                        );
                        ui.add_space(LABEL_GAP);
                        match source.as_deref().map(|source| image_source(app, source)) {
                            Some(Some(source)) => {
                                ui.add(
                                    egui::Image::new(source)
                                        .fit_to_exact_size(vec2(half, tallest))
                                        .maintain_aspect_ratio(true),
                                );
                            }
                            // The side is there but is not a data URI this build can read.
                            Some(None) => {
                                ui.label(
                                    RichText::new("(cannot read this image)").color(palette.warn),
                                );
                            }
                            None => {
                                ui.label(RichText::new(missing).color(palette.muted));
                            }
                        }
                    });
                    ui.add_space(IMAGE_GAP);
                }
            });
        });
}

/// The image's name, and beside it a way to open the file itself - the pane can only show it
/// at the size it fits, and comparing pixels needs a real viewer.
///
/// The name is a label rather than a button so it can be selected and copied. One widget
/// cannot both sweep a selection and act on a click, so opening it is its own button.
fn draw_file_link(app: &mut App, ui: &mut Ui, file_path: &str, palette: &Palette) {
    let full_path = app.repo_file_path(file_path).filter(|path| path.exists());

    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(RichText::new(file_path).size(SMALL_SIZE).color(
                if full_path.is_some() {
                    palette.accent
                } else {
                    palette.muted
                },
            ))
            .selectable(true),
        );

        // A review served from somewhere else has no local file to open.
        let Some(full_path) = full_path else {
            return;
        };
        if widgets::quiet_button(ui, "[open]")
            .on_hover_text(format!("open {}", full_path.display()))
            .clicked()
            && let Err(error) = webbrowser::open(&full_path.to_string_lossy())
        {
            app.model
                .error(format!("could not open {file_path}: {error}"));
        }
    });
}

/// The image behind a `data:` URI, decoded on first sight and kept until the window closes.
fn image_source(app: &mut App, data_uri: &str) -> Option<egui::ImageSource<'static>> {
    let key = hash_of(data_uri);
    let decoded = app
        .decoded_images
        .entry(key)
        .or_insert_with(|| decode_image_data_uri(data_uri));
    let (extension, bytes) = decoded.as_ref()?;

    Some(egui::ImageSource::Bytes {
        // Named after the content: two hunks showing the same image share one texture, and an
        // image that changes gets a URI egui has never seen.
        uri: format!("bytes://image-{key:016x}.{extension}").into(),
        bytes: egui::load::Bytes::Shared(Arc::clone(bytes)),
    })
}

/// The file extension egui should read a MIME type as. It sniffs the bytes for most formats,
/// but the URI's extension is what routes an SVG to the loader that can draw it.
const MIME_EXTENSIONS: &[(&str, &str)] = &[
    ("image/apng", "apng"),
    ("image/avif", "avif"),
    ("image/gif", "gif"),
    ("image/jpeg", "jpg"),
    ("image/png", "png"),
    ("image/svg+xml", "svg"),
    ("image/webp", "webp"),
];

/// Splits `data:<mime>;base64,<payload>` into the extension to name it by and its bytes.
fn decode_image_data_uri(data_uri: &str) -> Option<(&'static str, Arc<[u8]>)> {
    let body = data_uri.strip_prefix("data:")?;
    let (mime_type, payload) = body.split_once(";base64,")?;
    let extension = MIME_EXTENSIONS
        .iter()
        .find(|(known, _)| *known == mime_type)
        .map(|(_, extension)| *extension)?;
    let bytes = BASE64.decode(payload).ok()?;
    Some((extension, Arc::from(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_uri_gives_up_its_bytes_and_a_name_to_read_them_as() {
        let uri = format!(
            "data:image/png;base64,{}",
            BASE64.encode(b"not really a png")
        );

        let (extension, bytes) = decode_image_data_uri(&uri).expect("the uri should decode");

        assert_eq!(extension, "png");
        assert_eq!(&*bytes, b"not really a png");
    }

    #[test]
    fn an_svg_keeps_the_extension_its_loader_is_chosen_by() {
        let uri = format!("data:image/svg+xml;base64,{}", BASE64.encode(b"<svg/>"));

        let (extension, _) = decode_image_data_uri(&uri).expect("the uri should decode");

        assert_eq!(extension, "svg");
    }

    #[test]
    fn anything_that_is_not_a_base64_image_data_uri_is_refused() {
        assert!(decode_image_data_uri("https://example.com/cat.png").is_none());
        assert!(decode_image_data_uri("data:image/png,raw%20bytes").is_none());
        assert!(decode_image_data_uri("data:text/plain;base64,aGk=").is_none());
        assert!(decode_image_data_uri("data:image/png;base64,not base64!").is_none());
    }
}
