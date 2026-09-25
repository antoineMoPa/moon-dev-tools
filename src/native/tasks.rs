//! Backend calls run on worker threads and come back as edits to the model.
//!
//! Git commands take tens of milliseconds and a remote backend takes a network round-trip,
//! so nothing the UI thread does is allowed to wait on either. A task is a piece of work
//! plus what to do with its result; the result lands in an inbox the UI drains each frame.
//!
//! In a browser there are no threads to run a task on, and nothing may wait. There a task's
//! work runs in rounds - see [`crate::backend::remote::rounds`]: again from the top each time
//! the answers it asked for are in, until a run asks for nothing new. So work is `Fn`, and
//! asks the same things in the same order every time it runs. Its result lands in the inbox
//! the same as a worker's would.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use anyhow::Result;

use crate::{backend::Backend, native::model::Model};

/// An edit the UI applies to the model once a task finishes.
type Apply = Box<dyn FnOnce(&mut Model) + Send>;

/// A task asked for in a lane, waiting for the one before it to finish.
type Waiting = Box<dyn FnOnce() + Send>;

/// What a running task holds until it finishes.
enum Hold {
    Nothing,
    /// Its key, which no second task can be started under meanwhile - see
    /// [`Tasks::spawn_keyed`].
    Key(String),
    /// Its lane, whose next task starts once it lets go - see [`Tasks::spawn_in_order`].
    Lane(String),
}

/// The way a task edits the model while it is still running, for work that has something
/// to show before it is done - a search streaming its matches in. Each edit lands in the
/// same inbox a finished task's result does, and in the same order.
#[derive(Clone)]
pub(crate) struct ModelEdits {
    inbox: Arc<Mutex<Vec<Apply>>>,
    ctx: egui::Context,
}

impl ModelEdits {
    pub(crate) fn push(&self, edit: impl FnOnce(&mut Model) + Send + 'static) {
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push(Box::new(edit));
        }
        self.ctx.request_repaint();
    }
}

#[derive(Clone)]
pub(crate) struct Tasks {
    backend: Arc<dyn Backend>,
    inbox: Arc<Mutex<Vec<Apply>>>,
    /// Keys of tasks already running, so holding a key down cannot queue a hundred of them.
    inflight: Arc<Mutex<HashSet<String>>>,
    /// Lanes with a task running, each with the tasks asked for behind it, first first.
    lanes: Arc<Mutex<HashMap<String, VecDeque<Waiting>>>>,
    ctx: egui::Context,
}

impl Tasks {
    pub(crate) fn new(backend: Arc<dyn Backend>, ctx: egui::Context) -> Self {
        Self {
            backend,
            inbox: Arc::new(Mutex::new(Vec::new())),
            inflight: Arc::new(Mutex::new(HashSet::new())),
            lanes: Arc::new(Mutex::new(HashMap::new())),
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
        W: Fn(&dyn Backend) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        self.spawn_keyed(None, work, apply);
    }

    /// The same, but at most one task per key is in flight at a time. Use it for anything
    /// a repaint or a held key can trigger repeatedly, like polling a review.
    pub(crate) fn spawn_keyed<T, W, A>(&self, key: Option<String>, work: W, apply: A)
    where
        T: Send + 'static,
        W: Fn(&dyn Backend) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        self.spawn_editing(key, move |backend, _| work(backend), apply);
    }

