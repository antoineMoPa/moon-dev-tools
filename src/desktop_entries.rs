//! Desktop entries - lists the applications installed on this machine, read from the
//! `.desktop` files their packages install.
//!
//! This is the freedesktop Desktop Entry specification, which is how a packaged program with
//! a window says it is there to be started - on Debian as on every other distribution:
//! `chromium` installs `/usr/share/applications/chromium.desktop`, and a desktop's menu is
//! whatever those files say. So `moon › Applications` is read from them rather than written
//! by hand: see [`folders`] for where they are and which of them speak, and [`entry`] for
//! what one says.
//!
//! The list is the server's, like the desktop its applications are started on - see
//! [`crate::display`] - and like it, only a server on Linux has one. A window asks through
//! [`Backend::installed_applications`](crate::backend::Backend::installed_applications), and
//! a server off Linux answers that it is not on Linux rather than with an empty list.
//!
//! On a server that gives each person a Unix user - see [`crate::unix_users`] - the list is
//! the person's who asks, as the desktop is: read with their home and their `PATH`, as them.
//!
//! The files are read each time a window asks. There are a few dozen of them, of a few
//! kilobytes each, and a package installed since the last time is then on the list without
//! anything having to watch the folders.

#[cfg(any(target_os = "linux", test))]
mod entry;
#[cfg(any(target_os = "linux", test))]
mod folders;

use axum::{Extension, Json, extract::State};

use crate::{
    api::{AppError, AppState, applications::InstalledApplications},
    server::{profiles::Profiles, users::UserId},
    unix_users::Person,
};

/// What of the server's environment decides which entries a menu offers, and how they read.
#[cfg(any(target_os = "linux", test))]
struct Session {
    /// The desktops the session calls itself, out of `XDG_CURRENT_DESKTOP`: `GNOME`, `KDE`.
    /// An entry's `OnlyShowIn` and `NotShowIn` are read against them. A server's session
    /// calls itself none, and neither does `moon desktop`'s.
    desktops: Vec<String>,
    /// The locale its messages are in - `fr_CA.UTF-8` - which is the language an entry's
    /// name is read in. `None` when the environment names no locale.
    messages_locale: Option<String>,
    /// `PATH`, where the program a `TryExec` names is looked for.
    path: std::ffi::OsString,
}

/// The variables that name the locale messages are in. The first one that is set decides,
/// which is the order the C library reads them in.
#[cfg(target_os = "linux")]
const MESSAGES_LOCALE_VARIABLES: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

/// The applications installed on this machine that a menu of the server's own user offers,
/// sorted by name.
#[cfg(target_os = "linux")]
pub(crate) fn installed() -> InstalledApplications {
    use std::env;

    offered_to(&Whose {
        home: env::var_os("HOME"),
        data_home: env::var_os("XDG_DATA_HOME"),
        path: env::var_os("PATH").unwrap_or_default(),
    })
}

/// The applications a menu of `person` offers: the entries of their own home before the
/// machine's, and the programs those name looked for on their login `PATH`.
///
/// Read on a thread working as them - see [`crate::unix_users::as_user`] - since their
/// home is theirs alone to read.
#[cfg(target_os = "linux")]
fn installed_for(person: &Person) -> anyhow::Result<InstalledApplications> {
    let whose = Whose {
        home: Some(person.unix_user.home.clone().into_os_string()),
        // `XDG_DATA_HOME` is what a login of theirs says, and the server is none: their
        // entries are read from where they are when it says nothing, under their home.
        data_home: None,
        path: person.unix_user.login_path()?.into(),
    };
    crate::unix_users::as_user(person, || offered_to(&whose))
}

/// Whose menu is read: what of their own environment decides where their entries are, and
/// which of the programs those name are installed for them.
#[cfg(target_os = "linux")]
struct Whose {
    home: Option<std::ffi::OsString>,
    data_home: Option<std::ffi::OsString>,
    path: std::ffi::OsString,
}

/// What a menu offers `whose` it is. The rest is the machine's, and read from the server's
/// own environment for everyone: the desktop the session calls itself, the locale, and the
/// folders the system's entries are in.
#[cfg(target_os = "linux")]
fn offered_to(whose: &Whose) -> InstalledApplications {
    use std::env;

    let session = Session {
        desktops: env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|desktop| !desktop.is_empty())
            .map(str::to_owned)
            .collect(),
        messages_locale: MESSAGES_LOCALE_VARIABLES
            .iter()
            .filter_map(|variable| env::var(variable).ok())
            .find(|locale| !locale.is_empty()),
        path: whose.path.clone(),
    };
    let folders = folders::entries_folders(
        whose.home.clone(),
        whose.data_home.clone(),
        env::var_os("XDG_DATA_DIRS"),
    );
    InstalledApplications::Listed(folders::offered_in(&folders, &session))
}

/// Desktop entries are how Linux lists its applications. The other systems a server runs on
/// have no such files, and no desktop to start an application on - see [`crate::display`].
#[cfg(not(target_os = "linux"))]
pub(crate) fn installed() -> InstalledApplications {
    InstalledApplications::NotOnLinux
}

#[cfg(not(target_os = "linux"))]
fn installed_for(_person: &Person) -> anyhow::Result<InstalledApplications> {
    Ok(InstalledApplications::NotOnLinux)
}

/// `GET /api/applications`: the applications installed on the server, for a window on
/// another machine to offer - those of whoever asks, on a server that gives each person a
/// Unix user.
pub(crate) async fn listed(
    State(state): State<AppState>,
    State(profiles): State<Profiles>,
    Extension(user): Extension<UserId>,
) -> Result<Json<InstalledApplications>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    let person = profiles.session_owner(&user)?.person;
    let installed = tokio::task::spawn_blocking(move || match &person {
        Some(person) => installed_for(person),
        None => Ok(installed()),
    })
    .await??;
    Ok(Json(installed))
}
