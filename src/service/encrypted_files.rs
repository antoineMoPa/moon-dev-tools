//! A review's encrypted files - see [`crate::api::encrypted_files`]: read with the passphrase
//! they were encrypted with, and written back encrypted with one.
//!
//! The text only ever exists here in memory, on its way between gpg and the tab. Plain reads
//! and writes of such a name are refused in [`super::files`].

use anyhow::{Result, bail};

use crate::api::{
    AppState, FileContentPayload,
    encrypted_files::{WriteEncryptedFileRequest, is_encrypted_name},
};

use super::files::folder_a_file_is_in;

/// The text of an encrypted file of the repo, decrypted with `passphrase`. It has nothing
/// committed to be new against: what HEAD has of it is as encrypted as the file.
pub(crate) fn decrypted_session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    passphrase: &str,
) -> Result<FileContentPayload> {
    if !is_encrypted_name(file_path) {
        bail!("{file_path} is not named as an encrypted file");
    }
    // gpg runs without the sessions lock, which nothing else should wait on.
    let folder = crate::api::with_session(state, session_id, |session| {
        folder_a_file_is_in(&session.repo_path, file_path)
    })?;
    let encrypted = crate::git::read_repo_bytes(&folder, file_path)?;
    Ok(FileContentPayload {
        file_path: file_path.to_string(),
        content: crate::gpg::decrypt(&encrypted, passphrase)?,
        outside_the_repo: false,
        only_written_by: None,
        committed: None,
        image_src: None,
    })
}

/// Encrypt a tab's text with the passphrase it carries and write it as the file - creating it
/// for the first save of a new file.
pub(crate) fn write_encrypted_session_file(
    state: &AppState,
    session_id: &str,
    request: &WriteEncryptedFileRequest,
) -> Result<()> {
    if !is_encrypted_name(&request.file_path) {
        bail!("{} is not named as an encrypted file", request.file_path);
    }
    crate::api::ensure_session_is_writable(state, session_id)?;
    let folder = crate::api::with_session(state, session_id, |session| {
        folder_a_file_is_in(&session.repo_path, &request.file_path)
    })?;
    let encrypted = crate::gpg::encrypt_with_passphrase(&request.content, &request.passphrase)?;
    match request.creates {
        true => crate::git::create_repo_file(&folder, &request.file_path, &encrypted),
        false => crate::git::write_repo_file(&folder, &request.file_path, &encrypted),
    }
}
