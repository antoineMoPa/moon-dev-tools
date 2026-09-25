//! The shells the server is running, looked up by id: named, attached to, asked what they are
//! doing and whether they want a person, and taken away.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use tokio::sync::broadcast;

use crate::api::TerminalAttentionView;

use super::{OwnedShell, TerminalProgram, TerminalRegistry, TerminalSession};

impl TerminalRegistry {
    pub(crate) fn new(last_activity: Arc<Mutex<Instant>>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            run: crate::moontasks::store::new_uuid()[..8].to_string(),
            next_id: AtomicU64::new(0),
            last_activity,
        }
    }

    pub(super) fn get(&self, terminal_id: &str) -> Option<Arc<TerminalSession>> {
        self.sessions.lock().unwrap().get(terminal_id).cloned()
    }

    /// What a shell is called, if it has been named - see [`TerminalSession::name`]. `None`
    /// for a shell that has not been, and for one the server no longer has.
    pub(crate) fn name(&self, terminal_id: &str) -> Option<String> {
        self.get(terminal_id)?.name.lock().unwrap().clone()
    }

    /// The names of every shell the server has, whoever owns it.
    pub(crate) fn live_names(&self) -> Vec<String> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter_map(|session| session.name.lock().unwrap().clone())
            .collect()
    }

    /// The task a shell belongs to, if it is a task's. `None` for a workspace shell, and for
    /// one the server no longer has.
    pub(crate) fn owner(&self, terminal_id: &str) -> Option<String> {
        self.get(terminal_id)?.owner.clone()
    }

    /// Call a shell something else. A blank name is refused: a tab has to read as something.
    pub(crate) fn rename(&self, terminal_id: &str, name: &str) -> anyhow::Result<()> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("a shell's name cannot be empty");
        }
        let session = self
            .get(terminal_id)
            .ok_or_else(|| anyhow::anyhow!("unknown terminal {terminal_id}"))?;
        *session.name.lock().unwrap() = Some(name.to_string());
        Ok(())
    }

    /// Attach the native window to a shell: everything it has printed so far, then
    /// everything it prints from here on, delivered to whichever thread owns the
    /// terminal emulator. Web tabs attach to the same shell over a websocket.
    pub(crate) fn attach(
        &self,
        terminal_id: &str,
    ) -> anyhow::Result<(std::sync::mpsc::Receiver<Vec<u8>>, Arc<TerminalSession>)> {
        let session = self
            .get(terminal_id)
            .ok_or_else(|| anyhow::anyhow!("unknown terminal {terminal_id}"))?;
        // Subscribe before replaying so nothing written in between is lost.
        let mut output = session.output.subscribe();
        let replay = session.scrollback.lock().unwrap().replay();

        let (sender, receiver) = std::sync::mpsc::channel();
        if !replay.is_empty() {
            let _ = sender.send(replay);
        }

        std::thread::spawn(move || {
            loop {
                match output.blocking_recv() {
                    Ok(chunk) => {
                        if sender.send(chunk).is_err() {
                            return;
                        }
                    }
                    // Lagged: the window fell behind, keep going with what follows.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });

        Ok((receiver, session))
    }

    /// The shells the workspace has of its own - a task's shells are the task's to show.
    pub(crate) fn terminal_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| session.owner.is_none())
            .map(|(terminal_id, _)| terminal_id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// The shells with something running in them right now - see
    /// [`TerminalSession::is_running_a_command`]. This is what quitting would interrupt, as
    /// opposed to the shells that are merely open.
    pub(crate) fn terminals_running_a_command(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| session.is_running_a_command())
            .map(|(terminal_id, _)| terminal_id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Whether this shell is still one the server has, which is what tells a task's recorded
    /// agent run from one that ended - or that died with a previous run of the server.
    pub(crate) fn is_live(&self, terminal_id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(terminal_id)
    }

    /// How long since this shell last printed anything. Nothing for a shell the server does
    /// not have, or one that has not printed yet - still starting up, which is not quiet.
    ///
    /// An agent draws a spinner for as long as it works and stops once it is waiting - on a
    /// question it asked, or on the person - so a run that has been quiet for a while is one
    /// to look at, and this is what the board reads it off.
    pub(crate) fn quiet_for(&self, terminal_id: &str) -> Option<std::time::Duration> {
        let session = self.get(terminal_id)?;
        let last_output = *session.last_output.lock().unwrap();
        last_output.map(|last| last.elapsed())
    }

    /// What this shell is asking a person for, if anything - see [`crate::attention`].
    pub(crate) fn attention(&self, terminal_id: &str) -> Option<TerminalAttentionView> {
        self.get(terminal_id)?.attention_view(terminal_id)
    }

    /// Every shell the server has that is asking for a person, whoever owns it.
    pub(crate) fn wanting_attention(&self) -> Vec<TerminalAttentionView> {
        let mut asking: Vec<TerminalAttentionView> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(terminal_id, session)| session.attention_view(terminal_id))
            .collect();
        asking.sort_by(|a, b| {
            a.at_unix
                .cmp(&b.at_unix)
                .then(a.terminal_id.cmp(&b.terminal_id))
        });
        asking
    }

    /// Every visualization the Codex runs in these terminals have announced - see
    /// [`crate::visualizations::rollout`]. Reads their rollouts, so it is file work.
    pub(crate) fn visualizations(&self) -> Vec<crate::visualizations::VisualizationView> {
        let codex_runs: Vec<(String, Arc<TerminalSession>)> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| session.visualizations.is_some())
            .map(|(terminal_id, session)| (terminal_id.clone(), Arc::clone(session)))
            .collect();
        let mut views = Vec::new();
        for (terminal_id, session) in codex_runs {
            let (Some(rollouts), Some(pid)) = (&session.visualizations, session.child_pid) else {
                continue;
            };
            let mut rollouts = rollouts.lock().unwrap();
            // A Codex that has ended holds nothing open, and has said all it will.
            let announced = if session.child_ended.load(Ordering::Relaxed) {
                rollouts.announced()
            } else {
                rollouts.poll(pid)
            };
            for (fragment_path, announced) in announced {
                // A fragment removed since it was announced has nothing left to show.
                let Ok(modified) =
                    std::fs::metadata(fragment_path).and_then(|meta| meta.modified())
                else {
                    continue;
                };
                views.push(crate::visualizations::VisualizationView {
                    terminal_id: terminal_id.clone(),
                    fragment_path: fragment_path.display().to_string(),
                    announced: *announced,
                    modified_unix_ms: modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("a file is written after the epoch")
                        .as_millis() as u64,
                });
            }
        }
        views
    }

    /// The plain shells one task has open right now, oldest first.
    ///
    /// This is the whole record of them: a shell has nothing to come back to once it ends, so
    /// the board lists the ones the server has rather than remembering the ones it had.
    pub(crate) fn owned_shells(&self, owner: &str) -> Vec<OwnedShell> {
        let mut shells: Vec<OwnedShell> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| {
                session.owner.as_deref() == Some(owner)
                    && session.program == TerminalProgram::LoginShell
            })
            .map(|(terminal_id, session)| OwnedShell {
                terminal_id: terminal_id.clone(),
                name: session.name.lock().unwrap().clone(),
                started_at_unix: session.started_at_unix,
                order: session.order,
            })
            .collect();
        shells.sort_by_key(|shell| shell.order);
        shells
    }

    /// End every shell belonging to one task, which is what finishing the task does.
    pub(crate) fn remove_owned_by(&self, owner: &str) {
        let owned: Vec<String> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| session.owner.as_deref() == Some(owner))
            .map(|(terminal_id, _)| terminal_id.clone())
            .collect();
        for terminal_id in owned {
            self.remove(&terminal_id);
        }
    }

    pub(crate) fn remove(&self, terminal_id: &str) {
        let removed = self.sessions.lock().unwrap().remove(terminal_id);
        let Some(session) = removed else {
            return;
        };
        end(&session);
    }
}

/// The shells go with the registry that started them. A server's registry lives as long as
/// the process does, but a window a test opens has one of its own, and shells left running
/// after it hold their ptys open until the test process runs out of files.
impl Drop for TerminalRegistry {
    fn drop(&mut self) {
        for session in self.sessions.get_mut().unwrap().values() {
            end(session);
        }
    }
}

fn end(session: &TerminalSession) {
    let mut child = session.child.lock().unwrap();
    let _ = child.kill();
    let _ = child.wait();
}
