/// The workspace is a tree of splits whose leaves are frames. A frame holds tabs (panes),
/// one of which is visible. Every function here is pure: it takes a layout and returns the
/// next one, so the React state is a plain value that can be stored and restored.

export type PaneKind = "review" | "agents" | "terminal";

export type Pane =
  /// One review of one repo. The page opens on the repo it was launched in; changed
  /// submodules are further reviews, each with its own session and its own tab title.
  | { paneId: string; kind: "review"; sessionId: string; title: string }
  | { paneId: string; kind: "agents" }
  | { paneId: string; kind: "terminal"; terminalId: string };

/// A pane the user asked for, before it has an id.
export type OpenPaneRequest =
  | { kind: "review"; sessionId: string; title: string }
  | { kind: "agents" }
  | { kind: "terminal" };

export type SplitDirection = "row" | "column";

/// Where a dragged tab lands when dropped on a frame: onto its tab strip, or against one
/// of its edges, which splits that frame in two.
export type DropSide = "tabs" | "left" | "right" | "top" | "bottom";

export type LayoutNode =
  | { kind: "frame"; frameId: string }
  | { kind: "split"; direction: SplitDirection; children: LayoutNode[]; sizes: number[] };

export type Frame = {
  frameId: string;
  paneIds: string[];
  activePaneId: string | null;
};

export type WorkspaceLayout = {
  root: LayoutNode;
  frames: Record<string, Frame>;
  panes: Record<string, Pane>;
  activeFrameId: string;
};

export const mapDropSideToDirection = {
  left: "row",
  right: "row",
  top: "column",
  bottom: "column",
} satisfies Record<Exclude<DropSide, "tabs">, SplitDirection>;

const mapDropSideToOffset = {
  left: 0,
  top: 0,
  right: 1,
  bottom: 1,
} satisfies Record<Exclude<DropSide, "tabs">, number>;

const MIN_SPLIT_FRACTION = 0.1;

export function makeId(prefix: string) {
  return `${prefix}-${Math.random().toString(36).slice(2, 10)}`;
}

export function emptyLayout(): WorkspaceLayout {
  const frameId = makeId("frame");
  return {
    root: { kind: "frame", frameId },
    frames: { [frameId]: { frameId, paneIds: [], activePaneId: null } },
    panes: {},
    activeFrameId: frameId,
  };
}

export function defaultLayout(sessionId: string): WorkspaceLayout {
  const layout = emptyLayout();
  return addPane(layout, layout.activeFrameId, {
    paneId: makeId("pane"),
    kind: "review",
    sessionId,
    title: "review",
  });
}

/// The top-left frame: its tab strip doubles as the app header.
export function primaryFrameId(node: LayoutNode): string {
  return frameIdsInLayout(node)[0];
}

/// What each kind of pane needs to carry. A layout stored by an older version can be
/// missing these, and a pane without them would render as a nameless tab.
const mapPaneKindToShapeCheck: {
  [Kind in PaneKind]: (pane: Extract<Pane, { kind: Kind }>) => boolean;
} = {
  review: (pane) => typeof pane.sessionId === "string" && typeof pane.title === "string",
  agents: () => true,
  terminal: (pane) => typeof pane.terminalId === "string",
};

function isCoherentPane(pane: Pane): boolean {
  const check = mapPaneKindToShapeCheck[pane?.kind] as ((value: Pane) => boolean) | undefined;
  return Boolean(pane) && typeof pane.paneId === "string" && Boolean(check) && check!(pane);
}

/// A restored layout is only usable if its tree, its frames and its panes still agree; a
/// half-written or outdated one from storage is thrown away rather than rendered.
export function isCoherentLayout(layout: WorkspaceLayout): boolean {
  const frameIds = frameIdsInLayout(layout.root);
  return (
    frameIds.length > 0 &&
    frameIds.length === Object.keys(layout.frames).length &&
    frameIds.every((frameId) => Boolean(layout.frames[frameId])) &&
    Boolean(layout.frames[layout.activeFrameId]) &&
    Object.values(layout.panes).every(isCoherentPane) &&
    Object.values(layout.frames).every((frame) =>
      frame.paneIds.every((paneId) => Boolean(layout.panes[paneId])),
    )
  );
}

