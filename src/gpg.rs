//! gpg - encrypts and decrypts the files a tab keeps encrypted, with a passphrase, by running
//! the `gpg` the person installed.
//!
//! Symmetric only: the passphrase is the key, and there is no recipient to pick. The
//! passphrase is the first line gpg reads on its standard input and the file is the rest of
//! it, so neither is ever on a command line or on disk, and that input is wiped from memory
//! once gpg has it. gpg-agent is told not to keep the passphrase: a tab asks for it each time
//! it opens or saves a file, and a wrong one typed after a right one must fail rather than be
//! answered from the agent's cache.

use std::io::Write;

use anyhow::{Context, Result, bail};
use zeroize::Zeroizing;

/// What every run of gpg here is given: no question asked of a terminal or a pinentry window,
/// the passphrase read from standard input, and none of it remembered.
const GPG_PASSPHRASE_ARGS: &[&str] = &[
    "--batch",
    "--quiet",
    "--yes",
    "--no-symkey-cache",
    "--pinentry-mode",
    "loopback",
    "--passphrase-fd",
    "0",
];

/// The text of an encrypted file, from its bytes as they are on disk - binary or armored,
/// gpg reads both.
pub(crate) fn decrypt(encrypted: &[u8], passphrase: &str) -> Result<String> {
    let plain = run_gpg(&["--decrypt"], passphrase, encrypted)?;
    String::from_utf8(plain).context("the decrypted file is not text")
}

/// `text` encrypted with `passphrase` alone, as the bytes of a `.gpg` file.
pub(crate) fn encrypt_with_passphrase(text: &str, passphrase: &str) -> Result<Vec<u8>> {
    run_gpg(
        &["--symmetric", "--output", "-"],
        passphrase,
        text.as_bytes(),
    )
}

fn run_gpg(args: &[&str], passphrase: &str, input: &[u8]) -> Result<Vec<u8>> {
    if passphrase.is_empty() {
        bail!("the passphrase is empty");
    }
    // gpg reads the passphrase up to the first line break, and the rest would be read as the
    // file.
    if passphrase.contains(['\n', '\r']) {
        bail!("a passphrase cannot have a line break in it");
    }
    let mut child = crate::shell_path::installed_tool("gpg")?
        .args(GPG_PASSPHRASE_ARGS)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("could not run gpg - is it installed?")?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    // Wiped as it is dropped: it holds the passphrase, and the text in the clear.
    let mut fed = Zeroizing::new(Vec::with_capacity(passphrase.len() + 1 + input.len()));
    fed.extend_from_slice(passphrase.as_bytes());
    fed.push(b'\n');
    fed.extend_from_slice(input);
    // Fed from a thread of its own: gpg writes as it reads, and a file larger than a pipe
    // holds would leave both sides waiting on the other. A gpg that stopped reading - a wrong
    // passphrase - breaks the pipe, which its exit status says better.
    let feeding = std::thread::spawn(move || {
        let _ = stdin.write_all(&fed);
    });
    let output = child.wait_with_output().context("gpg did not finish")?;
    feeding
        .join()
        .expect("the thread feeding gpg does not panic");
    if !output.status.success() {
        // gpg starts each of its lines with `gpg:`, which says who is talking already.
        let said = String::from_utf8_lossy(&output.stderr);
        match said.trim() {
            "" => bail!("gpg failed with {}", output.status),
            said => bail!("{said}"),
        }
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the gpg of the machine the tests run on.
    #[test]
    fn text_encrypted_with_a_passphrase_opens_with_it_and_only_with_it() {
        let text = "line one\nline two\n";
        let encrypted = encrypt_with_passphrase(text, "right horse").unwrap();
        assert_ne!(encrypted, text.as_bytes());
        assert_eq!(decrypt(&encrypted, "right horse").unwrap(), text);
        assert!(decrypt(&encrypted, "wrong horse").is_err());
    }

    #[test]
    fn a_passphrase_with_a_line_break_is_refused() {
        assert!(encrypt_with_passphrase("text", "two\nlines").is_err());
        assert!(encrypt_with_passphrase("text", "").is_err());
    }
}
