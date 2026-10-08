//! Image formats - which file extensions name a picture, and the MIME type each one travels
//! under in a `data:` URI.
//!
//! The repo's side reads a path's extension to say what it is sending; the window reads the
//! MIME type back into an extension, which is what egui picks a loader by.

/// A path's extension, in lower case, and the MIME type of the picture it names. Where two
/// extensions name one format, the first row is the extension its MIME type is read back as.
const IMAGE_FORMATS: &[(&str, &str)] = &[
    ("apng", "image/apng"),
    ("avif", "image/avif"),
    ("gif", "image/gif"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("svg", "image/svg+xml"),
    ("webp", "image/webp"),
];

/// The MIME type of the picture a path names by its extension, whatever case it is written
/// in. `None` for a path that names no picture.
pub(crate) fn mime_type_of_path(path: &str) -> Option<&'static str> {
    let extension = std::path::Path::new(path).extension()?.to_str()?;
    IMAGE_FORMATS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(extension))
        .map(|(_, mime_type)| *mime_type)
}

/// The extension a picture of this MIME type is named with. `None` for a type that is not a
/// picture's.
pub(crate) fn extension_of_mime_type(mime_type: &str) -> Option<&'static str> {
    IMAGE_FORMATS
        .iter()
        .find(|(_, known)| *known == mime_type)
        .map(|(extension, _)| *extension)
}
