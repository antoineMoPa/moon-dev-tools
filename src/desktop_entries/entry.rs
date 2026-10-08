//! Desktop entry - reads one `.desktop` file: whether a menu offers it, what it is called,
//! and the line of shell that starts it.
//!
//! The file is lines of `Key=value` under a `[Desktop Entry]` heading. What each key means
//! is the Desktop Entry specification's, and the ones read here are the ones that decide a
//! menu: `Type`, `Name`, `Exec`, `TryExec`, `Path`, `Categories`, `Icon`, and the keys that
//! take an entry off a menu - `Hidden`, `NoDisplay`, `OnlyShowIn`, `NotShowIn`, `Terminal`.

use std::{
    collections::HashMap,
    env,
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
};

use anyhow::{Context, Result, bail, ensure};

use super::Session;
use crate::{api::applications::InstalledApplication, shell_quoting::single_quoted};

/// The heading the keys of an entry are under, which is the first thing in its file.
const ENTRY_GROUP: &str = "[Desktop Entry]";

/// The `Type` of an entry that is a program to start. The others are a link and a menu's
/// own folder, which the same kind of file describes.
const APPLICATION: &str = "Application";

/// The keys of an entry's `[Desktop Entry]` group, by name. A translation is a key of its
/// own: `Name[fr]`.
type Keys<'a> = HashMap<&'a str, &'a str>;

/// What a field code of an `Exec` line stands for when the entry is started from a menu.
#[derive(Clone, Copy)]
enum FieldCode {
    /// The files or addresses the entry is opened on, of which a menu gives it none - and
    /// the codes the specification has deprecated, which are taken out the same way.
    Nothing,
    /// A `%` of the command's own.
    Percent,
    /// `--icon` and the entry's `Icon`, when it has one.
    Icon,
    /// The entry's name, in the language it is listed in.
    Name,
    /// Where the entry's file is.
    EntryFile,
}

/// Every field code there is, by the letter after its `%`.
const FIELD_CODES: &[(char, FieldCode)] = &[
    ('f', FieldCode::Nothing),
    ('F', FieldCode::Nothing),
    ('u', FieldCode::Nothing),
    ('U', FieldCode::Nothing),
    ('d', FieldCode::Nothing),
    ('D', FieldCode::Nothing),
    ('n', FieldCode::Nothing),
    ('N', FieldCode::Nothing),
    ('v', FieldCode::Nothing),
    ('m', FieldCode::Nothing),
    ('%', FieldCode::Percent),
    ('i', FieldCode::Icon),
    ('c', FieldCode::Name),
    ('k', FieldCode::EntryFile),
];

/// The escapes a value is written with, by the letter after the `\`, and the character each
/// one is.
const ESCAPES: &[(char, char)] = &[
    ('s', ' '),
    ('n', '\n'),
    ('t', '\t'),
    ('r', '\r'),
    ('\\', '\\'),
];

/// Read the entry in `file`: the application it offers a menu, or `None` for an entry that
/// is in order and offers none - hidden, not a program, for another desktop, not installed.
/// An entry the specification does not allow is an error, which says what is wrong with it.
pub(super) fn read(file: &Path, session: &Session) -> Result<Option<InstalledApplication>> {
    let text = fs::read_to_string(file).context("it could not be read")?;
    let keys = keys_of_entry(&text)?;
    let says = |key: &str| -> Result<bool> {
        match keys.get(key).copied() {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            Some(other) => bail!("its {key} is `{other}`, which is neither true nor false"),
        }
    };

    // A hidden entry is one that has been deleted, and is often this one key and no other:
    // it is how a person takes an entry of the system's off their own menu.
    if says("Hidden")? {
        return Ok(None);
    }
    let kind = *keys.get("Type").context("it has no Type")?;
    if kind != APPLICATION || says("NoDisplay")? {
        return Ok(None);
    }
    // A program that draws in a terminal - vim, htop - has no window of its own to show,
    // and moon has no call that opens a shell on a line of somebody else's writing. It is
    // also a word typed in a shell away, which is where its user already is.
    if says("Terminal")? {
        return Ok(None);
    }
    if !is_for_desktops(&keys, &session.desktops) {
        return Ok(None);
    }
    if let Some(program) = keys.get("TryExec")
        && !is_installed(program, &session.path)
    {
        return Ok(None);
    }
    let Some(exec) = keys.get("Exec") else {
        // Started by its name on D-Bus rather than by a command, which moon does not do.
        ensure!(says("DBusActivatable")?, "it has no Exec");
        return Ok(None);
    };

    let name = unescaped(localized(&keys, "Name", session).context("it has no Name")?);
    let mut command = command_line(&unescaped(exec), &name, keys.get("Icon").copied(), file)?;
    // The folder the entry has its program run in, for the few that mind which.
    if let Some(folder) = keys.get("Path").filter(|folder| !folder.is_empty()) {
        command = format!("cd {} && {command}", single_quoted(&unescaped(folder)));
    }
    Ok(Some(InstalledApplication {
        name,
        command,
        categories: items(keys.get("Categories").copied().unwrap_or_default())
            .map(str::to_owned)
            .collect(),
    }))
}

