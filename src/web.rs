//! Web entry point - starts the window as a web page, which `moon serve` serves at `/moon`.
//!
//! The same window as `moon review --remote`, on the server that served the page. What it
//! opens on comes from the page's address: `?repo=/path/to/repo` for the repo, which the
//! window asks for when it is left off, and `?frame=review` or `?frame=shell` for the review or
//! a shell rather than the task board - the board being where remote work is started from.
//!
//! On a server that gives each person a Unix user, a browser that has not signed in with
//! GitHub gets the sign-in and nothing else - see [`SignInPane`].

pub(crate) mod account;
pub(crate) mod activity;
pub(crate) mod http;

use std::sync::Arc;

use wasm_bindgen::prelude::*;

use crate::{
    api::OpenSessionRequest,
    backend::remote::RemoteBackend,
    cli::{FRAMES, Frame},
    native::{Launch, app, web_account::SignInPane},
};

/// Start the window in `canvas`. Called once, by the page's own script - see `web/index.html`.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    activity::install()?;
    let account = account::Account::prepare().await?;
    // Before a sign-in, such a server refuses everything a window asks of it. So no window is
    // made: only the pane that signs in, and signing in loads the page again.
    if account.0.borrow().profile.only_sign_in_is_offered() {
        return run(canvas, move |ctx| Box::new(SignInPane::new(ctx, account))).await;
    }
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

    run(canvas, move |ctx| {
        let mut app = app::App::new(ctx.clone(), launch);
        app.web_account = Some(account);
        app.restore_web_window(ctx);
        Box::new(app)
    })
    .await
}

/// Give the canvas to what `make` makes, for as long as the page is open.
async fn run(
    canvas: web_sys::HtmlCanvasElement,
    make: impl FnOnce(&egui::Context) -> Box<dyn eframe::App> + 'static,
) -> Result<(), JsValue> {
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |creation| {
                let ctx = creation.egui_ctx.clone();
                std::mem::forget(activity::subscribe(move |active| {
                    if active {
                        ctx.request_repaint();
                    }
                }));
                Ok(make(&creation.egui_ctx))
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
