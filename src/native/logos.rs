//! The three apps' logos, embedded as pixels.
//!
//! The sources are the `logo-*.svg` files at the repo root. They are rasterized once, by
//! `scripts/render-logos.sh`, into the PNGs under `assets/logos/` that are embedded here -
//! rather than at build or run time, because the shell logo sets its text in Futura, and only
//! a machine that has the font renders it right.

use crate::cli::Frame;

/// The logo of one executable at one of the rendered sizes, as PNG bytes.
///
/// The sizes are the ones the icons actually need - the `.icns` entries, the Linux launcher
/// icon and the window icon - and asking for any other size is a bug, not a fallback.
pub(crate) fn logo_png(frame: Frame, size: usize) -> &'static [u8] {
    match (frame, size) {
        (Frame::Review, 32) => include_bytes!("../../assets/logos/moonreview-32.png"),
        (Frame::Review, 64) => include_bytes!("../../assets/logos/moonreview-64.png"),
        (Frame::Review, 128) => include_bytes!("../../assets/logos/moonreview-128.png"),
        (Frame::Review, 256) => include_bytes!("../../assets/logos/moonreview-256.png"),
        (Frame::Review, 512) => include_bytes!("../../assets/logos/moonreview-512.png"),
        (Frame::Tasks, 32) => include_bytes!("../../assets/logos/moontasks-32.png"),
        (Frame::Tasks, 64) => include_bytes!("../../assets/logos/moontasks-64.png"),
        (Frame::Tasks, 128) => include_bytes!("../../assets/logos/moontasks-128.png"),
        (Frame::Tasks, 256) => include_bytes!("../../assets/logos/moontasks-256.png"),
        (Frame::Tasks, 512) => include_bytes!("../../assets/logos/moontasks-512.png"),
        (Frame::Shell, 32) => include_bytes!("../../assets/logos/moonshell-32.png"),
        (Frame::Shell, 64) => include_bytes!("../../assets/logos/moonshell-64.png"),
        (Frame::Shell, 128) => include_bytes!("../../assets/logos/moonshell-128.png"),
        (Frame::Shell, 256) => include_bytes!("../../assets/logos/moonshell-256.png"),
        (Frame::Shell, 512) => include_bytes!("../../assets/logos/moonshell-512.png"),
        (frame, size) => panic!("no {size}px logo is rendered for {}", frame.program()),
    }
}

/// The logo as something to draw inside the window, at one of the rendered sizes.
///
/// The bytes are handed to egui under a `bytes://` URI naming the frame and the size, which is
/// what makes it decode and upload the texture on the first frame that shows the logo and reuse
/// it on every frame after.
pub(crate) fn logo_image_source(frame: Frame, size: usize) -> egui::ImageSource<'static> {
    egui::ImageSource::Bytes {
        uri: format!("bytes://logo-{}-{size}.png", frame.program()).into(),
        bytes: egui::load::Bytes::Static(logo_png(frame, size)),
    }
}

/// The logo as the window icon: the dock, the task switcher and the title bar all take it
/// from here. 256 pixels because the Dock on a Retina display draws the icon at up to
/// 128 points, and eframe hands these exact pixels to `setApplicationIconImage` - a smaller
/// bitmap is stretched there and arrives blurry.
pub(crate) fn window_icon(frame: Frame) -> egui::IconData {
    let image = image::load_from_memory(logo_png(frame, 256))
        .expect("the embedded logo is a valid png")
        .to_rgba8();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::FRAMES;

    /// Every size an icon writer asks for has to be there for every frame, and has to be the
    /// square it says it is.
    #[test]
    fn every_frame_has_its_logo_at_every_rendered_size() {
        for frame in FRAMES {
            for size in [32, 64, 128, 256, 512] {
                let image = image::load_from_memory(logo_png(*frame, size))
                    .expect("the embedded logo should be a valid png");
                assert_eq!(
                    (image.width() as usize, image.height() as usize),
                    (size, size),
                    "{} at {size}px",
                    frame.program()
                );
            }
        }
    }
}
