//! Browser account - Rust-owned startup identity and serialized automatic layout writes.
use crate::api::profiles::{DeviceView, Layout, ProfileView, entry_layout, entry_repo};
use serde_json::json;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::spawn_local;

thread_local! { static IDENTITY: RefCell<Option<String>> = const { RefCell::new(None) }; }
pub(crate) fn expected_profile() -> Option<String> {
    IDENTITY.with(|id| id.borrow().clone())
}
#[derive(Clone)]
pub(crate) struct Account(pub Rc<RefCell<State>>);
pub(crate) struct State {
    pub profile: ProfileView,
    pub active: Layout,
    captured: bool,
    pub stopped: bool,
    pub busy: bool,
    pub error: String,
    pub device: Option<DeviceView>,
    queue: VecDeque<Action>,
    saved: Option<Layout>,
    next_verify: f64,
    next_poll: f64,
    expires: f64,
}
pub(crate) use crate::api::profiles::Action;

fn browser() -> web_sys::Window {
    web_sys::window().expect("browser window")
}
fn failed(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}
async fn request(
    profile: Option<&str>,
    path: &str,
    method: &str,
    body: Option<serde_json::Value>,
) -> Result<String, String> {
    super::activity::wait().await;
    send_request(profile, path, method, body).await
}
async fn send_request(
    profile: Option<&str>,
    path: &str,
    method: &str,
    body: Option<serde_json::Value>,
) -> Result<String, String> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    super::http::send(
        method,
        &format!("/api/me{path}"),
        body.as_ref().map(|b| b.to_string()).as_deref(),
        profile,
        30_000,
        true,
        move |status, text| {
            let answer = if (200..300).contains(&status) {
                Ok(text)
            } else {
                let reason = match (status, text.as_str()) {
                    (401, "") => "Admission expired; reload this window",
                    (409, "") => "Account changed; reload this window",
                    _ => &text,
                };
                Err(format!("{status}: {reason}"))
            };
            let _ = sender.send(answer);
        },
    )
    .map_err(failed)?;
    receiver.await.map_err(|e| e.to_string())?
}
fn normalized(mut window: Layout) -> Layout {
    if !matches!(window.frame.as_str(), "review" | "tasks" | "shell") {
        window.frame = "tasks".into();
    }
    window
}
impl Account {
    pub async fn prepare() -> Result<Self, JsValue> {
        let text = request(None, "", "GET", None)
            .await
            .map_err(|e| JsValue::from_str(&e))?;
        let mut profile: ProfileView =
            serde_json::from_str(&text).map_err(|e| JsValue::from_str(&e.to_string()))?;
        IDENTITY.with(|id| *id.borrow_mut() = Some(profile.profile_namespace.clone()));
        let saved = profile.layout.take();
        let location = browser().location();
        let params = web_sys::UrlSearchParams::new_with_str(&location.search()?)?;
        let repo = entry_repo(
            saved.as_ref(),
            params.get("repo"),
            params.has("default_repo"),
        );
        let active = normalized(entry_layout(saved.as_ref(), repo, params.get("frame")));
        params.delete("window");
        params.delete("default_repo");
        params.set("frame", &active.frame);
        if let Some(repo) = &active.repo {
            params.set("repo", repo);
        } else {
            params.delete("repo");
        }
        browser().history()?.replace_state_with_url(
            &JsValue::NULL,
            "",
            Some(&format!("{}?{}", location.pathname()?, params.to_string())),
        )?;
        let account = Self(Rc::new(RefCell::new(State {
            profile,
            active,
            captured: false,
            stopped: false,
            busy: false,
            error: String::new(),
            device: None,
            queue: VecDeque::new(),
            saved,
            next_verify: 0.,
            next_poll: 0.,
            expires: 0.,
        })));
        account.install_pagehide();
        Ok(account)
    }
    fn install_pagehide(&self) {
        let account = self.clone();
        let listener = Closure::<dyn FnMut()>::new(move || {
            let state = account.0.borrow();
            if !state.captured
                || state.busy
                || state.stopped
                || state.saved.as_ref() == Some(&state.active)
            {
                return;
            }
            let Some(profile_id) = &state.profile.id else {
                return;
            };
            // Never race an account mutation already in flight. Keepalive preserves this last
            // complete snapshot after the browser tears down the WASM runner.
            let options = web_sys::RequestInit::new();
            options.set_method("PUT");
            // web-sys does not currently expose RequestInit.keepalive. Set this browser
            // option directly while all persistence decisions remain in Rust.
            if js_sys::Reflect::set(options.as_ref(), &"keepalive".into(), &true.into()).is_err() {
                return;
            }
            options.set_body(&JsValue::from_str(
                &json!({"profile_id":profile_id,"layout":state.active}).to_string(),
            ));
            if let Ok(request) = web_sys::Request::new_with_str_and_init("/api/me/layout", &options)
            {
                let headers = request.headers();
                if headers.set("Content-Type", "application/json").is_ok()
                    && headers
                        .set("x-moon-profile", &state.profile.profile_namespace)
                        .is_ok()
                {
                    let _ = browser().fetch_with_request(&request);
                }
            }
        });
        if browser()
            .add_event_listener_with_callback("pagehide", listener.as_ref().unchecked_ref())
            .is_ok()
        {
            listener.forget();
        }
    }
    pub fn layout(&self) -> Option<String> {
        self.0.borrow().active.layout.clone()
    }
    pub fn save_layout(&self, layout: String, repo: Option<String>, frame: String) {
        let mut state = self.0.borrow_mut();
        if state.stopped {
            return;
        }
        state.captured = true;
        state.active.layout = Some(layout);
        state.active.repo = repo;
        state.active.frame = frame;
        drop(state);
        self.dispatch(Action::Save);
    }
    pub fn tick(&self) {
        if !super::activity::active() {
            return;
        }
        let now = js_sys::Date::now();
        let mut state = self.0.borrow_mut();
        if state.stopped || state.busy {
            return;
        }
        let action = if state.device.is_some() && now >= state.next_poll {
            if now >= state.expires {
                state.device = None;
                state.error = "GitHub connection expired. Try again.".into();
                None
            } else {
                state.next_poll =
                    now + state.device.as_ref().unwrap().interval.max(5) as f64 * 1000.;
                Some(Action::Poll(state.device.as_ref().unwrap().flow_id.clone()))
            }
        } else if now >= state.next_verify {
            state.next_verify = now + 15000.;
            Some(Action::Refresh)
        } else {
            None
        };
        drop(state);
        if let Some(action) = action {
            self.dispatch(action);
        }
    }
    pub fn cancel_device(&self) {
        self.0.borrow_mut().device = None;
    }
    pub fn dispatch(&self, action: Action) {
        let mut state = self.0.borrow_mut();
        if state.stopped {
            return;
        }
        if matches!(action, Action::Device) {
            if state.busy {
                return;
            }
            state.error.clear();
            // Open during the sign-in click, before the async queue loses user activation.
            // The server only accepts this canonical GitHub verification URL.
            let _ = browser().open_with_url_and_target_and_features(
                "https://github.com/login/device",
                "_blank",
                "noopener,noreferrer",
            );
        }
        crate::api::profiles::enqueue(&mut state.queue, action);
        if state.busy {
            return;
        }
        state.busy = true;
        drop(state);
        let account = self.clone();
        spawn_local(async move {
            loop {
                let action = account.0.borrow_mut().queue.pop_front();
                let Some(action) = action else { break };
                if account.0.borrow().stopped {
                    break;
                }
                if let Err(error) = account.perform(action).await {
                    let mut state = account.0.borrow_mut();
                    if error.starts_with("401:") || error.starts_with("409:") {
                        state.stopped = true;
                        state.queue.clear();
                    }
                    state.error = error;
                    // A failed save must not be followed by queued mutations.
                    state.queue.clear();
                    break;
                }
            }
            account.0.borrow_mut().busy = false;
        });
    }
    async fn perform(&self, action: Action) -> Result<(), String> {
        let profile = self.0.borrow().profile.clone();
        let namespace = Some(profile.profile_namespace.as_str());
        match action {
            Action::Refresh => {
                // The pinned identity is checked by auth middleware. No layout or account
                // data needs refreshing in the single-layout UI.
                request(namespace, "", "HEAD", None).await?;
            }
            Action::Disconnect => {
                request(namespace, "/disconnect", "POST", Some(json!({}))).await?;
                self.stop();
                Self::home();
            }
            Action::Device => {
                // Opening GitHub may blur Moon. Finish this explicit sign-in request;
                // subsequent approval polling still waits for Moon to regain focus.
                let text =
                    send_request(namespace, "/github/device", "POST", Some(json!({}))).await?;
                let flow: DeviceView = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let mut state = self.0.borrow_mut();
                state.next_poll = js_sys::Date::now() + flow.interval.max(5) as f64 * 1000.;
                state.expires = js_sys::Date::now() + flow.expires_in as f64 * 1000.;
                state.device = Some(flow);
            }
            Action::Poll(flow_id) => {
                let text = request(
                    namespace,
                    "/github/poll",
                    "POST",
                    Some(json!({"flow_id":flow_id})),
                )
                .await?;
                if self
                    .0
                    .borrow()
                    .device
                    .as_ref()
                    .is_some_and(|flow| flow.flow_id == flow_id)
                    && serde_json::from_str::<serde_json::Value>(&text)
                        .map_err(|e| e.to_string())?["connected"]
                        == true
                {
                    self.stop();
                    Self::home();
                }
            }
            Action::Save => {
                let Some(profile_id) = profile.id else {
                    return Ok(());
                };
                let layout = {
                    let state = self.0.borrow();
                    if state.saved.as_ref() == Some(&state.active) {
                        return Ok(());
                    }
                    state.active.clone()
                };
                request(
                    namespace,
                    "/layout",
                    "PUT",
                    Some(json!({"profile_id":profile_id,"layout":layout})),
                )
                .await?;
                self.0.borrow_mut().saved = Some(layout);
            }
        }
        Ok(())
    }
    fn stop(&self) {
        let mut state = self.0.borrow_mut();
        state.stopped = true;
        state.queue.clear();
    }
    pub fn home() {
        super::activity::when_active(|| {
            if let Ok(path) = browser().location().pathname() {
                let _ = browser().location().assign(&path);
            }
        });
    }
    pub fn reload() {
        let _ = browser().location().reload();
    }
}