/// The keys of the file's `[Desktop Entry]` group. The groups after it are the entry's
/// actions - `[Desktop Action new-window]` - which a menu of applications does not list.
fn keys_of_entry(text: &str) -> Result<Keys<'_>> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    ensure!(
        lines.next() == Some(ENTRY_GROUP),
        "it does not start with {ENTRY_GROUP}"
    );
    let mut keys = Keys::new();
    for line in lines.take_while(|line| !line.starts_with('[')) {
        let (key, value) = line
            .split_once('=')
            .with_context(|| format!("its line `{line}` is neither a heading nor a key"))?;
        keys.insert(key.trim_end(), value.trim_start());
    }
    Ok(keys)
}

/// The items of a value that is a list: apart by `;`, with or without one after the last.
fn items(list: &str) -> impl Iterator<Item = &str> {
    list.split(';').filter(|item| !item.is_empty())
}

/// The value of a key in the session's language: the most particular translation the entry
/// has, and the key as it is written without one when it has none.
fn localized<'a>(keys: &Keys<'a>, key: &str, session: &Session) -> Option<&'a str> {
    session
        .messages_locale
        .iter()
        .flat_map(|locale| translation_tags(locale))
        .find_map(|tag| keys.get(format!("{key}[{tag}]").as_str()))
        .or_else(|| keys.get(key))
        .copied()
}

/// What a translation of a key may be tagged with for this locale, the most particular
/// first - the order the specification matches them in. For `sr_RS.UTF-8@latin`:
/// `sr_RS@latin`, `sr_RS`, `sr@latin`, `sr`. The encoding is never part of a tag.
fn translation_tags(locale: &str) -> Vec<String> {
    let (locale, modifier) = match locale.split_once('@') {
        Some((locale, modifier)) => (locale, Some(modifier)),
        None => (locale, None),
    };
    let locale = locale.split_once('.').map_or(locale, |(locale, _encoding)| locale);
    let (language, country) = match locale.split_once('_') {
        Some((language, country)) => (language, Some(country)),
        None => (locale, None),
    };

    let mut tags = Vec::new();
    if let (Some(country), Some(modifier)) = (country, modifier) {
        tags.push(format!("{language}_{country}@{modifier}"));
    }
    if let Some(country) = country {
        tags.push(format!("{language}_{country}"));
    }
    if let Some(modifier) = modifier {
        tags.push(format!("{language}@{modifier}"));
    }
    tags.push(language.to_owned());
    tags
}

/// Whether the entry is for a session that calls itself these desktops: `OnlyShowIn` lists
/// the only ones it is for, `NotShowIn` the ones it is not for. A session that calls itself
/// none is none of the ones an `OnlyShowIn` lists, so it is offered no other desktop's own
/// settings panels.
fn is_for_desktops(keys: &Keys<'_>, desktops: &[String]) -> bool {
    let lists_one_of_them = |key: &str| {
        keys.get(key)
            .map(|listed| items(listed).any(|listed| desktops.iter().any(|ours| ours == listed)))
    };
    lists_one_of_them("OnlyShowIn").unwrap_or(true)
        && !lists_one_of_them("NotShowIn").unwrap_or(false)
}

/// Whether the program a `TryExec` names is there to be run - how an entry that outlives
/// its program, or is installed ahead of it, stays off the menu. A path is looked at as it
/// is, a bare name in each folder of `PATH`.
fn is_installed(program: &str, path: &OsStr) -> bool {
    let can_be_run = |file: &Path| {
        fs::metadata(file)
            .is_ok_and(|found| found.is_file() && found.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return can_be_run(Path::new(program));
    }
    env::split_paths(path).any(|folder| can_be_run(&folder.join(program)))
}

/// A value with its escapes read: `\s` is a space, `\\` a backslash - see [`ESCAPES`].
///
/// A backslash before anything else is left as it is written. Entries write `\"` and `\$`
/// in an `Exec` line the way a shell has them, and a shell is what the line is handed to.
fn unescaped(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        let escaped = match (character, characters.peek()) {
            ('\\', Some(letter)) => ESCAPES.iter().find(|(known, _)| known == letter),
            _ => None,
        };
        match escaped {
            Some((_, stands_for)) => {
                text.push(*stands_for);
                characters.next();
            }
            None => text.push(character),
        }
    }
    text
}

