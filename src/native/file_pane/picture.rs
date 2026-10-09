//! Picture - an image file in a file tab, drawn as the picture it is in place of an editor.
//!
//! The repo's side sends such a file as a `data:` URI, the way it sends the two sides of a
//! changed image - see [`crate::native::review::image_diff`], which is also what reads it
//! here. The tab has no text, so nothing about it can be edited or saved.

use std::sync::Arc;

use egui::{RichText, Ui};

use crate::native::{model::hash_of, review::image_diff, theme::Palette};

/// The picture a tab is showing, decoded once as it arrives.
pub(super) struct Picture {
    /// The hash of the `data:` URI the picture arrived as, which egui keeps its texture by.
    key: u64,
    /// What egui reads the bytes as.
    extension: &'static str,
    bytes: Arc<[u8]>,
    /// How wide and how tall it is in pixels, as its own header has it. `None` for bytes no
    /// decoder of this build reads, which egui says for itself where the picture would be.
    size: Option<(u32, u32)>,
}

impl Picture {
    /// The picture behind the `data:` URI a read of the file came back with.
    pub(super) fn from_data_uri(data_uri: &str) -> Self {
        let (extension, bytes) = image_diff::decode_image_data_uri(data_uri)
            .expect("a picture is sent as a base64 `data:` URI of a known image format");
        Self {
            key: hash_of(data_uri),
            extension,
            size: size_of(&bytes),
            bytes,
        }
    }

    /// At its own size, and scaled down to the pane when it is larger than that.
    pub(super) fn draw(&self, ui: &mut Ui, palette: &Palette) {
        // A picture is drawn from one texture, and the graphics card says how long a side of
        // one may be. Handing it a longer one takes the window down, so that is said instead.
        let longest_side = ui.ctx().input(|input| input.max_texture_side);
        if let Some(too_long) = self.size.and_then(|size| longer_than(size, longest_side)) {
            ui.label(RichText::new(too_long).color(palette.warn));
            return;
        }
        ui.add(
            egui::Image::new(image_diff::bytes_source(
                self.key,
                self.extension,
                &self.bytes,
            ))
            .fit_to_original_size(1.0)
            .max_size(ui.available_size()),
        );
    }
}

/// How wide and how tall a picture is in pixels, as its own header has it. `None` for bytes
/// no decoder of this build reads.
pub(super) fn size_of(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
}

/// What is said in place of a picture with a side longer than the graphics card draws, and
/// `None` for one that fits.
pub(super) fn longer_than((width, height): (u32, u32), longest_side: usize) -> Option<String> {
    (width.max(height) as usize > longest_side).then(|| {
        format!(
            "{width} × {height} pixels is more than this window can draw: {longest_side} a side at most"
        )
    })
}
