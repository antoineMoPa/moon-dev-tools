//! The desktop frontend: one window, and — when it is reviewing the machine it runs on —
//! the review server in the same process and the same executable.

pub(crate) mod app;
pub(crate) mod bindings;
pub(crate) mod board;
pub(crate) mod commit_pane;
pub(crate) mod file_pane;
pub(crate) mod find;
pub(crate) mod fonts;
pub(crate) mod launchers;
pub(crate) mod logos;
pub(crate) mod menu;
pub(crate) mod model;
pub(crate) mod palette;
pub(crate) mod panes;
mod programs;
pub(crate) mod review;
pub(crate) mod tasks;
pub(crate) mod theme;
#[cfg(test)]
mod commit_pane_tests;
#[cfg(test)]
mod ui_tests;
pub(crate) mod widgets;
pub(crate) mod workspace;

use std::{
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

use anyhow::{Context, Result};

use crate::{
    api::OpenSessionRequest,
    backend::{Backend, local::LocalBackend, remote::RemoteBackend},
    server,
};

pub(crate) struct Launch {
    pub(crate) backend: Arc<dyn Backend>,
    /// The review to open on startup. `None` means ask, which is what a remote connection
    /// does when it was given an address but no path.
    pub(crate) open: Option<OpenSessionRequest>,
    /// Whether a browser can reach this review, which decides if the window offers the link.
    pub(crate) serves_web: bool,
    /// What the window opens on: which of the three executables this is.
    pub(crate) frame: crate::cli::Frame,
}

/// Review the repo on this machine. The window and the web frontend end up sharing one
/// server, in this process, so the same review is open in both.
pub(crate) fn launch_local(
    open: OpenSessionRequest,
    serve_web: bool,
    frame: crate::cli::Frame,
) -> Result<Launch> {
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let state = server::build_state(last_activity);

    if serve_web {
        let served_state = state.clone();
        // No idle timeout: the window decides how long this process lives, not the clock.
        thread::Builder::new()
            .name("moonreview-server".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        eprintln!("[moonreview] could not start the web server: {error}");
                        return;
                    }
                };
                if let Err(error) = runtime.block_on(server::serve(served_state, None)) {
                    // A busy port is the common case, and it is not fatal: the window works
                    // either way, it just cannot also be opened in a browser.
                    eprintln!("[moonreview] web frontend unavailable: {error}");
                }
            })
            .context("failed to start the web server thread")?;
    }

    Ok(Launch {
        backend: Arc::new(LocalBackend::new(state)),
        open: Some(open),
        serves_web: serve_web,
        frame,
    })
}

/// Review a repo on another machine through its `moonreview serve`.
pub(crate) fn launch_remote(
    target: &str,
    repo_path: Option<String>,
    frame: crate::cli::Frame,
) -> Result<Launch> {
    let backend = RemoteBackend::connect(target)?;
    let open = repo_path.map(|repo_path| OpenSessionRequest {
        repo_path,
        diff_target: None,
        active_commit: None,
    });

    Ok(Launch {
        backend: Arc::new(backend),
        open,
        serves_web: true,
        frame,
    })
}

/// The window with nothing to open on: it asks which repo to review, which is what a launcher
/// started from the OS needs, since it starts outside every repo.
pub(crate) fn launch_prompt(frame: crate::cli::Frame) -> Result<Launch> {
    let last_activity = Arc::new(Mutex::new(Instant::now()));

    Ok(Launch {
        backend: Arc::new(LocalBackend::new(server::build_state(last_activity))),
        open: None,
        // Nothing is served until a repo is picked, and the window is the only frontend then.
        serves_web: false,
        frame,
    })
}

pub(crate) fn run(launch: Launch) -> Result<()> {
    // Which project it is on is only known once the session opens, and the window says so
    // then; until then it is named after the executable alone.
    let title = app::window_title(launch.frame, None);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([720.0, 420.0])
            .with_app_id("moonreview")
            // Each executable wears its own logo, which is also what its launcher carries.
            .with_icon(logos::window_icon(launch.frame)),
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "moonreview",
        options,
        Box::new(|creation| {
            let mut app = app::App::new(creation.egui_ctx.clone(), launch);
            app.install_menu();
            app.restore_layout_from(creation.storage);
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("the window could not be opened: {error}"))
}
