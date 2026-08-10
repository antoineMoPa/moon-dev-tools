//! Backend calls run on worker threads and come back as edits to the model.
//!
//! Git commands take tens of milliseconds and a remote backend takes a network round-trip,
//! so nothing the UI thread does is allowed to wait on either. A task is a piece of work
//! plus what to do with its result; the result lands in an inbox the UI drains each frame.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    thread,
};

use anyhow::Result;

use crate::{backend::Backend, native::model::Model};

/// An edit the UI applies to the model once a task finishes.
type Apply = Box<dyn FnOnce(&mut Model) + Send>;

#[derive(Clone)]
pub(crate) struct Tasks {
    backend: Arc<dyn Backend>,
    inbox: Arc<Mutex<Vec<Apply>>>,
    /// Keys of tasks already running, so holding a key down cannot queue a hundred of them.
    inflight: Arc<Mutex<HashSet<String>>>,
    ctx: egui::Context,
}

impl Tasks {
    pub(crate) fn new(backend: Arc<dyn Backend>, ctx: egui::Context) -> Self {
        Self {
            backend,
            inbox: Arc::new(Mutex::new(Vec::new())),
            inflight: Arc::new(Mutex::new(HashSet::new())),
            ctx,
        }
    }

    pub(crate) fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// Run `work` on a worker thread, then hand its result to `apply` on the UI thread.
    pub(crate) fn spawn<T, W, A>(&self, work: W, apply: A)
    where
        T: Send + 'static,
        W: FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        self.spawn_keyed(None, work, apply);
    }

    /// The same, but at most one task per key is in flight at a time. Use it for anything
    /// a repaint or a held key can trigger repeatedly, like polling a review.
    pub(crate) fn spawn_keyed<T, W, A>(&self, key: Option<String>, work: W, apply: A)
    where
        T: Send + 'static,
        W: FnOnce(&dyn Backend) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        if let Some(key) = &key {
            let Ok(mut inflight) = self.inflight.lock() else {
                return;
            };
            if !inflight.insert(key.clone()) {
                return;
            }
        }

        let backend = Arc::clone(&self.backend);
        let inbox = Arc::clone(&self.inbox);
        let inflight = Arc::clone(&self.inflight);
        let ctx = self.ctx.clone();

        thread::spawn(move || {
            let result = work(backend.as_ref());
            if let Some(key) = &key
                && let Ok(mut inflight) = inflight.lock()
            {
                inflight.remove(key);
            }
            if let Ok(mut inbox) = inbox.lock() {
                inbox.push(Box::new(move |model| apply(model, result)));
            }
            // The result is only visible once the UI draws again, and the UI may well be
            // idle waiting for exactly this.
            ctx.request_repaint();
        });
    }

    /// Fire an action whose only interesting outcome is whether it failed, and refresh the
    /// review afterwards so the window shows what the repo now looks like.
    pub(crate) fn act<W>(&self, session_id: &str, context: &'static str, work: W)
    where
        W: FnOnce(&dyn Backend) -> Result<()> + Send + 'static,
    {
        let session_id = session_id.to_string();
        self.spawn(work, move |model, result| {
            model.report(result, context);
            model.review(&session_id).refresh_requested = true;
        });
    }

    /// The same, but says so when it worked.
    ///
    /// Most actions show their own result — a staged hunk moves, a comment appears. Handing
    /// work to an agent does not, so it has to be acknowledged.
    pub(crate) fn act_and_say<W>(
        &self,
        session_id: &str,
        context: &'static str,
        note: String,
        work: W,
    ) where
        W: FnOnce(&dyn Backend) -> Result<()> + Send + 'static,
    {
        let session_id = session_id.to_string();
        self.spawn(work, move |model, result| {
            match result {
                Ok(()) => model.info(note),
                Err(error) => model.error(format!("{context}: {error}")),
            }
            model.review(&session_id).refresh_requested = true;
        });
    }

    pub(crate) fn is_busy(&self, key: &str) -> bool {
        self.inflight
            .lock()
            .map(|inflight| inflight.contains(key))
            .unwrap_or(false)
    }

    /// Apply everything that finished since the last frame.
    pub(crate) fn drain(&self, model: &mut Model) {
        let pending = {
            let Ok(mut inbox) = self.inbox.lock() else {
                return;
            };
            std::mem::take(&mut *inbox)
        };
        for apply in pending {
            apply(model);
        }
    }
}