    /// Run `work` on a worker thread with a way to edit the model as it goes, then hand its
    /// result to `apply` on the UI thread once it is over. In a browser, see the module.
    pub(crate) fn spawn_editing<T, W, A>(&self, key: Option<String>, work: W, apply: A)
    where
        T: Send + 'static,
        W: Fn(&dyn Backend, &ModelEdits) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        let hold = match key {
            None => Hold::Nothing,
            Some(key) => {
                let Ok(mut inflight) = self.inflight.lock() else {
                    return;
                };
                if !inflight.insert(key.clone()) {
                    return;
                }
                Hold::Key(key)
            }
        };
        self.start(hold, work, apply);
    }

    /// The same, but after every task asked for in `lane` before it has finished.
    ///
    /// For writes that each say the whole of something - a card's tags - and are sent faster
    /// than they come back: side by side on worker threads, an earlier one landing last would
    /// be written over the later.
    pub(crate) fn spawn_in_order<T, W, A>(&self, lane: String, work: W, apply: A)
    where
        T: Send + 'static,
        W: Fn(&dyn Backend) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        let tasks = self.clone();
        let hold = Hold::Lane(lane.clone());
        let start: Waiting =
            Box::new(move || tasks.start(hold, move |backend, _| work(backend), apply));
        let mut lanes = self.lanes.lock().expect("the lanes lock was poisoned");
        match lanes.get_mut(&lane) {
            Some(waiting) => waiting.push_back(start),
            None => {
                lanes.insert(lane, VecDeque::new());
                drop(lanes);
                start();
            }
        }
    }

    /// Start the task waiting next in `lane`, or let the lane go when none is.
    fn next_in(&self, lane: &str) {
        let mut lanes = self.lanes.lock().expect("the lanes lock was poisoned");
        let waiting = lanes
            .get_mut(lane)
            .expect("a lane is kept for as long as a task of it runs");
        match waiting.pop_front() {
            Some(start) => {
                drop(lanes);
                start();
            }
            None => {
                lanes.remove(lane);
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start<T, W, A>(&self, hold: Hold, work: W, apply: A)
    where
        T: Send + 'static,
        W: Fn(&dyn Backend, &ModelEdits) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        let tasks = self.clone();
        std::thread::spawn(move || {
            let edits = ModelEdits {
                inbox: Arc::clone(&tasks.inbox),
                ctx: tasks.ctx.clone(),
            };
            let result = work(tasks.backend.as_ref(), &edits);
            tasks.finish(hold, Box::new(move |model| apply(model, result)));
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn start<T, W, A>(&self, hold: Hold, work: W, apply: A)
    where
        T: Send + 'static,
        W: Fn(&dyn Backend, &ModelEdits) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        self.attempt(
            crate::backend::remote::rounds::Round::new(),
            hold,
            work,
            apply,
        );
    }

    /// One run of the work in its round - see the module. The edits a run pushes are held
    /// until the run that counts, so a run that is thrown away leaves none behind.
    #[cfg(target_arch = "wasm32")]
    fn attempt<T, W, A>(
        &self,
        round: crate::backend::remote::rounds::Round,
        hold: Hold,
        work: W,
        apply: A,
    ) where
        T: Send + 'static,
        W: Fn(&dyn Backend, &ModelEdits) -> Result<T> + Send + 'static,
        A: FnOnce(&mut Model, Result<T>) + Send + 'static,
    {
        let edits_held = Arc::new(Mutex::new(Vec::new()));
        let edits = ModelEdits {
            inbox: Arc::clone(&edits_held),
            ctx: self.ctx.clone(),
        };
        let result = round.run(|| work(self.backend.as_ref(), &edits));
        if round.is_waiting() {
            let tasks = self.clone();
            let again = round.clone();
            round.when_answered(move || tasks.attempt(again, hold, work, apply));
            return;
        }

        let held = std::mem::take(&mut *edits_held.lock().expect("a browser has one thread"));
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.extend(held);
        }
        self.finish(hold, Box::new(move |model| apply(model, result)));
    }

    fn finish(&self, hold: Hold, apply: Apply) {
        if let Hold::Key(key) = &hold
            && let Ok(mut inflight) = self.inflight.lock()
        {
            inflight.remove(key);
        }
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push(apply);
        }
        // After its result is in the inbox, so the next one's lands behind it.
        if let Hold::Lane(lane) = &hold {
            self.next_in(lane);
        }
        // The result is only visible once the UI draws again, and the UI may well be
        // idle waiting for exactly this.
        self.ctx.request_repaint();
    }

    /// Fire an action whose only interesting outcome is whether it failed, and refresh the
    /// review afterwards so the window shows what the repo now looks like.
    pub(crate) fn act<W>(&self, session_id: &str, context: &'static str, work: W)
    where
        W: Fn(&dyn Backend) -> Result<()> + Send + 'static,
    {
        let session_id = session_id.to_string();
        self.spawn(work, move |model, result| {
            model.report(result, context);
            model.review(&session_id).refresh_requested = true;
        });
    }

    /// The same, but says so when it worked.
    ///
    /// Most actions show their own result - a staged hunk moves, a comment appears. Handing
    /// work to an agent does not, so it has to be acknowledged.
    pub(crate) fn act_and_say<W>(
        &self,
        session_id: &str,
        context: &'static str,
        note: String,
        work: W,
    ) where
        W: Fn(&dyn Backend) -> Result<()> + Send + 'static,
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
