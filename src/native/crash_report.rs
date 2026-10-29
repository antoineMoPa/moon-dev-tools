//! Crash report - a panic on the window's thread ends the process, and from the Dock its
//! message goes to a stderr nobody reads. So it is also written to `~/.moonreview`, and the
//! next window to open says what took the last one down.
//!
//! Only the main thread counts: a worker that panics takes its own job down, not the window.

use std::{backtrace::Backtrace, fs, path::PathBuf};

const REPORT_FILE_NAME: &str = "last_crash.txt";
/// Where a report goes once a window has shown it, kept so it can still be opened.
const SHOWN_REPORT_FILE_NAME: &str = "last_crash.shown.txt";

/// Write a report on any panic of the main thread, after the default hook has printed it.
pub(crate) fn install() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        if std::thread::current().name() != Some("main") {
            return;
        }
        let Some(path) = report_path(REPORT_FILE_NAME) else {
            return;
        };
        let report = format!(
            "moon {} panicked at unix time {}\n{info}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            crate::native::messages::now_unix(),
            Backtrace::force_capture(),
        );
        if let Err(error) = fs::write(&path, report) {
            eprintln!("[moonreview] could not write the crash report to {path:?}: {error}");
        }
    }));
}

/// The message of the last crash not shown yet, and where its whole report now is. Moving the
/// file is what makes it shown: the window after this one opens quietly.
pub(crate) fn take_unshown() -> Option<String> {
    let path = report_path(REPORT_FILE_NAME)?;
    let report = fs::read_to_string(&path).ok()?;
    let shown = report_path(SHOWN_REPORT_FILE_NAME)?;
    fs::rename(&path, &shown).expect("the crash report was just read from this directory");
    // The second line is the panic's location and message; the first is the version and time.
    let panic = report.lines().nth(1).unwrap_or("").to_string();
    Some(format!(
        "the last window crashed: {panic} - the whole report is in {}",
        shown.display()
    ))
}

fn report_path(file_name: &str) -> Option<PathBuf> {
    Some(crate::settings::moonreview_dir()?.join(file_name))
}