/// moonreview is a review tool: opening it shows the review of the repo it was launched
/// in, whatever the last session left behind. Closing that tab still works, it just does
/// not outlive the page.
export function withReviewPane(layout: WorkspaceLayout, sessionId: string): WorkspaceLayout {
  if (findReviewPane(layout, sessionId)) {
    return layout;
  }

  return addPane(layout, primaryFrameId(layout.root), {
    paneId: makeId("pane"),
    kind: "review",
    sessionId,
    title: "review",
  });
}

/// The pane reviewing one particular session, if it is open.
export function findReviewPane(layout: WorkspaceLayout, sessionId: string): Pane | null {
  return (
    Object.values(layout.panes).find(
      (pane) => pane.kind === "review" && pane.sessionId === sessionId,
    ) ?? null
  );
}

export function frameIdsInLayout(node: LayoutNode): string[] {
  if (node.kind === "frame") {
    return [node.frameId];
  }
  return node.children.flatMap(frameIdsInLayout);
}

export function frameHoldingPane(layout: WorkspaceLayout, paneId: string): Frame | null {
  for (const frame of Object.values(layout.frames)) {
    if (frame.paneIds.includes(paneId)) {
      return frame;
    }
  }
  return null;
}

export function findPaneOfKind(layout: WorkspaceLayout, kind: PaneKind): Pane | null {
  return Object.values(layout.panes).find((pane) => pane.kind === kind) ?? null;
}

export function addPane(
  layout: WorkspaceLayout,
  frameId: string,
  pane: Pane,
  beforePaneId?: string | null,
): WorkspaceLayout {
  const frame = layout.frames[frameId];
  const insertAt = beforePaneId ? frame.paneIds.indexOf(beforePaneId) : -1;
  const paneIds = [...frame.paneIds];
  paneIds.splice(insertAt === -1 ? paneIds.length : insertAt, 0, pane.paneId);

  return {
    ...layout,
    frames: { ...layout.frames, [frameId]: { ...frame, paneIds, activePaneId: pane.paneId } },
    panes: { ...layout.panes, [pane.paneId]: pane },
    activeFrameId: frameId,
  };
}

/// Put a new pane in a frame of its own, beside an existing frame.
export function addPaneInNewFrame(
  layout: WorkspaceLayout,
  targetFrameId: string,
  side: Exclude<DropSide, "tabs">,
  pane: Pane,
): WorkspaceLayout {
  const newFrameId = makeId("frame");
  const withFrame: WorkspaceLayout = {
    ...layout,
    root: insertFrameBeside(layout.root, targetFrameId, newFrameId, side),
    frames: {
      ...layout.frames,
      [newFrameId]: { frameId: newFrameId, paneIds: [], activePaneId: null },
    },
  };

  return addPane(withFrame, newFrameId, pane);
}

/// How much of the width a new right-hand column takes.
const RIGHT_COLUMN_FRACTION = 0.35;

/// Put a pane in a new full-height frame down the right of the workspace, which is where
/// shells live unless the user moves them.
export function addPaneInRightColumn(layout: WorkspaceLayout, pane: Pane): WorkspaceLayout {
  const newFrameId = makeId("frame");
  const newFrame: LayoutNode = { kind: "frame", frameId: newFrameId };
  const root: LayoutNode =
    layout.root.kind === "split" && layout.root.direction === "row"
      ? {
          ...layout.root,
          children: [...layout.root.children, newFrame],
          sizes: [
            ...layout.root.sizes.map((size) => size * (1 - RIGHT_COLUMN_FRACTION)),
            RIGHT_COLUMN_FRACTION,
          ],
        }
      : {
          kind: "split",
          direction: "row",
          children: [layout.root, newFrame],
          sizes: [1 - RIGHT_COLUMN_FRACTION, RIGHT_COLUMN_FRACTION],
        };

  const withFrame: WorkspaceLayout = {
    ...layout,
    root,
    frames: {
      ...layout.frames,
      [newFrameId]: { frameId: newFrameId, paneIds: [], activePaneId: null },
    },
  };

  return addPane(withFrame, newFrameId, pane);
}

