//! Handing a pass key to a window this one starts through `open`, without the key being in
//! anyone's arguments.
//!
//! A window started through its macOS launcher is started by LaunchServices, with an
//! environment of its own rather than this one, so the only way in is `open --env` - and
//! `open`'s arguments are in the process list for every user of the machine to read. What goes
//! there instead is the path of a file only this user can read, holding the key: a path is no
//! secret. The window reads the key out of it and deletes it straight away - see
//! [`take_handed_off`] - so the key is on disk for the moment the window takes to start.
//!
//! A window that never starts would leave its file behind, so every handoff first deletes the
//! ones older than [`STALE_AFTER`].

use std::{
    env,
    ffi::OsString,
    fs,
    sync::OnceLock,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

/// Where a window started through `open` finds the file its key was handed off in.
pub(crate) const PASS_KEY_FILE_ENV_VAR: &str = "MOON_PASS_KEY_FILE";

/// The folder under `~/.moonreview` the files are written to.
const HANDOFF_DIR_NAME: &str = "handoff";

/// How old a handoff has to be for its window to be taken as never having started. Starting a
/// window takes a second or two; minutes is a window that is not coming.
const STALE_AFTER: Duration = Duration::from_secs(5 * 60);

const NAME_LENGTH: usize = 16;

/// Write `pass_key` to a new file for a window about to be started, and say where. Older
/// handoffs no window took are deleted first.
/// How the process that started this one handed it a pass key, as its environment said at
/// startup.
pub(crate) enum HandedOver {
    /// A handoff file, to read and delete - see [`take_handed_off`].
    KeyFile(PathBuf),
    /// The key itself, in `MOON_PASS_KEY`: set by a window starting another directly, or by a
    /// person in their shell.
    Key(OsString),
}

static HANDED_OVER: OnceLock<Option<HandedOver>> = OnceLock::new();

/// Take the pass key this process was handed out of its environment, keeping it for
/// [`handed_over`]. Out, because everything this process starts inherits its environment - a
/// git hook, an editor, a launcher - and a key left there would be handed to all of them, and
/// a handoff file's name to programs that would find it deleted.
///
/// Called once, first thing in `crate::run`.
pub(crate) fn take_from_environment() {
    let key_var = crate::backend::remote::PASS_KEY_ENV_VAR;
    let handed = match (env::var_os(PASS_KEY_FILE_ENV_VAR), env::var_os(key_var)) {
        (Some(path), _) => Some(HandedOver::KeyFile(PathBuf::from(path))),
        (None, Some(key)) => Some(HandedOver::Key(key)),
        (None, None) => None,
    };
    // SAFETY: `crate::run` calls this before it starts a thread of its own, and nothing in a
    // bare `main` has started one, so no other thread can be reading the environment while it
    // changes.
    unsafe {
        env::remove_var(PASS_KEY_FILE_ENV_VAR);
        env::remove_var(key_var);
    }
    assert!(
        HANDED_OVER.set(handed).is_ok(),
        "the environment's pass key is taken once, at startup"
    );
}

/// The pass key this process was handed at startup, if it was - see
/// [`take_from_environment`].
pub(crate) fn handed_over() -> Option<&'static HandedOver> {
    HANDED_OVER
        .get()
        .expect("the environment's pass key is taken at startup, before it is asked for")
        .as_ref()
}

pub(crate) fn hand_off(pass_key: &str) -> Result<PathBuf> {
    hand_off_in(&handoff_dir()?, pass_key, SystemTime::now())
}

/// The key a window was handed at `path`, which is deleted as it is read: a handoff is for
/// one window, once. A file that is not there is an error rather than no key - the window was
/// told a key would be there.
pub(crate) fn take_handed_off(path: &Path) -> Result<String> {
    let pass_key = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read the pass key handed off in {}",
            path.display()
        )
    })?;
    fs::remove_file(path)
        .with_context(|| format!("failed to delete the pass key handoff {}", path.display()))?;
    if pass_key.is_empty() {
        bail!("the pass key handoff {} was empty", path.display());
    }
    Ok(pass_key)
}

