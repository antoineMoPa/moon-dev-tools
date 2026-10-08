//! Browser activity - defer network work while the page is hidden or unfocused.
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

type Listener = dyn Fn(bool);
thread_local! {
    static LISTENERS: RefCell<Vec<Weak<Listener>>> = const { RefCell::new(Vec::new()) };
    static WAITING: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn active() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .is_some_and(|document| !document.hidden() && document.has_focus().unwrap_or(false))
}

pub(crate) fn install() -> Result<(), JsValue> {
    let window = web_sys::window().expect("browser window");
    let document = window.document().expect("browser document");
    for name in ["focus", "blur", "pageshow", "pagehide", "visibilitychange"] {
        let listener = Closure::<dyn FnMut()>::new(changed);
        if name == "visibilitychange" {
            document.add_event_listener_with_callback(name, listener.as_ref().unchecked_ref())?;
        } else {
            window.add_event_listener_with_callback(name, listener.as_ref().unchecked_ref())?;
        }
        listener.forget();
    }
    Ok(())
}

fn changed() {
    let active = active();
    let listeners = LISTENERS.with(|stored| {
        let mut stored = stored.borrow_mut();
        stored.retain(|listener| listener.strong_count() > 0);
        stored.iter().filter_map(Weak::upgrade).collect::<Vec<_>>()
    });
    for listener in listeners {
        listener(active);
    }
    if active {
        let waiting = WAITING.with(|waiting| std::mem::take(&mut *waiting.borrow_mut()));
        for work in waiting {
            when_active(work);
        }
    }
}

/// The caller keeps the returned subscription alive; stale subscriptions are pruned.
pub(crate) fn subscribe(listener: impl Fn(bool) + 'static) -> Rc<Listener> {
    let listener: Rc<Listener> = Rc::new(listener);
    LISTENERS.with(|stored| stored.borrow_mut().push(Rc::downgrade(&listener)));
    listener
}

pub(crate) fn when_active(work: impl FnOnce() + 'static) {
    if active() {
        work();
    } else {
        WAITING.with(|waiting| waiting.borrow_mut().push(Box::new(work)));
    }
}

pub(crate) async fn wait() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        when_active(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