/// The frame panes of a kind already live in: the one asked for if it holds one, else
/// wherever the others are. Keeps shells with shells and reviews with reviews.
export function frameHoldingKind(
  layout: WorkspaceLayout,
  kind: PaneKind,
  preferredFrameId: string,
): string | null {
  const holdsKind = (frameId: string) =>
    layout.frames[frameId]?.paneIds.some((paneId) => layout.panes[paneId].kind === kind);

  if (holdsKind(preferredFrameId)) {
    return preferredFrameId;
  }

  return frameIdsInLayout(layout.root).find(holdsKind) ?? null;
}

export function focusPane(layout: WorkspaceLayout, paneId: string): WorkspaceLayout {
  const frame = frameHoldingPane(layout, paneId);
  if (!frame) {
    return layout;
  }
  // Already in front: hand back the same layout so a click does not churn the tree.
  if (frame.activePaneId === paneId && layout.activeFrameId === frame.frameId) {
    return layout;
  }

  return {
    ...layout,
    frames: { ...layout.frames, [frame.frameId]: { ...frame, activePaneId: paneId } },
    activeFrameId: frame.frameId,
  };
}

export function replacePane(layout: WorkspaceLayout, pane: Pane): WorkspaceLayout {
  return { ...layout, panes: { ...layout.panes, [pane.paneId]: pane } };
}

export function closePane(layout: WorkspaceLayout, paneId: string): WorkspaceLayout {
  const frame = frameHoldingPane(layout, paneId);
  if (!frame) {
    return layout;
  }

  const paneIds = frame.paneIds.filter((id) => id !== paneId);
  const panes = { ...layout.panes };
  delete panes[paneId];

  const nextFrame: Frame = {
    ...frame,
    paneIds,
    activePaneId:
      frame.activePaneId === paneId ? (paneIds[paneIds.length - 1] ?? null) : frame.activePaneId,
  };
  const withoutPane: WorkspaceLayout = {
    ...layout,
    frames: { ...layout.frames, [frame.frameId]: nextFrame },
    panes,
  };

  // The last frame stays even when empty so the workspace always has a drop target.
  if (paneIds.length > 0 || frameIdsInLayout(layout.root).length === 1) {
    return withoutPane;
  }

  return removeFrame(withoutPane, frame.frameId);
}

function removeFrame(layout: WorkspaceLayout, frameId: string): WorkspaceLayout {
  const frames = { ...layout.frames };
  delete frames[frameId];
  const root = removeFrameNode(layout.root, frameId);
  const remainingFrameIds = frameIdsInLayout(root);

  return {
    ...layout,
    root,
    frames,
    activeFrameId: remainingFrameIds.includes(layout.activeFrameId)
      ? layout.activeFrameId
      : remainingFrameIds[0],
  };
}

function removeFrameNode(node: LayoutNode, frameId: string): LayoutNode {
  if (node.kind === "frame") {
    return node;
  }

  const removeIndex = node.children.findIndex(
    (child) => child.kind === "frame" && child.frameId === frameId,
  );

  if (removeIndex === -1) {
    return {
      ...node,
      children: node.children.map((child) => removeFrameNode(child, frameId)),
    };
  }

  const children = node.children.filter((_, index) => index !== removeIndex);
  const sizes = node.sizes.filter((_, index) => index !== removeIndex);
  if (children.length === 1) {
    return children[0];
  }

  return { ...node, children, sizes: rescaleToOne(sizes) };
}

