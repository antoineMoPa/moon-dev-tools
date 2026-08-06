//! The desktop frontend: one window, and — when it is reviewing the machine it runs on —
//! the review server in the same process and the same executable.

pub(crate) mod app;
pub(crate) mod bindings;
pub(crate) mod board;
pub(crate) mod file_pane;
pub(crate) mod find;
pub(crate) mod fonts;
pub(crate) mod menu;
pub(crate) mod model;
pub(crate) mod palette;
pub(crate) mod panes;
pub(crate) mod review;
pub(crate) mod tasks;
pub(crate) mod theme;
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

/// The window icon: a crescent moon, generated rather than shipped as a file.
///
/// The dock, the task switcher and the title bar all want an icon, and a generated one keeps
/// the executable self-contained — the whole point of this frontend.
fn icon() -> egui::IconData {
    const SIZE: usize = 64;
    // The dark disc and the lighter crescent, taken from the app's own dark palette.
    let backdrop = [0x10u8, 0x14, 0x1c];
    let moon = [0xee_u8, 0x8d, 0x68];

    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let center = (SIZE as f32 - 1.0) / 2.0;
    let disc = center * 0.94;
    // The bite taken out of the disc, offset up and to the right.
    let bite_center = (center + disc * 0.45, center - disc * 0.25);
    let bite = disc * 0.80;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32, y as f32);
            let from_center = ((fx - center).powi(2) + (fy - center).powi(2)).sqrt();
            let from_bite =
                ((fx - bite_center.0).powi(2) + (fy - bite_center.1).powi(2)).sqrt();

            // One pixel of feathering keeps the edges from looking sawn off.
            let inside = (disc - from_center).clamp(0.0, 1.0);
            let outside_bite = (from_bite - bite).clamp(0.0, 1.0);
            let coverage = inside * outside_bite;

            let at = (y * SIZE + x) * 4;
            rgba[at..at + 3].copy_from_slice(if coverage > 0.5 { &moon } else { &backdrop });
            // The backdrop is only drawn where the disc is, so the icon is a round moon.
            rgba[at + 3] = (inside * 255.0) as u8;
        }
    }

    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

pub(crate) fn run(launch: Launch) -> Result<()> {
    let title = format!("🌚 {}", launch.frame.program());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // The crescent stands in for the app's mark, which the tab strip no longer carries.
            .with_title(title)
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([720.0, 420.0])
            .with_app_id("moonreview")
            .with_icon(icon()),
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
