//! The before/after view of a changed image.
//!
//! The server hands both sides over as `data:` URIs, which is what the web frontend puts in an
//! `<img>` tag. egui has no loader for that scheme, so each side is decoded here once and given
//! to egui as bytes under a `bytes://` URI derived from the image's own content — which is also
//! what makes egui drop the old texture when the picture changes.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use egui::{RichText, Ui, vec2};

use crate::{
    api::ImageDiffView,
    native::{app::App, model::hash_of, theme::{Palette, SMALL_SIZE}},
};

/// The longest edge either side of the comparison is drawn at.
const MAX_IMAGE_EDGE: f32 = 320.0;

pub(crate) fn draw_image_diff(app: &mut App, ui: &mut Ui, image: &ImageDiffView, palette: &Palette) {
    ui.horizontal(|ui| {
        for (label, source, missing) in [
            ("before", &image.before_src, "(added)"),
            ("after", &image.after_src, "(deleted)"),
        ] {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(label)
                        .size(SMALL_SIZE - 1.0)
                        .color(palette.muted),
                );
                match source.as_deref().map(|source| image_source(app, source)) {
                    Some(Some(source)) => {
                        ui.add(
                            egui::Image::new(source)
                                .max_size(vec2(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE))
                                .maintain_aspect_ratio(true),
                        );
                    }
                    // The side is there but is not a data URI this build knows how to read.
                    Some(None) => {
                        ui.label(RichText::new("(cannot read this image)").color(palette.warn));
                    }
                    None => {
                        ui.label(RichText::new(missing).color(palette.muted));
                    }
                }
            });
            ui.add_space(10.0);
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
        let uri = format!("data:image/png;base64,{}", BASE64.encode(b"not really a png"));

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