/// The line of shell that starts an entry from a menu: its `Exec`, with each field code
/// replaced by what it stands for when there is no file to open - see [`FIELD_CODES`].
///
/// An `Exec` line is quoted the way `sh` quotes: arguments apart by spaces, one that holds
/// a character the shell reads between double quotes, and `\` before a `"`, `` ` ``, `$` or
/// `\` inside them. So outside its field codes it is a line of shell already, and is left
/// as it is written. What takes a field code's place is a value rather than something
/// written for a shell, so it is quoted for one.
fn command_line(exec: &str, name: &str, icon: Option<&str>, file: &Path) -> Result<String> {
    let mut line = String::with_capacity(exec.len());
    let mut characters = exec.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            line.push(character);
            continue;
        }
        let letter = characters.next().context("its Exec ends in a `%`")?;
        let (_, stands_for) = FIELD_CODES
            .iter()
            .find(|(known, _)| *known == letter)
            .with_context(|| format!("its Exec has `%{letter}`, which is no field code"))?;
        match stands_for {
            FieldCode::Nothing => {}
            FieldCode::Percent => line.push('%'),
            FieldCode::Icon => {
                if let Some(icon) = icon.filter(|icon| !icon.is_empty()) {
                    line.push_str("--icon ");
                    line.push_str(&single_quoted(icon));
                }
            }
            FieldCode::Name => line.push_str(&single_quoted(name)),
            FieldCode::EntryFile => line.push_str(&single_quoted(
                file.to_str().context("its path is not UTF-8")?,
            )),
        }
    }
    // `chromium %U` leaves the space that was before its code.
    let line = line.trim();
    ensure!(!line.is_empty(), "its Exec names no program");
    Ok(line.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            desktops: Vec::new(),
            messages_locale: None,
            path: "/usr/bin:/bin".into(),
        }
    }

    /// Read an entry of this text, as a file in a scratch folder of the test's own.
    fn read_text(test: &str, text: &str, session: &Session) -> Result<Option<InstalledApplication>> {
        let file = env::temp_dir().join(format!(
            "moonreview-desktop-entry-{}-{test}.desktop",
            std::process::id()
        ));
        fs::write(&file, text).expect("the entry's file");
        let read = read(&file, session);
        fs::remove_file(&file).expect("the entry's file removed");
        read
    }

    /// What Debian's chromium package installs, less its translations but one.
    const CHROMIUM: &str = "\
[Desktop Entry]
Version=1.0
Name=Chromium Web Browser
Name[fr]=Navigateur Web Chromium
GenericName=Web Browser
# A comment, and a blank line after it.

Exec=/usr/bin/chromium %U
Terminal=false
Type=Application
Icon=chromium
Categories=Network;WebBrowser;
StartupNotify=true

