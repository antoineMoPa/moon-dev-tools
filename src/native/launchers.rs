//! Desktop launchers: the entry the OS itself offers for each executable.
//!
//! `cargo install` and the install script both leave three plain executables on `PATH`, which
//! is all a shell needs. Spotlight, Launchpad and an application grid need more than that: on
//! macOS an application is a `.app` bundle, and on Linux a `.desktop` file. Both are written
//! here, out of the executables that are already installed beside this one.
//!
//! Each launcher runs the real executable from where it is installed, so upgrading in place -
//! `cargo install`, or rerunning the install script - needs no new launcher.

use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};

use crate::{
    cli::{FRAMES, Frame},
    native::{
        logos::logo_png,
        programs::{executable_for, install_dir},
    },
};

/// A launcher that was written, and where it landed.
pub(crate) struct InstalledLauncher {
    pub(crate) frame: Frame,
    pub(crate) path: PathBuf,
}

/// Write a launcher for every executable installed beside this one.
pub(crate) fn install() -> Result<Vec<InstalledLauncher>> {
    let mut installed = Vec::new();
    for frame in FRAMES {
        let Some(executable) = executable_for(*frame) else {
            continue;
        };
        installed.push(InstalledLauncher {
            frame: *frame,
            path: platform::write_launcher(*frame, &executable)?,
        });
    }

    if installed.is_empty() {
        bail!(
            "no moonreview executables in {} to make launchers for",
            install_dir()?.display()
        );
    }

    Ok(installed)
}

/// The launcher this frame was given, if one was ever written and is still there.
///
/// A window opened through its launcher is opened by the OS, which is what puts it in front
/// with its own icon; opening the plain executable leaves it to arrive wherever it lands.
pub(crate) fn installed_launcher(frame: Frame) -> Option<PathBuf> {
    platform::installed_launcher(frame)
}

/// Where the launchers go, said in one line - the tail of what the CLI and the window report.
pub(crate) fn destination_hint() -> String {
    platform::destination_hint()
}

#[cfg(target_os = "macos")]
mod platform {
    use std::{
        fs,
        os::unix::{ffi::OsStrExt, fs::symlink},
        path::{Path, PathBuf},
    };

    use anyhow::{Context, Result, bail};

    use super::logo_png;
    use crate::cli::Frame;

    /// Where the bundles go, in the order they are wanted: the shared folder every Finder
    /// window and Launchpad shows first, and the per-user one when that cannot be written -
    /// which is the case for an account that is not an administrator.
    const APPLICATIONS_DIRS: &[&str] = &["/Applications", "~/Applications"];

    /// The icon sizes an `.icns` carries, with the four-byte type that names each one. PNG data
    /// under these types is what macOS reads today; the older raw formats are not needed.
    ///
    /// Each point size comes in a 1x and a 2x type, and a Retina display only draws the 2x ones
    /// sharp: `ic11`..`ic14` are 16, 32, 128 and 256 points at 2x, so their PNG data is twice
    /// that many pixels.
    const ICNS_ENTRIES: &[(&[u8; 4], usize)] = &[
        (b"icp5", 32),
        (b"ic11", 32),
        (b"icp6", 64),
        (b"ic12", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic13", 256),
        (b"ic09", 512),
        (b"ic14", 512),
    ];

    pub(super) fn destination_hint() -> String {
        match applications_dir() {
            Ok(dir) => dir.display().to_string(),
            // Nothing to write to is reported by the write itself; the hint just says where.
            Err(_) => APPLICATIONS_DIRS[0].to_string(),
        }
    }

    /// The first of [`APPLICATIONS_DIRS`] this account can write.
    fn applications_dir() -> Result<PathBuf> {
        for candidate in APPLICATIONS_DIRS {
            let dir = match candidate.strip_prefix("~/") {
                Some(under_home) => super::home()?.join(under_home),
                None => PathBuf::from(candidate),
            };
            // Making the directory first, so a `~/Applications` that does not exist yet is
            // still an answer rather than being skipped.
            fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
            if is_writable(&dir) {
                return Ok(dir);
            }
        }

        bail!(
            "none of {} can be written, so there is nowhere to put the bundles",
            APPLICATIONS_DIRS.join(" or ")
        )
    }

    fn is_writable(dir: &Path) -> bool {
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes())
            .expect("a path holds no interior nul byte");
        // SAFETY: the pointer is a nul-terminated string that outlives the call.
        unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
    }

