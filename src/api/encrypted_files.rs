//! Encrypted files - which names say a file is kept encrypted with gpg, and what a tab sends to
//! open one or write it back.
//!
//! The repo's side refuses to write such a name as plain text, and the window asks for a
//! passphrase before it shows or saves one - see `crate::gpg` for the encrypting itself.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The parts of a file name, between its dots, that say the file is encrypted. Any part
/// counts, not only the last one: `notes.gpg.md` is as much a secret as `notes.md.gpg`, and a
/// name that says so is never written out as plain text.
const ENCRYPTED_NAME_PARTS: &[&str] = &["gpg"];

/// Whether the file a path names is kept encrypted, by its name - see
/// [`ENCRYPTED_NAME_PARTS`]. The first part is the name itself and not a mark: `gpg.conf` is
/// a plain file.
pub(crate) fn is_encrypted_name(file_path: &str) -> bool {
    let Some(name) = std::path::Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    name.split('.').skip(1).any(|part| {
        ENCRYPTED_NAME_PARTS
            .iter()
            .any(|marked| marked.eq_ignore_ascii_case(part))
    })
}

/// An encrypted file of a session, asked for with the passphrase to read it. The passphrase
/// is wiped from memory as the request is dropped.
#[derive(Serialize, Deserialize)]
pub(crate) struct DecryptFileRequest {
    pub(crate) file_path: String,
    pub(crate) passphrase: Zeroizing<String>,
}

/// The text of an encrypted file, to be encrypted with `passphrase` and written in its place.
/// `creates` for the first save of a tab on a path nothing is at yet, which is refused where
/// something already is - the same as [`crate::backend::Backend::create_file`]. The
/// passphrase is wiped from memory as the request is dropped.
#[derive(Serialize, Deserialize)]
pub(crate) struct WriteEncryptedFileRequest {
    pub(crate) file_path: String,
    pub(crate) content: String,
    pub(crate) passphrase: Zeroizing<String>,
    pub(crate) creates: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gpg_part_anywhere_after_the_name_marks_a_file_encrypted() {
        assert!(is_encrypted_name("secrets.gpg"));
        assert!(is_encrypted_name("notes/passwords.md.gpg"));
        assert!(is_encrypted_name("notes.GPG.md"));
        assert!(!is_encrypted_name("gpg.conf"));
        assert!(!is_encrypted_name("src/gpg.rs"));
        assert!(!is_encrypted_name("gpg/notes.md"));
        assert!(!is_encrypted_name("notes.gpgx"));
    }
}
