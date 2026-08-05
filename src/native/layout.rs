//! The workspace is a tree of splits whose leaves are frames. A frame holds tabs (panes),
//! one of which is visible. Every function here is pure: it takes a layout and returns the
//! next one, which is what makes the arrangement something the app can store and restore.
//!
//! This mirrors `web/src/components/workspace/layout.ts` so both frontends arrange panes
//! the same way.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::api::AgentKind;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn make_id(prefix: &str) -> String {
    format!("{prefix}-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PaneKind {
    Review,
    Agents,
    Terminal,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum Pane {
    /// One review of one repo. The window opens on the repo it was launched in; changed
    /// submodules are further reviews, each with its own session and its own tab title.
    Review {
        pane_id: String,
        session_id: String,
        title: String,
    },
    Agents {
        pane_id: String,
    },
    Terminal {
        pane_id: String,
        terminal_id: String,
        #[serde(default)]
        command: Option<AgentKind>,
    },
}

impl Pane {
    pub(crate) fn pane_id(&self) -> &str {
        match self {
            Self::Review { pane_id, .. }
            | Self::Agents { pane_id }
            | Self::Terminal { pane_id, .. } => pane_id,
        }
    }

    pub(crate) fn kind(&self) -> PaneKind {
        match self {
            Self::Review { .. } => PaneKind::Review,
            Self::Agents { .. } => PaneKind::Agents,
            Self::Terminal { .. } => PaneKind::Terminal,
        }
    }

    pub(crate) fn tab_title(&self) -> String {
        match self {
            Self::Review { title, .. } => title.clone(),
            Self::Agents { .. } => "comment agents".to_string(),
            Self::Terminal { command, .. } => match command {
                Some(AgentKind::Claude) => "claude".to_string(),
                Some(AgentKind::Codex) => "codex".to_string(),
                Some(AgentKind::OpenCode) => "opencode".to_string(),
                _ => "terminal".to_string(),
            },
        }
    }
}

/// A pane the user asked for, before it has an id.
#[derive(Clone)]
pub(crate) enum OpenPaneRequest {
    Review {
        session_id: String,
        title: String,
    },
    /// A review of a repo that has no session yet, which is how a changed submodule is
    /// opened: the session gets created on the way to the pane.
    ReviewRepo {
        repo_path: String,
        title: String,
    },
    Agents,
    Terminal {
        command: Option<AgentKind>,
    },
}


#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SplitDirection {
    Row,
    Column,
}

/// Where a dragged tab lands when dropped on a frame: onto its tab strip, or against one of
/// its edges, which splits that frame in two.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DropSide {
    Tabs,
    Left,
    Right,
    Top,
    Bottom,
}

impl DropSide {
    fn direction(self) -> Option<SplitDirection> {
        match self {
            Self::Tabs => None,
            Self::Left | Self::Right => Some(SplitDirection::Row),
            Self::Top | Self::Bottom => Some(SplitDirection::Column),
        }
    }