fn hand_off_in(dir: &Path, pass_key: &str, now: SystemTime) -> Result<PathBuf> {
    let mut folder = fs::DirBuilder::new();
    folder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut folder, 0o700);
    folder
        .create(dir)
        .with_context(|| format!("failed to create {}", dir.display()))?;
    delete_stale_handoffs(dir, now)?;

    let mut name = [0u8; NAME_LENGTH];
    getrandom_03::fill(&mut name)
        .map_err(|error| anyhow::anyhow!("the OS has no randomness to name a handoff: {error}"))?;
    let path = dir.join(URL_SAFE_NO_PAD.encode(name));

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&path)
        .and_then(|mut file| file.write_all(pass_key.as_bytes()));
    if let Err(error) = written {
        let _ = fs::remove_file(&path);
        return Err(error).with_context(|| format!("failed to write {}", path.display()));
    }
    Ok(path)
}

/// Delete the handoffs in `dir` last written longer than [`STALE_AFTER`] before `now`. One
/// that is gone by the time it is deleted was taken by its window, or swept by another.
fn delete_stale_handoffs(dir: &Path, now: SystemTime) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to list {}", dir.display()))? {
        let path = entry
            .with_context(|| format!("failed to list {}", dir.display()))?
            .path();
        let written = match fs::metadata(&path).and_then(|metadata| metadata.modified()) {
            Ok(written) => written,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        // A file written after `now` - a clock set back - is no older than anything else.
        let age = now.duration_since(written).unwrap_or(Duration::ZERO);
        if age <= STALE_AFTER {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to delete {}", path.display()));
            }
        }
    }
    Ok(())
}

/// Where handoffs are written. Under test, a scratch folder of the test process's own, as with
/// the secret - a test run must never write into the real `~/.moonreview`.
fn handoff_dir() -> Result<PathBuf> {
    #[cfg(test)]
    {
        Ok(std::env::temp_dir().join(format!(
            "moonreview-test-{HANDOFF_DIR_NAME}-{}",
            std::process::id()
        )))
    }
    #[cfg(not(test))]
    {
        Ok(crate::settings::moonreview_dir()
            .context("no home directory to hand a pass key off in")?
            .join(HANDOFF_DIR_NAME))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("moonreview-handoff-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_handoff_is_only_its_owner_s_and_is_gone_once_taken() {
        let dir = scratch("taken");

        let path = hand_off_in(&dir, "id.mac", SystemTime::now()).expect("expected a handoff");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| {
                fs::metadata(path)
                    .expect("expected it to be there")
                    .permissions()
                    .mode()
                    & 0o777
            };
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(&dir), 0o700);
        }
        assert_eq!(take_handed_off(&path).expect("expected the key"), "id.mac");
        assert!(
            !path.exists(),
            "the handoff should be deleted as it is read"
        );
        let error = take_handed_off(&path).expect_err("a handoff is taken once");
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "got {error}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A window that never started leaves its handoff behind; the next one written sweeps it,
    /// and leaves one that may still be on its way alone.
    #[test]
    fn a_handoff_no_window_took_is_swept_by_the_next() {
        let dir = scratch("stale");
        let abandoned = hand_off_in(&dir, "abandoned", SystemTime::now()).expect("expected one");
        let recent = hand_off_in(&dir, "recent", SystemTime::now()).expect("expected one");

        let later = SystemTime::now() + STALE_AFTER + Duration::from_secs(1);
        let next = hand_off_in(&dir, "next", later).expect("expected one");

        assert!(!abandoned.exists() && !recent.exists());
        assert!(next.exists());

        let fresh = hand_off_in(&dir, "fresh", SystemTime::now()).expect("expected one");
        assert!(
            next.exists(),
            "a handoff minutes old is left for its window"
        );
        assert!(fresh.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