[Desktop Action new-window]
Name=New Window
Exec=/usr/bin/chromium --new-window
";

    #[test]
    fn an_entry_reads_as_its_name_its_command_and_its_categories() {
        let read = read_text("chromium", CHROMIUM, &session()).expect("an entry in order");

        assert_eq!(
            read,
            Some(InstalledApplication {
                name: "Chromium Web Browser".to_owned(),
                command: "/usr/bin/chromium".to_owned(),
                categories: vec!["Network".to_owned(), "WebBrowser".to_owned()],
            })
        );
    }

    #[test]
    fn the_name_is_the_most_particular_translation_the_entry_has() {
        let in_french = Session {
            messages_locale: Some("fr_CA.UTF-8".to_owned()),
            ..session()
        };
        let in_german = Session {
            messages_locale: Some("de_DE.UTF-8".to_owned()),
            ..session()
        };

        let french = read_text("french", CHROMIUM, &in_french).expect("an entry in order");
        let german = read_text("german", CHROMIUM, &in_german).expect("an entry in order");

        assert_eq!(french.expect("offered").name, "Navigateur Web Chromium");
        assert_eq!(german.expect("offered").name, "Chromium Web Browser");
        assert_eq!(
            translation_tags("sr_RS.UTF-8@latin"),
            ["sr_RS@latin", "sr_RS", "sr@latin", "sr"]
        );
    }

    #[test]
    fn an_entry_that_is_in_order_and_not_for_a_menu_offers_nothing() {
        let entry = |more: &str| {
            format!("[Desktop Entry]\nType=Application\nName=Clock\nExec=xclock\n{more}\n")
        };
        let not_offered = [
            ("no-display", entry("NoDisplay=true")),
            ("hidden", "[Desktop Entry]\nHidden=true\n".to_owned()),
            ("terminal", entry("Terminal=true")),
            ("other-desktop", entry("OnlyShowIn=GNOME;KDE;")),
            ("not-installed", entry("TryExec=moonreview-no-such-program")),
            ("not-installed-path", entry("TryExec=/no/such/program")),
            (
                "link",
                "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.com\n".to_owned(),
            ),
            (
                "dbus",
                "[Desktop Entry]\nType=Application\nName=Files\nDBusActivatable=true\n".to_owned(),
            ),
        ];

        for (test, text) in not_offered {
            let read = read_text(test, &text, &session()).expect("an entry in order");
            assert_eq!(read, None, "{test}");
        }

        let installed = read_text("installed", &entry("TryExec=sh"), &session());
        assert!(installed.expect("an entry in order").is_some());
    }

    #[test]
    fn an_entry_is_offered_to_the_desktops_it_is_for() {
        let on_gnome = Session {
            desktops: vec!["ubuntu".to_owned(), "GNOME".to_owned()],
            ..session()
        };
        let entry = |more: &str| {
            format!("[Desktop Entry]\nType=Application\nName=Panel\nExec=panel\n{more}\n")
        };

        let only = read_text("only", &entry("OnlyShowIn=GNOME;"), &on_gnome);
        let not = read_text("not", &entry("NotShowIn=GNOME;"), &on_gnome);
        let not_elsewhere = read_text("not-elsewhere", &entry("NotShowIn=GNOME;"), &session());

        assert!(only.expect("an entry in order").is_some());
        assert!(not.expect("an entry in order").is_none());
        assert!(not_elsewhere.expect("an entry in order").is_some());
    }

    #[test]
    fn an_entry_the_specification_does_not_allow_says_what_is_wrong_with_it() {
        let broken = [
            ("no-group", "Name=Clock\n", "does not start with"),
            ("no-type", "[Desktop Entry]\nName=Clock\nExec=xclock\n", "no Type"),
            ("no-name", "[Desktop Entry]\nType=Application\nExec=xclock\n", "no Name"),
            ("no-exec", "[Desktop Entry]\nType=Application\nName=Clock\n", "no Exec"),
            (
                "not-a-key",
                "[Desktop Entry]\nType=Application\nName=Clock\nxclock\n",
                "neither a heading nor a key",
            ),
            (
                "not-a-boolean",
                "[Desktop Entry]\nType=Application\nName=Clock\nExec=xclock\nHidden=yes\n",
                "neither true nor false",
            ),
            (
                "not-a-field-code",
                "[Desktop Entry]\nType=Application\nName=Clock\nExec=xclock %z\n",
                "no field code",
            ),
        ];

        for (test, text, wrong) in broken {
            let error = read_text(test, text, &session()).expect_err(test);
            assert!(format!("{error:#}").contains(wrong), "{test}: {error:#}");
        }
    }

    #[test]
    fn a_field_code_is_replaced_by_what_it_stands_for_with_no_file_to_open() {
        let file = Path::new("/usr/share/applications/it's.desktop");
        let line = |exec: &str, icon: Option<&str>| {
            command_line(exec, "Text Editor", icon, file).expect("an Exec in order")
        };

        assert_eq!(line("gedit %F", None), "gedit");
        assert_eq!(line("vlc --started-from-file %U", None), "vlc --started-from-file");
        assert_eq!(line("printf 100%% %f", None), "printf 100%");
        assert_eq!(
            line("kate %i -caption %c", Some("kate")),
            "kate --icon 'kate' -caption 'Text Editor'"
        );
        assert_eq!(line("kate %i -b", None), "kate  -b");
        assert_eq!(
            line("launch %k", None),
            r"launch '/usr/share/applications/it'\''s.desktop'"
        );
        // What the entry quoted for a shell stays quoted the way it wrote it.
        assert_eq!(
            line(r#"sh -c "echo \"$HOME\" | wc" %u"#, None),
            r#"sh -c "echo \"$HOME\" | wc""#
        );
    }

    #[test]
    fn the_escapes_of_a_value_are_read_and_a_shell_s_are_left() {
        assert_eq!(unescaped(r"a\sb\tc\\d"), "a b\tc\\d");
        assert_eq!(unescaped(r#"sh -c "echo \"\$HOME\"""#), r#"sh -c "echo \"\$HOME\"""#);
        assert_eq!(unescaped(r"ends in \"), r"ends in \");
    }

    #[test]
    fn an_entry_with_a_folder_of_its_own_is_started_in_it() {
        let text = "[Desktop Entry]\nType=Application\nName=Game\nExec=./game %f\nPath=/opt/a game\n";

        let read = read_text("path", text, &session()).expect("an entry in order");

        assert_eq!(read.expect("offered").command, "cd '/opt/a game' && ./game");
    }
}
