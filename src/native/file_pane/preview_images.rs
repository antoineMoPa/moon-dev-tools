//! Preview images - the pictures a markdown file's rendered page shows.
//!
//! The page asks egui for each picture by a URI, and egui asks its loaders for the bytes. A
//! picture written `![the board](img/board.png)` is a file of the repo beside the markdown
//! file, and the repo may be on another machine - so its bytes come through the backend, by
//! the read a picture's own tab makes (see [`super::picture`]), and not off this machine's
//! disk. [`PreviewImages`] is the loader that answers for those URIs: it says the picture is
//! on its way and notes that it was asked for, and the pane that drew the page sends for it.
//!
//! A picture written as an address - `https://…` - is not one of these, and is not fetched.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use egui::load::{BytesLoadResult, BytesLoader, BytesPoll, LoadError};

use super::picture;
use crate::{
    api::FileContentPayload,
    native::{model::hash_of, review::image_diff, widgets},
};

/// What the URI of a page's picture starts with. Egui hands every loader every URI, and this
/// is how the one below knows its own.
const SCHEME: &str = "markdown-image://";

/// The pictures of every rendered page of a window, as egui's loader for them.
#[derive(Default)]
pub(crate) struct PreviewImages {
    shelf: Mutex<Shelf>,
}

#[derive(Default)]
struct Shelf {
    /// The pages pictures are asked for from, each under the word its pictures' URIs carry
    /// after the scheme.
    pages: HashMap<String, Page>,
    /// By the URI each was asked for under.
    pictures: HashMap<String, Picture>,
    /// Asked for by a page and not sent for yet.
    asked: Vec<Asked>,
}

/// A markdown file a page is rendered from.
struct Page {
    session_id: String,
    file_path: String,
}

enum Picture {
    OnItsWay,
    Here {
        bytes: Arc<[u8]>,
        /// How wide and how tall it is in pixels, as its own header has it - see
        /// [`picture::size_of`].
        size: Option<(u32, u32)>,
    },
    /// Why there is no picture to show, which egui writes where it would have been.
    Failed(String),
}

/// A picture a page asked for: the URI it is kept under, and the read that fetches it.
pub(super) struct Asked {
    uri: String,
    pub(super) session_id: String,
    /// Where the picture is, from the repo's root.
    pub(super) file_path: String,
}

impl PreviewImages {
    /// The loader of this context, put on it the first time a page is rendered in it.
    pub(super) fn of(ctx: &egui::Context) -> Arc<Self> {
        let id = egui::Id::new("markdown-preview-images");
        if let Some(images) = ctx.data(|data| data.get_temp::<Arc<Self>>(id)) {
            return images;
        }
        let images = Arc::new(Self::default());
        ctx.add_bytes_loader(Arc::clone(&images) as _);
        ctx.data_mut(|data| data.insert_temp(id, Arc::clone(&images)));
        images
    }

    /// What the URI of every picture of this page starts with. The markdown viewer puts it in
    /// front of a picture written with no scheme of its own, so the URI ends in the picture
    /// as the file writes it.
    pub(super) fn uris_of_page(&self, session_id: &str, file_path: &str) -> String {
        let page = format!("{:016x}", hash_of(&format!("{session_id}\n{file_path}")));
        let uris = format!("{SCHEME}{page}/");
        self.shelf().pages.entry(page).or_insert_with(|| Page {
            session_id: session_id.to_string(),
            file_path: file_path.to_string(),
        });
        uris
    }

    /// The pictures asked for since this was last called, for the pane to send for.
    pub(super) fn take_asked(&self) -> Vec<Asked> {
        std::mem::take(&mut self.shelf().asked)
    }

    /// What a read of a picture came back with.
    pub(super) fn arrived(&self, asked: &Asked, read: anyhow::Result<FileContentPayload>) {
        let arrived = match read {
            Err(error) => Picture::Failed(format!("{error}")),
            Ok(payload) => match payload
                .image_src
                .as_deref()
                .and_then(image_diff::decode_image_data_uri)
            {
                Some((_extension, bytes)) => Picture::Here {
                    size: picture::size_of(&bytes),
                    bytes,
                },
                // The file is there and is text, or a format the repo's side sends as text.
                None => Picture::Failed(format!(
                    "{} is not a picture this window draws",
                    asked.file_path
                )),
            },
        };
        // A picture forgotten while it was on its way stays forgotten: it is asked for again
        // by the next page that shows it.
        if let Some(kept) = self.shelf().pictures.get_mut(&asked.uri) {
            *kept = arrived;
        }
    }

    /// Let go of every picture of a page, so the next draw of the page reads them again: a
    /// picture written since the page was last opened is then the one shown.
    pub(super) fn forget_page(&self, ctx: &egui::Context, uris_of_page: &str) {
        let uris: Vec<String> = self
            .shelf()
            .pictures
            .keys()
            .filter(|uri| uri.starts_with(uris_of_page))
            .cloned()
            .collect();
        // Through the context rather than off the shelf here: the picture egui decoded from
        // the bytes, and the texture it made of that, are kept by other loaders under the
        // same URI.
        for uri in uris {
            ctx.forget_image(&uri);
        }
    }

    fn shelf(&self) -> std::sync::MutexGuard<'_, Shelf> {
        self.shelf.lock().expect("the shelf's lock was poisoned")
    }
}