    /// Whether the new frame goes before (0) or after (1) the frame it was dropped on.
    fn offset(self) -> usize {
        match self {
            Self::Left | Self::Top => 0,
            _ => 1,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum LayoutNode {
    Frame {
        frame_id: String,
    },
    Split {
        direction: SplitDirection,
        children: Vec<LayoutNode>,
        sizes: Vec<f32>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Frame {
    pub(crate) frame_id: String,
    pub(crate) pane_ids: Vec<String>,
    pub(crate) active_pane_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct WorkspaceLayout {
    pub(crate) root: LayoutNode,
    pub(crate) frames: HashMap<String, Frame>,
    pub(crate) panes: HashMap<String, Pane>,
    pub(crate) active_frame_id: String,
}

const MIN_SPLIT_FRACTION: f32 = 0.1;
/// How much of the width a new right-hand column takes.
pub(crate) const RIGHT_COLUMN_FRACTION: f32 = 0.35;

pub(crate) fn empty_layout() -> WorkspaceLayout {
    let frame_id = make_id("frame");
    WorkspaceLayout {
        root: LayoutNode::Frame {
            frame_id: frame_id.clone(),
        },
        frames: HashMap::from([(
            frame_id.clone(),
            Frame {
                frame_id: frame_id.clone(),
                pane_ids: Vec::new(),
                active_pane_id: None,
            },
        )]),
        panes: HashMap::new(),
        active_frame_id: frame_id,
    }
}

pub(crate) fn default_layout(session_id: &str) -> WorkspaceLayout {
    let layout = empty_layout();
    let frame_id = layout.active_frame_id.clone();
    add_pane(
        layout,
        &frame_id,
        Pane::Review {
            pane_id: make_id("pane"),
            session_id: session_id.to_string(),
            title: "review".to_string(),
        },
        None,
    )
}

impl WorkspaceLayout {
    /// The top-left frame: its tab strip doubles as the app header.
    pub(crate) fn primary_frame_id(&self) -> String {
        frame_ids_in(&self.root)
            .first()
            .cloned()
            .unwrap_or_else(|| self.active_frame_id.clone())
    }

    pub(crate) fn frame_ids(&self) -> Vec<String> {
        frame_ids_in(&self.root)
    }

    pub(crate) fn frame_holding_pane(&self, pane_id: &str) -> Option<&Frame> {
        self.frames
            .values()
            .find(|frame| frame.pane_ids.iter().any(|id| id == pane_id))
    }

    /// The pane reviewing one particular session, if it is open.
    pub(crate) fn find_review_pane(&self, session_id: &str) -> Option<&Pane> {
        self.panes.values().find(|pane| {
            matches!(pane, Pane::Review { session_id: pane_session, .. } if pane_session == session_id)
        })
    }

    pub(crate) fn find_pane_of_kind(&self, kind: PaneKind) -> Option<&Pane> {
        self.panes.values().find(|pane| pane.kind() == kind)
    }

    /// The frame panes of a kind already live in: the one asked for if it holds one, else
    /// wherever the others are. Keeps shells with shells and reviews with reviews.
    pub(crate) fn frame_holding_kind(
        &self,
        kind: PaneKind,
        preferred_frame_id: &str,
    ) -> Option<String> {
        let holds_kind = |frame_id: &str| {
            self.frames.get(frame_id).is_some_and(|frame| {
                frame
                    .pane_ids
                    .iter()
                    .any(|pane_id| self.panes.get(pane_id).is_some_and(|pane| pane.kind() == kind))
            })
        };

        if holds_kind(preferred_frame_id) {
            return Some(preferred_frame_id.to_string());
        }
        self.frame_ids().into_iter().find(|id| holds_kind(id))
    }

    /// A restored layout is only usable if its tree, its frames and its panes still agree;
    /// a half-written or outdated one from storage is thrown away rather than rendered.
    pub(crate) fn is_coherent(&self) -> bool {
        let frame_ids = self.frame_ids();
        !frame_ids.is_empty()
            && frame_ids.len() == self.frames.len()
            && frame_ids.iter().all(|id| self.frames.contains_key(id))
            && self.frames.contains_key(&self.active_frame_id)
            && self.frames.values().all(|frame| {
                frame
                    .pane_ids
                    .iter()
                    .all(|pane_id| self.panes.contains_key(pane_id))
            })
    }
}

pub(crate) fn frame_ids_in(node: &LayoutNode) -> Vec<String> {
    match node {
        LayoutNode::Frame { frame_id } => vec![frame_id.clone()],
        LayoutNode::Split { children, .. } => children.iter().flat_map(frame_ids_in).collect(),
    }
}

/// Keep a restored arrangement's splits, but none of its panes.
///
/// A stored layout is worth keeping for its shape: where the user put their columns and rows.
/// Its panes are not — a review pane names a session that no longer exists, and a terminal
/// pane names a shell that died with the last process. So the frames come back empty and get
/// refilled.
pub(crate) fn shape_only(mut layout: WorkspaceLayout) -> WorkspaceLayout {
    layout.panes.clear();
    for frame in layout.frames.values_mut() {
        frame.pane_ids.clear();
        frame.active_pane_id = None;
    }
    layout
}

/// moonreview is a review tool: opening it shows the review of the repo it was launched in,
/// whatever the last session left behind.
pub(crate) fn with_review_pane(layout: WorkspaceLayout, session_id: &str) -> WorkspaceLayout {
    if layout.find_review_pane(session_id).is_some() {
        return layout;
    }

    let frame_id = layout.primary_frame_id();
    add_pane(
        layout,
        &frame_id,
        Pane::Review {
            pane_id: make_id("pane"),
            session_id: session_id.to_string(),
            title: "review".to_string(),
        },
        None,
    )
}

pub(crate) fn add_pane(
    mut layout: WorkspaceLayout,
    frame_id: &str,
    pane: Pane,
    before_pane_id: Option<&str>,
) -> WorkspaceLayout {
    let pane_id = pane.pane_id().to_string();
    let Some(frame) = layout.frames.get_mut(frame_id) else {
        return layout;
    };

    let insert_at = before_pane_id
        .and_then(|before| frame.pane_ids.iter().position(|id| id == before))
        .unwrap_or(frame.pane_ids.len());
    frame.pane_ids.insert(insert_at, pane_id.clone());
    frame.active_pane_id = Some(pane_id.clone());

    layout.panes.insert(pane_id, pane);
    layout.active_frame_id = frame_id.to_string();
    layout
}

/// Put a new pane in a frame of its own, beside an existing frame.
pub(crate) fn add_pane_in_new_frame(
    mut layout: WorkspaceLayout,
    target_frame_id: &str,
    side: DropSide,
    pane: Pane,
) -> WorkspaceLayout {
    let Some(direction) = side.direction() else {
        return add_pane(layout, target_frame_id, pane, None);
    };

    let new_frame_id = make_id("frame");
    layout.root = insert_frame_beside(
        layout.root,
        target_frame_id,
        &new_frame_id,
        direction,
        side.offset(),
    );
    layout.frames.insert(
        new_frame_id.clone(),
        Frame {
            frame_id: new_frame_id.clone(),
            pane_ids: Vec::new(),
            active_pane_id: None,
        },
    );

    add_pane(layout, &new_frame_id, pane, None)
}

/// Put a pane in a new full-height frame down the right of the workspace, which is where
/// shells live unless the user moves them.
pub(crate) fn add_pane_in_right_column(
    mut layout: WorkspaceLayout,
    pane: Pane,
) -> WorkspaceLayout {
    let new_frame_id = make_id("frame");
    let new_frame = LayoutNode::Frame {
        frame_id: new_frame_id.clone(),
    };

    layout.root = match layout.root {
        LayoutNode::Split {
            direction: SplitDirection::Row,
            mut children,
            sizes,
        } => {
            children.push(new_frame);
            let mut sizes: Vec<f32> = sizes
                .into_iter()
                .map(|size| size * (1.0 - RIGHT_COLUMN_FRACTION))
                .collect();
            sizes.push(RIGHT_COLUMN_FRACTION);
            LayoutNode::Split {
                direction: SplitDirection::Row,
                children,
                sizes,
            }
        }
        root => LayoutNode::Split {
            direction: SplitDirection::Row,
            children: vec![root, new_frame],
            sizes: vec![1.0 - RIGHT_COLUMN_FRACTION, RIGHT_COLUMN_FRACTION],
        },
    };

    layout.frames.insert(
        new_frame_id.clone(),
        Frame {
            frame_id: new_frame_id.clone(),
            pane_ids: Vec::new(),
            active_pane_id: None,
        },
    );

    add_pane(layout, &new_frame_id, pane, None)
}

pub(crate) fn focus_pane(mut layout: WorkspaceLayout, pane_id: &str) -> WorkspaceLayout {
    let Some(frame_id) = layout
        .frame_holding_pane(pane_id)
        .map(|frame| frame.frame_id.clone())
    else {
        return layout;
    };

    if let Some(frame) = layout.frames.get_mut(&frame_id) {
        frame.active_pane_id = Some(pane_id.to_string());
    }
    layout.active_frame_id = frame_id;
    layout
}

pub(crate) fn close_pane(mut layout: WorkspaceLayout, pane_id: &str) -> WorkspaceLayout {
    let Some(frame_id) = layout
        .frame_holding_pane(pane_id)
        .map(|frame| frame.frame_id.clone())
    else {
        return layout;
    };

    let only_frame_left = layout.frame_ids().len() == 1;
    let frame_is_empty = {
        let Some(frame) = layout.frames.get_mut(&frame_id) else {
            return layout;
        };
        frame.pane_ids.retain(|id| id != pane_id);
        if frame.active_pane_id.as_deref() == Some(pane_id) {
            frame.active_pane_id = frame.pane_ids.last().cloned();
        }
        frame.pane_ids.is_empty()
    };
    layout.panes.remove(pane_id);

    // The last frame stays even when empty so the workspace always has a drop target.
    if !frame_is_empty || only_frame_left {
        return layout;
    }

    remove_frame(layout, &frame_id)
}

fn remove_frame(mut layout: WorkspaceLayout, frame_id: &str) -> WorkspaceLayout {
    layout.frames.remove(frame_id);
    layout.root = remove_frame_node(layout.root, frame_id);
    let remaining = layout.frame_ids();
    if !remaining.contains(&layout.active_frame_id) {
        layout.active_frame_id = remaining.first().cloned().unwrap_or_default();
    }
    layout
}

fn remove_frame_node(node: LayoutNode, frame_id: &str) -> LayoutNode {
    let LayoutNode::Split {
        direction,
        children,
        sizes,
    } = node
    else {
        return node;
    };

    let remove_index = children.iter().position(
        |child| matches!(child, LayoutNode::Frame { frame_id: id } if id == frame_id),
    );

    let Some(remove_index) = remove_index else {
        return LayoutNode::Split {
            direction,
            children: children
                .into_iter()
                .map(|child| remove_frame_node(child, frame_id))
                .collect(),
            sizes,
        };
    };

    let mut children = children;
    let mut sizes = sizes;
    children.remove(remove_index);
    if remove_index < sizes.len() {
        sizes.remove(remove_index);
    }
    if children.len() == 1 {
        return children.remove(0);
    }

    LayoutNode::Split {
        direction,
        children,
        sizes: rescale_to_one(sizes),
    }
}

/// Move a pane onto another frame's tab strip, or against one of its edges, which puts the
/// pane in a brand new frame beside it.
pub(crate) fn move_pane_to_frame(
    layout: WorkspaceLayout,
    pane_id: &str,
    target_frame_id: &str,
    side: DropSide,
    before_pane_id: Option<&str>,
) -> WorkspaceLayout {
    let Some(pane) = layout.panes.get(pane_id).cloned() else {
        return layout;
    };
    let Some(source_frame) = layout.frame_holding_pane(pane_id) else {
        return layout;
    };
    let source_frame_id = source_frame.frame_id.clone();
    let source_pane_count = source_frame.pane_ids.len();

    if side == DropSide::Tabs {
        if source_frame_id == target_frame_id {
            return reorder_pane(layout, &source_frame_id, pane_id, before_pane_id);
        }
        let layout = close_pane(layout, pane_id);
        return add_pane(layout, target_frame_id, pane, before_pane_id);
    }

    // A lone tab dropped on its own frame's edge would just rebuild the same frame.
    if source_frame_id == target_frame_id && source_pane_count == 1 {
        return layout;
    }

    let layout = close_pane(layout, pane_id);
    add_pane_in_new_frame(layout, target_frame_id, side, pane)
}

fn reorder_pane(
    mut layout: WorkspaceLayout,
    frame_id: &str,
    pane_id: &str,
    before_pane_id: Option<&str>,
) -> WorkspaceLayout {
    if let Some(frame) = layout.frames.get_mut(frame_id) {
        frame.pane_ids.retain(|id| id != pane_id);
        let insert_at = before_pane_id
            .and_then(|before| frame.pane_ids.iter().position(|id| id == before))
            .unwrap_or(frame.pane_ids.len());
        frame.pane_ids.insert(insert_at, pane_id.to_string());
        frame.active_pane_id = Some(pane_id.to_string());
    }
    layout.active_frame_id = frame_id.to_string();
    layout
}

fn insert_frame_beside(
    node: LayoutNode,
    target_frame_id: &str,
    new_frame_id: &str,
    direction: SplitDirection,
    offset: usize,
) -> LayoutNode {
    let new_frame = LayoutNode::Frame {
        frame_id: new_frame_id.to_string(),
    };

    match node {
        LayoutNode::Frame { frame_id } => {
            if frame_id != target_frame_id {
                return LayoutNode::Frame { frame_id };
            }
            let existing = LayoutNode::Frame { frame_id };
            let children = if offset == 0 {
                vec![new_frame, existing]
            } else {
                vec![existing, new_frame]
            };
            LayoutNode::Split {
                direction,
                children,
                sizes: vec![0.5, 0.5],
            }
        }
        LayoutNode::Split {
            direction: node_direction,
            children,
            sizes,
        } => {
            let target_index = children.iter().position(
                |child| matches!(child, LayoutNode::Frame { frame_id } if frame_id == target_frame_id),
            );

            let Some(target_index) = target_index.filter(|_| node_direction == direction) else {
                return LayoutNode::Split {
                    direction: node_direction,
                    children: children
                        .into_iter()
                        .map(|child| {
                            insert_frame_beside(
                                child,
                                target_frame_id,
                                new_frame_id,
                                direction,
                                offset,
                            )
                        })
                        .collect(),
                    sizes,
                };
            };

            // Same direction as the surrounding split: the new frame becomes another
            // sibling and takes half of the space the target had.
            let mut children = children;
            let mut sizes = sizes;
            let half = sizes.get(target_index).copied().unwrap_or(0.5) / 2.0;
            if let Some(size) = sizes.get_mut(target_index) {
                *size = half;
            }
            children.insert(target_index + offset, new_frame);
            sizes.insert(target_index + offset, half);

            LayoutNode::Split {
                direction: node_direction,
                children,
                sizes,
            }
        }
    }
}

pub(crate) fn set_split_sizes(
    node: LayoutNode,
    path: &[usize],
    new_sizes: &[f32],
) -> LayoutNode {
    let LayoutNode::Split {
        direction,
        children,
        sizes,
    } = node
    else {
        return node;
    };

    if path.is_empty() {
        return LayoutNode::Split {
            direction,
            children,
            sizes: rescale_to_one(
                new_sizes
                    .iter()
                    .map(|size| size.max(MIN_SPLIT_FRACTION))
                    .collect(),
            ),
        };
    }

    let (index, rest) = (path[0], &path[1..]);
    LayoutNode::Split {
        direction,
        children: children
            .into_iter()
            .enumerate()
            .map(|(child_index, child)| {
                if child_index == index {
                    set_split_sizes(child, rest, new_sizes)
                } else {
                    child
                }
            })
            .collect(),
        sizes,
    }
}

fn rescale_to_one(sizes: Vec<f32>) -> Vec<f32> {
    let total: f32 = sizes.iter().sum();
    if total <= 0.0 {
        let share = 1.0 / sizes.len().max(1) as f32;
        return sizes.iter().map(|_| share).collect();
    }
    sizes.into_iter().map(|size| size / total).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_pane(session_id: &str) -> Pane {
        Pane::Review {
            pane_id: make_id("pane"),
            session_id: session_id.to_string(),
            title: "review".to_string(),
        }
    }

    fn terminal_pane() -> Pane {
        Pane::Terminal {
            pane_id: make_id("pane"),
            terminal_id: make_id("terminal"),
            command: None,
        }
    }

    #[test]
    fn default_layout_opens_one_review_in_one_frame() {
        let layout = default_layout("session-1");

        assert_eq!(layout.frame_ids().len(), 1);
        assert_eq!(layout.panes.len(), 1);
        assert!(layout.find_review_pane("session-1").is_some());
        assert!(layout.is_coherent());
    }

    #[test]
    fn with_review_pane_does_not_open_the_same_review_twice() {
        let layout = default_layout("session-1");
        let panes_before = layout.panes.len();

        let layout = with_review_pane(layout, "session-1");

        assert_eq!(layout.panes.len(), panes_before);
    }

    #[test]
    fn dropping_a_pane_on_an_edge_splits_the_frame() {
        let layout = default_layout("session-1");
        let frame_id = layout.active_frame_id.clone();
        let layout = add_pane(layout, &frame_id, terminal_pane(), None);
        let terminal_id = layout
            .frames
            .get(&frame_id)
            .expect("expected the frame")
            .pane_ids
            .last()
            .cloned()
            .expect("expected a terminal pane");

        let layout = move_pane_to_frame(layout, &terminal_id, &frame_id, DropSide::Right, None);

        assert_eq!(layout.frame_ids().len(), 2);
        assert!(layout.is_coherent());
        assert!(matches!(layout.root, LayoutNode::Split { .. }));
    }

    #[test]
    fn a_lone_tab_dropped_on_its_own_edge_changes_nothing() {
        let layout = default_layout("session-1");
        let frame_id = layout.active_frame_id.clone();
        let pane_id = layout.panes.keys().next().cloned().expect("expected a pane");

        let layout = move_pane_to_frame(layout, &pane_id, &frame_id, DropSide::Bottom, None);

        assert_eq!(layout.frame_ids().len(), 1);
    }

    #[test]
    fn closing_the_last_pane_of_a_split_frame_removes_that_frame() {
        let layout = default_layout("session-1");
        let frame_id = layout.active_frame_id.clone();
        let layout = add_pane_in_new_frame(layout, &frame_id, DropSide::Right, terminal_pane());
        let terminal_pane_id = layout
            .panes
            .values()
            .find(|pane| pane.kind() == PaneKind::Terminal)
            .map(|pane| pane.pane_id().to_string())
            .expect("expected a terminal pane");
        assert_eq!(layout.frame_ids().len(), 2);

        let layout = close_pane(layout, &terminal_pane_id);

        assert_eq!(layout.frame_ids().len(), 1);
        assert!(layout.is_coherent());
    }

    #[test]
    fn closing_the_last_pane_keeps_one_empty_frame_as_a_drop_target() {
        let layout = default_layout("session-1");
        let pane_id = layout.panes.keys().next().cloned().expect("expected a pane");

        let layout = close_pane(layout, &pane_id);

        assert_eq!(layout.frame_ids().len(), 1);
        assert!(layout.panes.is_empty());
        assert!(layout.is_coherent());
    }

    #[test]
    fn a_right_column_takes_its_share_from_the_existing_columns() {
        let layout = default_layout("session-1");

        let layout = add_pane_in_right_column(layout, terminal_pane());

        let LayoutNode::Split { sizes, .. } = &layout.root else {
            panic!("expected the root to be a split");
        };
        assert_eq!(sizes.len(), 2);
        assert!((sizes.iter().sum::<f32>() - 1.0).abs() < f32::EPSILON);
        assert!((sizes[1] - RIGHT_COLUMN_FRACTION).abs() < f32::EPSILON);
    }

    #[test]
    fn a_second_right_column_rescales_the_earlier_ones() {
        let layout = default_layout("session-1");
        let layout = add_pane_in_right_column(layout, terminal_pane());

        let layout = add_pane_in_right_column(layout, terminal_pane());

        let LayoutNode::Split { sizes, children, .. } = &layout.root else {
            panic!("expected the root to be a split");
        };
        assert_eq!(children.len(), 3);
        assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn reordering_tabs_within_a_frame_keeps_every_pane() {
        let layout = default_layout("session-1");
        let frame_id = layout.active_frame_id.clone();
        let layout = add_pane(layout, &frame_id, terminal_pane(), None);
        let pane_ids = layout
            .frames
            .get(&frame_id)
            .expect("expected the frame")
            .pane_ids
            .clone();

        let layout = move_pane_to_frame(
            layout,
            &pane_ids[1],
            &frame_id,
            DropSide::Tabs,
            Some(&pane_ids[0]),
        );

        let reordered = &layout.frames.get(&frame_id).expect("expected the frame").pane_ids;
        assert_eq!(reordered, &vec![pane_ids[1].clone(), pane_ids[0].clone()]);
        assert!(layout.is_coherent());
    }

    #[test]
    fn split_sizes_never_collapse_a_pane_to_nothing() {
        let layout = default_layout("session-1");
        let frame_id = layout.active_frame_id.clone();
        let layout = add_pane_in_new_frame(layout, &frame_id, DropSide::Right, terminal_pane());

        let root = set_split_sizes(layout.root, &[], &[0.0, 1.0]);

        let LayoutNode::Split { sizes, .. } = &root else {
            panic!("expected the root to be a split");
        };
        assert!(sizes[0] >= MIN_SPLIT_FRACTION * 0.9);
        assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn frame_holding_kind_keeps_shells_together() {
        let layout = default_layout("session-1");
        let review_frame = layout.active_frame_id.clone();
        let layout = add_pane_in_new_frame(layout, &review_frame, DropSide::Right, terminal_pane());
        let shell_frame = layout.active_frame_id.clone();

        assert_eq!(
            layout.frame_holding_kind(PaneKind::Terminal, &review_frame),
            Some(shell_frame)
        );
        assert_eq!(
            layout.frame_holding_kind(PaneKind::Review, &review_frame),
            Some(review_frame)
        );
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;

    #[test]
    fn a_stored_arrangement_comes_back_as_empty_frames() {
        let layout = default_layout("session-1");
        let frame_id = layout.active_frame_id.clone();
        let layout = add_pane_in_new_frame(
            layout,
            &frame_id,
            DropSide::Right,
            Pane::Terminal {
                pane_id: make_id("pane"),
                terminal_id: "terminal-0".to_string(),
                command: None,
            },
        );
        assert_eq!(layout.frame_ids().len(), 2);

        let restored = shape_only(layout);

        assert_eq!(restored.frame_ids().len(), 2, "the splits are the point");
        assert!(restored.panes.is_empty(), "stale panes must not come back");
        assert!(
            restored
                .frames
                .values()
                .all(|frame| frame.pane_ids.is_empty() && frame.active_pane_id.is_none())
        );
        assert!(restored.is_coherent());
    }

    #[test]
    fn a_restored_arrangement_gets_the_new_review_in_its_primary_frame() {
        let layout = default_layout("old-session");
        let frame_id = layout.active_frame_id.clone();
        let layout = add_pane_in_new_frame(
            layout,
            &frame_id,
            DropSide::Bottom,
            Pane::Agents {
                pane_id: make_id("pane"),
            },
        );

        let restored = with_review_pane(shape_only(layout), "new-session");

        assert_eq!(restored.frame_ids().len(), 2);
        assert_eq!(restored.panes.len(), 1);
        assert!(restored.find_review_pane("new-session").is_some());
        assert!(restored.find_review_pane("old-session").is_none());
        let primary = restored.primary_frame_id();
        assert_eq!(
            restored
                .frames
                .get(&primary)
                .expect("expected the primary frame")
                .pane_ids
                .len(),
            1,
            "the review belongs in the frame whose tab strip is the app header"
        );
    }

    #[test]
    fn a_stored_arrangement_survives_a_round_trip_through_json() {
        let layout = default_layout("session-1");
        let frame_id = layout.active_frame_id.clone();
        let layout = add_pane_in_new_frame(
            layout,
            &frame_id,
            DropSide::Right,
            Pane::Agents {
                pane_id: make_id("pane"),
            },
        );

        let encoded = serde_json::to_string(&layout).expect("expected the layout to encode");
        let decoded: WorkspaceLayout =
            serde_json::from_str(&encoded).expect("expected the layout to decode");

        assert!(decoded.is_coherent());
        assert_eq!(decoded.frame_ids().len(), layout.frame_ids().len());
        assert_eq!(decoded.active_frame_id, layout.active_frame_id);
    }

    #[test]
    fn a_layout_whose_frames_disagree_with_its_tree_is_rejected() {
        let mut layout = default_layout("session-1");
        // What a truncated or outdated stored value looks like.
        layout.frames.clear();

        assert!(!layout.is_coherent());
    }
}
