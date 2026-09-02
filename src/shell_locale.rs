//! The locale this window runs in, which is what decides the characters its children can
//! read and write.
//!
//! A window started from a desktop launcher inherits no locale at all: macOS sets one for
//! processes it launches from Terminal.app and from a login shell, but not for an app bundle.
//! With none, the C library falls back to ASCII, and every tool that checks it prints the
//! bytes of anything else rather than the character - `git log` writes an em dash as
//! `<E2><80><94>`, and a tool drawing a box in line-drawing characters writes those out too.
//!
//! It cuts the other way too, and that is the worse half: a tool that is handed UTF-8 and
//! finds no locale reads it in whatever the platform falls back to. On macOS that is
//! MacRoman, so `pbcopy` puts three characters on the pasteboard where an em dash went in,
//! and text copied out of a shell or an agent comes back mangled wherever it is pasted.
//!
//! So the window adopts the user's own locale, spelled with a UTF-8 charset, the way a
//! terminal application does - and adopts it into its own environment rather than onto each
//! command, so that everything it starts inherits it: the shells and the agents in them, the
//! agents it runs headless, `git`, the searcher, and whatever those run in turn.

use std::{env, process::Command, sync::OnceLock};

/// Put that locale in this process's own environment, where every child inherits it.
///
/// Called from `run` before this program has started a thread, which is the only time
/// writing to the environment is sound.
pub(crate) fn adopt_utf8_locale() {
    let Some(lang) = shell_lang() else {
        return;
    };
    // SAFETY: `run` is the first thing either executable does, and nothing here has started
    // a thread yet, so no other thread can be reading the environment.
    unsafe { env::set_var("LANG", lang) };
}

/// What `LANG` is set to, or `None` when the environment already says.
///
/// A locale the user set for themselves is left alone, UTF-8 or not: `LC_ALL=C` is a choice
/// someone makes, and a window is not the place to overrule it.
///
/// The answer is worked out once and kept, so a shell started after [`adopt_utf8_locale`]
/// still names the locale it was started under rather than reading back the one just set.
pub(crate) fn shell_lang() -> Option<&'static str> {
    static LANG: OnceLock<Option<String>> = OnceLock::new();
    LANG.get_or_init(|| {
        if ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .any(|name| env::var_os(name).is_some_and(|value| !value.is_empty()))
        {
            return None;
        }
        utf8_locale()
    })
    .as_deref()
}

/// The UTF-8 locale to use: the one this machine is set to, or a language-neutral one.
///
/// Only locales the C library actually has are offered - naming one it does not have is the
/// same as naming none, and leaves the shell back on ASCII.
fn utf8_locale() -> Option<String> {
    let installed = installed_locales();
    system_locale()
        .map(|locale| format!("{locale}.UTF-8"))
        .into_iter()
        .chain(FALLBACK_LOCALES.iter().map(|locale| (*locale).to_string()))
        .find(|locale| installed.iter().any(|installed| installed == locale))
}

/// Locales to fall back on, best first: one that is only a charset, then American English,
/// which is the one a machine with any locales at all has.
const FALLBACK_LOCALES: [&str; 2] = ["C.UTF-8", "en_US.UTF-8"];

/// The locale this machine is set to, e.g. `fr_CA`, without a charset. `None` where it
/// cannot be read, which is every platform but macOS - there the desktop sets the variables
/// this file is about, so there is nothing to work out.
#[cfg(target_os = "macos")]
fn system_locale() -> Option<String> {
    let output = Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let locale = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!locale.is_empty()).then_some(locale)
}

#[cfg(not(target_os = "macos"))]
fn system_locale() -> Option<String> {
    None
}

/// The locales this machine has, as `locale -a` lists them. Empty where it cannot be run,
/// which leaves the shell's locale alone rather than guessing at one.
fn installed_locales() -> Vec<String> {
    let Ok(output) = Command::new("locale").arg("-a").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: whatever is picked can print a character outside ASCII, so a shell
    /// started in it shows an em dash rather than the bytes of one.
    #[test]
    fn the_locale_a_shell_is_started_in_speaks_utf8() {
        let Some(lang) = utf8_locale() else {
            // A machine with no locales installed at all - nothing to assert about.
            return;
        };
        assert!(
            lang.ends_with(".UTF-8"),
            "a shell's locale should name the UTF-8 charset, picked {lang:?}"
        );
        assert!(
            installed_locales().contains(&lang),
            "a shell's locale should be one this machine has, picked {lang:?}"
        );
    }
}
