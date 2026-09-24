//! The window as a page: what `moon serve` hands a browser at `/moon`.
//!
//! The same window as `moon review --remote`, on the server that served the page. What it
//! opens on comes from the page's address: `?repo=/path/to/repo` for the repo, which the
//! window asks for when it is left off, and `?frame=review` or `?frame=shell` for the review or
//! a shell rather than the task board - the board being where remote work is started from.

use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    api::OpenSessionRequest,
    backend::remote::RemoteBackend,
    cli::{FRAMES, Frame},
    native::{Launch, app},
};

/// Start the window in `canvas`. Called once, by the page's own script - see `web/index.html`.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    let location = web_sys::window()
        .ok_or_else(|| JsValue::from_str("moon runs in a window"))?
        .location();
    let origin = location.origin()?;
    let asked = web_sys::UrlSearchParams::new_with_str(&location.search()?)?;

    let frame = match asked.get("frame") {
        Some(named) => frame_named(&named).map_err(|error| JsValue::from_str(&error))?,
        None => Frame::Tasks,
    };
    let backend = RemoteBackend::connect(&origin)
        .map_err(|error| JsValue::from_str(&format!("{error:#}")))?;
    let launch = Launch {
        backend: Arc::new(backend),
        open: asked.get("repo").map(|repo_path| OpenSessionRequest {
            repo_path,
            diff_target: None,
            active_commit: None,
        }),
        frame,
    };

    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |creation| {
                let mut app = app::App::new(creation.egui_ctx.clone(), launch);
                app.restore_layout_from(creation.storage);
                Ok(Box::new(app))
            }),
        )
        .await?;
    // The runner is the page's for as long as the page is open; nothing ever stops it.
    std::mem::forget(runner);
    Ok(())
}

/// The frame a `?frame=` names, by the same word `moon <subcommand>` opens it with.
fn frame_named(named: &str) -> Result<Frame, String> {
    FRAMES
        .iter()
        .copied()
        .find(|frame| frame.subcommand() == named)
        .ok_or_else(|| {
            let known: Vec<&str> = FRAMES.iter().map(|frame| frame.subcommand()).collect();
            format!("?frame={named} is none of {}", known.join(", "))
        })
}

/// Put one of Ghostty's log messages on the console: `env.log` hands the page a pointer and a
/// length into this module's memory, which only the module can read - see `web/ghostty_env.js`,
/// which the page gives this to.
#[wasm_bindgen]
pub fn ghostty_log(ptr: usize, len: usize) {
    // SAFETY: Ghostty passes the address and length of a message it holds in this module's
    // memory for the length of the call, which is the length of this one.
    let message = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    web_sys::console::log_1(&format!("[ghostty] {}", String::from_utf8_lossy(message)).into());
}
