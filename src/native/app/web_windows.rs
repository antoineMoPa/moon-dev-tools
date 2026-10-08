//! Browser windows - personal server persistence, without shared local browser layouts.

use egui_frames::Layout;
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use super::App;
use crate::native::panes::Pane;

#[derive(Serialize, Deserialize)]
struct SavedWindow {
    root_session: String,
    layout: Layout<Pane>,
    #[serde(default)]
    spaces: Vec<Option<SavedSpace>>,
    #[serde(default)]
    front: usize,
}

#[derive(Serialize, Deserialize)]
struct SavedSpace {
    root_session: String,
    layout: Layout<Pane>,
    repo: Option<String>,
    frame: String,
}

#[cfg(target_arch = "wasm32")]
impl App {
    pub(crate) fn save_web_window(&self) {
        // Saving the prompt/loading state would replace a useful saved arrangement with
        // an empty one before the server has opened its repository.
        if !matches!(self.model.stage, crate::native::model::Stage::Ready) {
            return;
        }
        let saved = SavedWindow {
            root_session: self.model.root_session_id.clone(),
            layout: self.model.layout.clone(),
            spaces: self
                .spaces
                .slots
                .iter()
                .map(|slot| match slot {
                    super::spaces::Slot::Parked(app) => Some(SavedSpace {
                        root_session: app.model.root_session_id.clone(),
                        layout: app.model.layout.clone(),
                        repo: app.model.opened_project.clone(),
                        frame: app.frame.subcommand().into(),
                    }),
                    _ => None,
                })
                .collect(),
            front: self.spaces.front,
        };
        if let Ok(encoded) = serde_json::to_string(&saved) {
            if let Some(account) = &self.web_account {
                account.save_layout(
                    encoded,
                    self.model.opened_project.clone(),
                    self.frame.subcommand().into(),
                );
            }
        }
    }

    pub(crate) fn restore_web_window(&mut self, ctx: &egui::Context) {
        let Some(encoded) = self
            .web_account
            .as_ref()
            .and_then(|account| account.layout())
        else {
            return;
        };
        match serde_json::from_str::<SavedWindow>(&encoded) {
            Ok(mut saved) => {
                if saved.front < super::spaces::SPACE_COUNT {
                    self.spaces.front = saved.front;
                    self.spaces.slots = (0..super::spaces::SPACE_COUNT)
                        .map(|_| super::spaces::Slot::Untouched)
                        .collect();
                    self.spaces.slots[saved.front] = super::spaces::Slot::Front;
                    for (index, space) in std::mem::take(&mut saved.spaces).into_iter().enumerate()
                    {
                        if index == saved.front || index >= super::spaces::SPACE_COUNT {
                            continue;
                        }
                        if let Some(space) = space {
                            let frame = match space.frame.as_str() {
                                "review" => crate::cli::Frame::Review,
                                "shell" => crate::cli::Frame::Shell,
                                _ => crate::cli::Frame::Tasks,
                            };
                            let mut app = App::new(
                                ctx.clone(),
                                crate::native::Launch {
                                    backend: std::sync::Arc::clone(self.backend()),
                                    open: space.repo.map(|repo_path| {
                                        crate::api::OpenSessionRequest {
                                            repo_path,
                                            diff_target: None,
                                            active_commit: None,
                                        }
                                    }),
                                    frame,
                                },
                            );
                            app.model.adopts_shells_on_open = false;
                            app.model.restored_layout =
                                Some(root_layout(space.layout, &space.root_session));
                            self.spaces.slots[index] = super::spaces::Slot::Parked(Box::new(app));
                        }
                    }
                }
                self.model.restored_layout = Some(root_layout(saved.layout, &saved.root_session));
            }
            Err(error) => eprintln!("[moon] ignoring saved personal window: {error}"),
        }
    }
}

/// Session IDs are transient. Only panes whose repository is verifiable as this window's
/// root can be rebound. Other review/file/commit panes are removed rather than pointing an
/// editor at a similarly named file in the wrong repository. Task and terminal IDs are
/// stable server resources and survive; expired terminals show the existing attach error.
fn root_layout(mut layout: Layout<Pane>, root_session: &str) -> Layout<Pane> {
    let ids: Vec<_> = layout.panes().map(|(id, _)| id).collect();
    for id in ids {
        let remove = match layout.pane_mut(id) {
            Some(
                Pane::Review { session_id, .. }
                | Pane::File { session_id, .. }
                | Pane::Commit { session_id },
            ) => {
                if session_id == root_session {
                    session_id.clear();
                    false
                } else {
                    true
                }
            }
            // Unsaved draft contents cannot be reconstructed from a layout.
            Some(Pane::NewTask { .. }) => true,
            _ => false,
        };
        if remove {
            layout.close_pane(id);
        }
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_window_round_trip_keeps_each_space_on_its_own_repository() {
        let mut layout = Layout::new();
        let frame = layout.primary_frame();
        layout.add_pane(frame, Pane::Tasks, None);
        let saved = SavedWindow {
            root_session: "front-session".into(),
            layout: layout.clone(),
            front: 2,
            spaces: vec![
                Some(SavedSpace {
                    root_session: "other-session".into(),
                    layout,
                    repo: Some("/projects/other".into()),
                    frame: "review".into(),
                }),
                None,
                None,
                None,
            ],
        };
        let encoded = serde_json::to_string(&saved).unwrap();
        let restored: SavedWindow = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.front, 2);
        assert_eq!(restored.root_session, "front-session");
        assert_eq!(restored.spaces.len(), 4);
        let parked = restored.spaces[0].as_ref().unwrap();
        assert_eq!(parked.root_session, "other-session");
        assert_eq!(parked.repo.as_deref(), Some("/projects/other"));
        assert_eq!(parked.frame, "review");
        assert_eq!(parked.layout.pane_count(), 1);
        assert!(restored.spaces[2].is_none());
    }

    #[test]
    fn other_repository_file_is_never_rebound_to_root() {
        let mut layout = Layout::new();
        let frame = layout.primary_frame();
        layout.add_pane(
            frame,
            Pane::File {
                session_id: "root".into(),
                file_path: "same.rs".into(),
                task_id: None,
                revision: None,
            },
            None,
        );
        layout.add_pane(
            frame,
            Pane::File {
                session_id: "submodule".into(),
                file_path: "same.rs".into(),
                task_id: None,
                revision: None,
            },
            None,
        );
        layout.add_pane(frame, Pane::Tasks, None);
        let restored = root_layout(layout, "root");
        assert_eq!(restored.pane_count(), 2);
        assert!(restored.panes().any(|(_, pane)| matches!(pane,
            Pane::File {session_id, ..} if session_id.is_empty())));
    }
}