    /// The bundle this frame was given, in whichever of [`APPLICATIONS_DIRS`] it landed in.
    /// Only one carrying our identifier counts: an application of the same name that someone
    /// else installed is not ours to open.
    pub(super) fn installed_launcher(frame: Frame) -> Option<PathBuf> {
        let bundle_name = format!("{}.app", frame.display_name());
        let identifier = format!("com.moonreview.{}", frame.program());
        for candidate in APPLICATIONS_DIRS {
            let dir = match candidate.strip_prefix("~/") {
                Some(under_home) => match super::home() {
                    Ok(home) => home.join(under_home),
                    Err(_) => continue,
                },
                None => PathBuf::from(candidate),
            };
            let bundle = dir.join(&bundle_name);
            if bundle.is_dir() && refuse_foreign_bundle(&bundle, &identifier).is_ok() {
                return Some(bundle);
            }
        }
        None
    }

    /// Build `<display name>.app` around the installed executable.
    pub(super) fn write_launcher(frame: Frame, executable: &Path) -> Result<PathBuf> {
        let bundle_name = format!("{}.app", frame.display_name());
        let bundle = applications_dir()?.join(&bundle_name);
        let identifier = format!("com.moonreview.{}", frame.program());
        refuse_foreign_bundle(&bundle, &identifier)?;

        let macos_dir = bundle.join("Contents/MacOS");
        let resources_dir = bundle.join("Contents/Resources");
        fs::create_dir_all(&macos_dir)
            .with_context(|| format!("failed to create {}", macos_dir.display()))?;
        fs::create_dir_all(&resources_dir)
            .with_context(|| format!("failed to create {}", resources_dir.display()))?;

        fs::write(
            bundle.join("Contents/Info.plist"),
            info_plist(frame, &identifier),
        )
        .context("failed to write Info.plist")?;
        fs::write(
            resources_dir.join(format!("{}.icns", frame.program())),
            icns(frame),
        )
        .context("failed to write the bundle icon")?;

        // The bundle executable is a link to the installed one rather than a copy of it, so
        // `cargo install` over the executable is also an upgrade of what the launcher opens.
        //
        // A shell script that runs it would be the other way to avoid the copy, and macOS
        // refuses to launch one as a bundle executable - `LSOpenURLsWithCompletionHandler()
        // failed with error -10669`. Launch Services follows the link and still reads the
        // bundle around it, so the icon, the name and the bundle id all still come from here.
        let link = macos_dir.join(frame.program());
        if link.symlink_metadata().is_ok() {
            fs::remove_file(&link)
                .with_context(|| format!("failed to replace {}", link.display()))?;
        }
        symlink(executable, &link).with_context(|| format!("failed to link {}", link.display()))?;

        // Finder and the Dock cache a bundle's icon against the bundle's own mtime, and
        // rewriting the files inside it leaves that untouched - so a launcher whose icon
        // changed kept showing the old one until the cache was flushed by hand.
        let _ = std::process::Command::new("touch").arg(&bundle).status();

        // Launch Services reads a bundle when it is first opened, but a bundle rewritten in
        // place can leave it holding the old plist, and Spotlight showing nothing.
        let _ = std::process::Command::new(
            "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
        )
        .arg("-f")
        .arg(&bundle)
        .status();

        remove_bundles_elsewhere(&bundle_name, &bundle, &identifier);
        Ok(bundle)
    }

    /// A bundle an earlier run left in one of the other folders would be a second copy of the
    /// same application, in Spotlight and in Launchpad both, so it goes. Only a bundle carrying
    /// this identifier is touched, and only a failure to delete one is passed over - there is a
    /// working launcher either way.
    fn remove_bundles_elsewhere(bundle_name: &str, written: &Path, identifier: &str) {
        for candidate in APPLICATIONS_DIRS {
            let dir = match candidate.strip_prefix("~/") {
                Some(under_home) => match super::home() {
                    Ok(home) => home.join(under_home),
                    Err(_) => continue,
                },
                None => PathBuf::from(candidate),
            };
            let bundle = dir.join(bundle_name);
            if bundle == written || refuse_foreign_bundle(&bundle, identifier).is_err() {
                continue;
            }
            if bundle.exists() {
                let _ = fs::remove_dir_all(&bundle);
            }
        }
    }

