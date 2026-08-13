//! The desktop frontend: one window, and — when it is reviewing the machine it runs on —
//! the review server in the same process and the same executable.

pub(crate) mod app;
pub(crate) mod bindings;
pub(crate) mod board;
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
mod ui_tests;
pub(crate) mod widgets;
pub(crate) mod workspace;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
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
    pub(crate) serves_web: ServesWeb,
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

    let serves_web = if serve_web {
        spawn_server(state.clone())?
    } else {
        ServesWeb::Never
    };

    Ok(Launch {
        backend: Arc::new(LocalBackend::new(state)),
        open: Some(open),
        serves_web,
        frame,
    })
}

/// Whether this window's review server is the one on the port right now.
#[derive(Clone)]
pub(crate) enum ServesWeb {
    /// No server of its own and none to reach, so there is nothing to offer a browser.
    Never,
    /// The far side serves, and was already serving before this window connected to it.
    Always,
    /// This process serves whenever it holds the port.
    WhileHeld(Arc<AtomicBool>),
}

impl ServesWeb {
    /// Whether a browser pointed at the review port right now would reach this window.
    pub(crate) fn is_serving(&self) -> bool {
        match self {
            ServesWeb::Never => false,
            ServesWeb::Always => true,
            ServesWeb::WhileHeld(held) => held.load(Ordering::Relaxed),
        }
    }

    /// Whether this window is ever the one serving — which is a different question, and the
    /// one the menu bar has to answer.
    pub(crate) fn may_serve(&self) -> bool {
        !matches!(self, ServesWeb::Never)
    }
}

/// How long to wait before looking again at a port another window is holding.
const PORT_RETRY: Duration = Duration::from_secs(2);

/// Serve for as long as this window lives, taking the port whenever it comes free.
fn spawn_server(state: crate::api::AppState) -> Result<ServesWeb> {
    let held = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&held);

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
            // A `MOONREVIEW_PORT` that is not a port is worth saying so about, but it is not
            // worth refusing to open the window over: the window is the frontend that works
            // without a server.
            let port = match crate::api::port() {
                Ok(port) => port,
                Err(error) => {
                    eprintln!("[moonreview] web frontend unavailable: {error}");
                    return;
                }
            };
            runtime.block_on(hold_port(crate::api::bind_host(), port, state, flag));
        })
        .context("failed to start the web server thread")?;

    Ok(ServesWeb::WhileHeld(held))
}

/// Serve on this address for as long as the window lives, taking it whenever it comes free.
async fn hold_port(host: String, port: u16, state: crate::api::AppState, held: Arc<AtomicBool>) {
    loop {
        match server::try_bind_on(&host, port).await {
            Ok(Some(listener)) => {
                println!("Moon Review listening on {}", crate::api::server_url());
                held.store(true, Ordering::Relaxed);
                // No idle timeout: the window decides how long this process lives, not the
                // clock.
                let served = server::serve_on(state.clone(), listener, None).await;
                held.store(false, Ordering::Relaxed);
                if let Err(error) = served {
                    eprintln!("[moonreview] the web frontend stopped: {error}");
                }
            }
            // Another window has it, which is ordinary and not for ever.
            Ok(None) => {}
            Err(error) => {
                // A port this process is never going to get, so there is nothing here worth
                // waiting for.
                eprintln!("[moonreview] web frontend unavailable: {error}");
                return;
            }
        }
        tokio::time::sleep(PORT_RETRY).await;
    }
}

#[cfg(test)]
mod port_tests {
    //! The second window opened has to live without the port until the first one quits, and
    //! then actually take it. Neither half needs a window to check, which is why this does not
    //! open one.

    use super::*;

    fn test_state() -> crate::api::AppState {
        server::build_state(Arc::new(Mutex::new(Instant::now())))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_window_waits_out_the_holder_and_then_takes_the_port() {
        let holder = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("failed to bind the port the test holds");
        let port = holder
            .local_addr()
            .expect("failed to read the held port")
            .port();

        let held = Arc::new(AtomicBool::new(false));
        let waiting = tokio::spawn(hold_port(
            "127.0.0.1".to_string(),
            port,
            test_state(),
            Arc::clone(&held),
        ));

        // The test is on the port, so the window cannot be — and must not have given up.
        tokio::time::sleep(PORT_RETRY * 2).await;
        assert!(
            !held.load(Ordering::Relaxed),
            "the port was held by the test, so the window cannot be serving on it"
        );

        drop(holder);

        let gave_up_at = Instant::now() + PORT_RETRY * 5;
        while Instant::now() < gave_up_at && !held.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            held.load(Ordering::Relaxed),
            "the holder let go, so the window that was waiting should have taken the port"
        );

        waiting.abort();
    }
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
        // The far side is the one serving, and it was before this window connected.
        serves_web: ServesWeb::Always,
        frame,
    })
}

/// The window with nothing to open on: it asks which repo to review, which is what a launcher
/// started from the OS needs, since it starts outside every repo.
pub(crate) fn launch_prompt(frame: crate::cli::Frame) -> Result<Launch> {
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let state = server::build_state(last_activity);
    // The server answers for whichever repo is asked for, so it needs no project up front.
    // Which is what lets a window opened on the launch screen — every window the Window menu
    // and the OS launchers open — still be one an agent can reach.
    let serves_web = spawn_server(state.clone())?;

    Ok(Launch {
        backend: Arc::new(LocalBackend::new(state)),
        open: None,
        serves_web,
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