impl BytesLoader for PreviewImages {
    fn id(&self) -> &str {
        egui::generate_loader_id!(PreviewImages)
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        let Some(named) = uri.strip_prefix(SCHEME) else {
            return Err(LoadError::NotSupported);
        };
        let mut shelf = self.shelf();
        if let Some(picture) = shelf.pictures.get(uri) {
            return match picture {
                Picture::OnItsWay => Ok(BytesPoll::Pending { size: None }),
                Picture::Failed(why) => Err(LoadError::Loading(why.clone())),
                Picture::Here { bytes, size } => {
                    let longest_side = ctx.input(|input| input.max_texture_side);
                    match size.and_then(|size| picture::longer_than(size, longest_side)) {
                        Some(too_long) => Err(LoadError::Loading(too_long)),
                        None => Ok(BytesPoll::Ready {
                            size: None,
                            bytes: egui::load::Bytes::Shared(Arc::clone(bytes)),
                            mime: None,
                        }),
                    }
                }
            };
        }

        let (page, written) = named
            .split_once('/')
            .expect("a page's picture is named by the page, a slash, and the picture");
        let page = shelf
            .pages
            .get(page)
            .expect("a page is noted before its pictures are asked for");
        let Some(file_path) = path_beside(&page.file_path, written) else {
            let outside = format!("{written} is outside the project");
            shelf
                .pictures
                .insert(uri.to_string(), Picture::Failed(outside.clone()));
            return Err(LoadError::Loading(outside));
        };
        let asked = Asked {
            uri: uri.to_string(),
            session_id: page.session_id.clone(),
            file_path,
        };
        shelf.asked.push(asked);
        shelf.pictures.insert(uri.to_string(), Picture::OnItsWay);
        Ok(BytesPoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.shelf().pictures.remove(uri);
    }

    fn forget_all(&self) {
        self.shelf().pictures.clear();
    }

    fn byte_size(&self) -> usize {
        self.shelf()
            .pictures
            .values()
            .map(|picture| match picture {
                Picture::Here { bytes, .. } => bytes.len(),
                Picture::OnItsWay | Picture::Failed(_) => 0,
            })
            .sum()
    }

    fn has_pending(&self) -> bool {
        self.shelf()
            .pictures
            .values()
            .any(|picture| matches!(picture, Picture::OnItsWay))
    }
}

/// Where in the repo a picture is, from how the markdown file writes it: against the folder
/// the file is in, or against the repo's root when it is written with a slash in front -
/// which is how GitHub reads the same line. `None` for one that climbs out of the repo.
fn path_beside(markdown_path: &str, written: &str) -> Option<String> {
    let written = percent_decoded(written);
    let (folder, picture) = match written.strip_prefix('/') {
        Some(from_the_root) => ("", from_the_root),
        None => (widgets::directory_of(markdown_path), written.as_str()),
    };
    let mut path: Vec<&str> = Vec::new();
    for part in folder.split('/').chain(picture.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                path.pop()?;
            }
            part => path.push(part),
        }
    }
    Some(path.join("/"))
}

/// A picture's destination as the path it names: markdown writes a space in one as `%20`.
fn percent_decoded(written: &str) -> String {
    let bytes = written.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = (bytes[at] == b'%')
            .then(|| written.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                at += 3;
            }
            None => {
                decoded.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_found_beside_the_markdown_file_that_writes_it() {
        for (markdown, written, path) in [
            ("README.md", "logo.png", Some("logo.png")),
            ("README.md", "./assets/logo.png", Some("assets/logo.png")),
            (
                "docs/guide/intro.md",
                "img/board.png",
                Some("docs/guide/img/board.png"),
            ),
            (
                "docs/guide/intro.md",
                "../img/board.png",
                Some("docs/img/board.png"),
            ),
            // From the repo's root, as GitHub reads it.
            (
                "docs/guide/intro.md",
                "/assets/logo.png",
                Some("assets/logo.png"),
            ),
            (
                "docs/intro.md",
                "a%20screen%20shot.png",
                Some("docs/a screen shot.png"),
            ),
            ("docs/intro.md", "../../elsewhere/secret.png", None),
        ] {
            assert_eq!(
                path_beside(markdown, written).as_deref(),
                path,
                "{written} in {markdown}"
            );
        }
    }

    /// The loader's whole round: asked for, a picture is on its way and is noted for the pane
    /// to send for; once the read is back, the bytes are what egui is given.
    #[test]
    fn a_picture_asked_for_is_on_its_way_until_its_read_comes_back() {
        let ctx = egui::Context::default();
        let images = PreviewImages::of(&ctx);
        let uri = format!(
            "{}img/dot.png",
            images.uris_of_page("session", "docs/intro.md")
        );

        assert!(matches!(
            images.load(&ctx, &uri),
            Ok(BytesPoll::Pending { .. })
        ));
        // Asked again on the next frame, it is sent for once.
        assert!(matches!(
            images.load(&ctx, &uri),
            Ok(BytesPoll::Pending { .. })
        ));
        let asked = images.take_asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].session_id, "session");
        assert_eq!(asked[0].file_path, "docs/img/dot.png");

        images.arrived(
            &asked[0],
            Ok(FileContentPayload {
                file_path: asked[0].file_path.clone(),
                content: String::new(),
                outside_the_repo: false,
                only_written_by: None,
                committed: None,
                image_src: Some("data:image/png;base64,aGVsbG8=".to_string()),
            }),
        );
        let Ok(BytesPoll::Ready { bytes, .. }) = images.load(&ctx, &uri) else {
            panic!("the picture's bytes should be there once its read is back");
        };
        assert_eq!(&bytes[..], b"hello");
    }
}