/// Move a pane onto another frame's tab strip, or against one of its edges, which puts the
/// pane in a brand new frame beside it.
export function movePaneToFrame(
  layout: WorkspaceLayout,
  paneId: string,
  targetFrameId: string,
  side: DropSide,
  beforePaneId?: string | null,
): WorkspaceLayout {
  const pane = layout.panes[paneId];
  const sourceFrame = frameHoldingPane(layout, paneId);
  if (!pane || !sourceFrame) {
    return layout;
  }

  if (side === "tabs") {
    if (sourceFrame.frameId === targetFrameId) {
      return reorderPane(layout, sourceFrame, paneId, beforePaneId ?? null);
    }
    return addPane(closePane(layout, paneId), targetFrameId, pane, beforePaneId);
  }

  // A lone tab dropped on its own frame's edge would just rebuild the same frame.
  if (sourceFrame.frameId === targetFrameId && sourceFrame.paneIds.length === 1) {
    return layout;
  }

  return addPaneInNewFrame(closePane(layout, paneId), targetFrameId, side, pane);
}

function reorderPane(
  layout: WorkspaceLayout,
  frame: Frame,
  paneId: string,
  beforePaneId: string | null,
): WorkspaceLayout {
  const paneIds = frame.paneIds.filter((id) => id !== paneId);
  const insertAt = beforePaneId ? paneIds.indexOf(beforePaneId) : -1;
  paneIds.splice(insertAt === -1 ? paneIds.length : insertAt, 0, paneId);

  return {
    ...layout,
    frames: { ...layout.frames, [frame.frameId]: { ...frame, paneIds, activePaneId: paneId } },
    activeFrameId: frame.frameId,
  };
}

function insertFrameBeside(
  node: LayoutNode,
  targetFrameId: string,
  newFrameId: string,
  side: Exclude<DropSide, "tabs">,
): LayoutNode {
  const direction = mapDropSideToDirection[side];
  const offset = mapDropSideToOffset[side];
  const newFrame: LayoutNode = { kind: "frame", frameId: newFrameId };

  if (node.kind === "frame") {
    if (node.frameId !== targetFrameId) {
      return node;
    }
    const children = offset === 0 ? [newFrame, node] : [node, newFrame];
    return { kind: "split", direction, children, sizes: [0.5, 0.5] };
  }

  const targetIndex = node.children.findIndex(
    (child) => child.kind === "frame" && child.frameId === targetFrameId,
  );

  if (targetIndex === -1 || node.direction !== direction) {
    return {
      ...node,
      children: node.children.map((child) =>
        insertFrameBeside(child, targetFrameId, newFrameId, side),
      ),
    };
  }

  // Same direction as the surrounding split: the new frame becomes another sibling and
  // takes half of the space the target had.
  const children = [...node.children];
  const sizes = [...node.sizes];
  const half = sizes[targetIndex] / 2;
  sizes[targetIndex] = half;
  children.splice(targetIndex + offset, 0, newFrame);
  sizes.splice(targetIndex + offset, 0, half);

  return { ...node, children, sizes };
}

export function setSplitSizes(
  node: LayoutNode,
  path: number[],
  sizes: number[],
): LayoutNode {
  if (node.kind === "frame") {
    return node;
  }

  if (path.length === 0) {
    return { ...node, sizes: rescaleToOne(sizes.map((size) => Math.max(size, MIN_SPLIT_FRACTION))) };
  }

  const [index, ...rest] = path;
  return {
    ...node,
    children: node.children.map((child, childIndex) =>
      childIndex === index ? setSplitSizes(child, rest, sizes) : child,
    ),
  };
}

function rescaleToOne(sizes: number[]): number[] {
  const total = sizes.reduce((sum, size) => sum + size, 0);
  return sizes.map((size) => size / total);
}
