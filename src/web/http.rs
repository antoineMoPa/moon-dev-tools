//! Browser HTTP - shared XHR setup for task rounds and account requests.
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

pub(crate) fn send(
    method: &str,
    url: &str,
    body: Option<&str>,
    profile: Option<&str>,
    timeout: u32,
    no_cache: bool,
    then: impl FnOnce(u16, String) + 'static,
) -> Result<(), JsValue> {
    let xhr = web_sys::XmlHttpRequest::new()?;
    xhr.open_with_async(method, url, true)?;
    if let Some(profile) = profile {
        xhr.set_request_header("x-moon-profile", profile)?;
    }
    if body.is_some() {
        xhr.set_request_header("content-type", "application/json")?;
    }
    xhr.set_timeout(timeout);
    if no_cache {
        xhr.set_request_header("Cache-Control", "no-cache")?;
    }
    let watched = xhr.clone();
    let done = Closure::once(move || {
        let status = watched.status().unwrap_or(0);
        let text = watched.response_text().ok().flatten().unwrap_or_default();
        then(status, text);
    });
    xhr.set_onloadend(Some(done.as_ref().unchecked_ref()));
    if let Err(error) = xhr.send_with_opt_str(body) {
        // A synchronous send failure never fires loadend: release its callback too.
        xhr.set_onloadend(None);
        return Err(error);
    }
    drop(done.into_js_value());
    Ok(())
}
