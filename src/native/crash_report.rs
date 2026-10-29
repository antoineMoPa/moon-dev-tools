//! Crash report - a panic on the window's thread ends the process, and from the Dock its
//! message goes to a stderr nobody reads. So it is also added to
//! `~/.moonreview/moon crash reports.txt`, and the next window to open says what took the
//! last one down.
//!
//! Only the main thread counts: a worker that panics takes its own job down, not the window.

use std::{backtrace::Backtrace, fs, io::Write, path::PathBuf};

const REPORTS_FILE_NAME: &str = "moon crash reports.txt";
/// Ends the first line of a report no window has shown yet. Showing it takes this off.
const NOT_SHOWN: &str = " - not shown yet";
/// How many of moon's own frames a report keeps: where in moon it went wrong, not the
/// whole stack of egui and winit above it.
const FRAMES_KEPT: usize = 8;

/// Add a report on any panic of the main thread, after the default hook has printed it.
pub(crate) fn install() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        if std::thread::current().name() != Some("main") {
            return;
        }
        let Some(path) = reports_path() else {
            return;
        };
        let report = format!(
            "unix time {} - moon {}{NOT_SHOWN}\n{info}\n{}\n",
            crate::native::messages::now_unix(),
            env!("CARGO_PKG_VERSION"),
            moon_frames(&Backtrace::force_capture().to_string()),
        );
        let written = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(report.as_bytes()));
        if let Err(error) = written {
            eprintln!("[moonreview] could not write the crash report to {path:?}: {error}");
        }
    }));
}

/// The frames of a printed backtrace that are moon's own code, each with the line it is at.
fn moon_frames(backtrace: &str) -> String {
    let lines: Vec<&str> = backtrace.lines().collect();
    lines
        .iter()
        .enumerate()
        // The hook's own frame is this file writing the report, not where the panic was.
        .filter(|(_, line)| line.contains("moonreview::") && !line.contains("crash_report::"))
        .take(FRAMES_KEPT)
        .map(|(index, line)| {
            let at = lines
                .get(index + 1)
                .filter(|next| next.trim_start().starts_with("at "))
                .map(|next| format!("\n{next}"))
                .unwrap_or_default();
            format!("{line}{at}\n")
        })
        .collect()
}

/// What the last crash no window has shown was, marking it shown.
pub(crate) fn take_unshown() -> Option<String> {
    let path = reports_path()?;
    let reports = fs::read_to_string(&path).ok()?;
    let lines: Vec<&str> = reports.lines().collect();
    let unshown = lines.iter().rposition(|line| line.ends_with(NOT_SHOWN))?;
    // Under the report's first line is what the panic printed: `panicked at <location>:`,
    // then its message.
    let panic = lines[unshown + 1..]
        .iter()
        .take(2)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let message = format!("the last window crashed: {panic} - see {}", path.display());
    fs::write(&path, reports.replace(NOT_SHOWN, ""))
        .expect("the crash reports were just read from this file");
    Some(message)
}

fn reports_path() -> Option<PathBuf> {
    Some(crate::settings::moonreview_dir()?.join(REPORTS_FILE_NAME))
}