    /// Anything already at that path has to be a bundle this wrote before, so an application
    /// the user installed by other means is never written into.
    fn refuse_foreign_bundle(bundle: &Path, identifier: &str) -> Result<()> {
        if !bundle.exists() {
            return Ok(());
        }

        let plist = bundle.join("Contents/Info.plist");
        let existing = fs::read_to_string(&plist).unwrap_or_default();
        if existing.contains(identifier) {
            return Ok(());
        }

        bail!(
            "{} already exists and is not a moonreview launcher; move it aside first",
            bundle.display()
        );
    }

    fn info_plist(frame: Frame, identifier: &str) -> String {
        let display_name = frame.display_name();
        let program = frame.program();
        let version = env!("MOONREVIEW_VERSION");

        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>{display_name}</string>
	<key>CFBundleDisplayName</key>
	<string>{display_name}</string>
	<key>CFBundleExecutable</key>
	<string>{program}</string>
	<key>CFBundleIdentifier</key>
	<string>{identifier}</string>
	<key>CFBundleIconFile</key>
	<string>{program}.icns</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
"#
        )
    }

    /// An `.icns` of the frame's logo: the four-byte magic, the length of the whole file,
    /// then one length-prefixed entry per size.
    fn icns(frame: Frame) -> Vec<u8> {
        let mut entries = Vec::new();
        for (icon_type, size) in ICNS_ENTRIES {
            let png = logo_png(frame, *size);
            entries.extend_from_slice(*icon_type);
            entries.extend_from_slice(&((png.len() + 8) as u32).to_be_bytes());
            entries.extend_from_slice(png);
        }

        let mut icns = Vec::with_capacity(entries.len() + 8);
        icns.extend_from_slice(b"icns");
        icns.extend_from_slice(&((entries.len() + 8) as u32).to_be_bytes());
        icns.extend_from_slice(&entries);
        icns
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use anyhow::{Context, Result};

    use super::logo_png;
    use crate::cli::Frame;

    pub(super) fn destination_hint() -> String {
        "~/.local/share/applications".to_string()
    }

    /// A desktop entry is a file the desktop reads, not a thing to run, so a new window here
    /// is the executable itself.
    pub(super) fn installed_launcher(_frame: Frame) -> Option<PathBuf> {
        None
    }

    /// The icon size the desktop entry points at. One size is enough: the themed icon
    /// directories are picked by size, and 256 is what launchers scale down from.
    const ICON_SIZE: usize = 256;

    /// Write `<program>.desktop`, and the icon it names.
    pub(super) fn write_launcher(frame: Frame, executable: &Path) -> Result<PathBuf> {
        let data_dir = super::home()?.join(".local/share");
        let icon_dir = data_dir.join(format!("icons/hicolor/{ICON_SIZE}x{ICON_SIZE}/apps"));
        let applications_dir = data_dir.join("applications");
        fs::create_dir_all(&icon_dir)
            .with_context(|| format!("failed to create {}", icon_dir.display()))?;
        fs::create_dir_all(&applications_dir)
            .with_context(|| format!("failed to create {}", applications_dir.display()))?;

        fs::write(
            icon_dir.join(format!("{}.png", frame.program())),
            logo_png(frame, ICON_SIZE),
        )
        .context("failed to write the launcher icon")?;

        let entry = applications_dir.join(format!("{}.desktop", frame.program()));
        fs::write(&entry, desktop_entry(frame, executable))
            .with_context(|| format!("failed to write {}", entry.display()))?;

        // Desktops that cache the application list only notice a new entry once told to look.
        let _ = std::process::Command::new("update-desktop-database")
            .arg(&applications_dir)
            .status();

        Ok(entry)
    }

    fn desktop_entry(frame: Frame, executable: &Path) -> String {
        format!(
            "[Desktop Entry]
Type=Application
Name={display_name}
Comment=Opens on {opens}
Exec={executable} %f
Icon={program}
Terminal=false
Categories=Development;RevisionControl;
StartupWMClass=moonreview
",
            display_name = frame.display_name(),
            opens = frame.opens(),
            executable = executable.display(),
            program = frame.program(),
        )
    }
}

/// Windows and the BSDs have no launcher written here; the executables still run from a shell.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result, bail};

    use crate::cli::Frame;

    pub(super) fn destination_hint() -> String {
        "nowhere on this platform".to_string()
    }

    pub(super) fn installed_launcher(_frame: Frame) -> Option<PathBuf> {
        None
    }

    pub(super) fn write_launcher(_frame: Frame, _executable: &Path) -> Result<PathBuf> {
        bail!("launchers are written for macOS and Linux only");
    }
}

fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set, so there is no place to put the launchers")
}
