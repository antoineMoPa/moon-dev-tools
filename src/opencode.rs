//! opencode - the models `opencode` offers, which the settings page lists for picking.
//!
//! The other agents have no command that lists their models, so for them the page takes a
//! model typed in instead - see `crate::native::settings_pane`.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

/// The models `opencode` can run with for the person, as `provider/model`, in the order it
/// lists them. `opencode models` runs no model, so it answers in about a second.
///
/// Asked as this thread's current user and in their home directory: the list is what their
/// own `opencode` is logged in to, and no repo's `opencode.json` is any business of a page
/// that is about the person rather than a project.
pub(crate) fn models() -> Result<Vec<String>> {
    let output = crate::shell_path::installed_tool("opencode")?
        .arg("models")
        .current_dir(home_of_the_current_user()?)
        .output()
        .context("failed to start opencode")?;
    if !output.status.success() {
        bail!(
            "opencode models failed (status {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    read_model_list(&String::from_utf8_lossy(&output.stdout))
}

/// The person's own home on a server that gives each a Unix user, this process's anywhere
/// else.
fn home_of_the_current_user() -> Result<PathBuf> {
    match crate::unix_users::current_user() {
        Some(person) => Ok(person.unix_user.home),
        None => std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .context("no home directory to run opencode in"),
    }
}

fn read_model_list(printed: &str) -> Result<Vec<String>> {
    printed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if !line.contains('/') {
                bail!("opencode models printed a line that is not a provider/model: {line}");
            }
            Ok(line.to_string())
        })
        .collect()
}
