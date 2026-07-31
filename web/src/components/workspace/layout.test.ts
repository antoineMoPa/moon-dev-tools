import { describe, expect, it } from "vitest";
import {
  addPane,
  addPaneInNewFrame,
  addPaneInRightColumn,
  closePane,
  defaultLayout,
  emptyLayout,
  findPaneOfKind,
  focusPane,
  frameHoldingPane,
  frameHoldingKind,
  frameIdsInLayout,
  isCoherentLayout,
  movePaneToFrame,
  setSplitSizes,
  withReviewPane,
} from "./layout";
import type { WorkspaceLayout } from "./layout";

function layoutWithReviewAndTerminal() {
  const layout = defaultLayout("session-root");
  const review = findPaneOfKind(layout, "review")!;
  const withTerminal = addPane(layout, layout.activeFrameId, {
    paneId: "pane-terminal",
    kind: "terminal",
    terminalId: "terminal-0",
  });
  return { layout: withTerminal, reviewPaneId: review.paneId };
}

describe("workspace layout", () => {
  it("starts as one frame holding the review", () => {
    const layout = defaultLayout("session-root");
    expect(frameIdsInLayout(layout.root)).toHaveLength(1);
    expect(findPaneOfKind(layout, "review")).not.toBeNull();
  });

  it("adds a tab to a frame and focuses it", () => {
    const { layout } = layoutWithReviewAndTerminal();
    const frame = layout.frames[layout.activeFrameId];
    expect(frame.paneIds).toHaveLength(2);
    expect(frame.activePaneId).toBe("pane-terminal");
  });

  it("splits a frame when a tab is dropped on its edge", () => {
    const { layout, reviewPaneId } = layoutWithReviewAndTerminal();
    const split = movePaneToFrame(layout, "pane-terminal", layout.activeFrameId, "right");

    expect(split.root.kind).toBe("split");
    expect(frameIdsInLayout(split.root)).toHaveLength(2);
    expect(frameHoldingPane(split, "pane-terminal")).not.toBe(
      frameHoldingPane(split, reviewPaneId),
    );
  });

  it("reuses the surrounding split when the new frame goes the same way", () => {
    const { layout } = layoutWithReviewAndTerminal();
    const twoFrames = movePaneToFrame(layout, "pane-terminal", layout.activeFrameId, "right");
    const withThird = addPane(twoFrames, twoFrames.activeFrameId, {
      paneId: "pane-agents",
      kind: "agents",
    });
    const threeFrames = movePaneToFrame(
      withThird,
      "pane-agents",
      withThird.activeFrameId,
      "right",
    );

    expect(threeFrames.root.kind).toBe("split");
    if (threeFrames.root.kind === "split") {
      expect(threeFrames.root.children).toHaveLength(3);
      expect(threeFrames.root.sizes.reduce((sum, size) => sum + size, 0)).toBeCloseTo(1);
    }
  });

  it("drops the frame once its last tab leaves", () => {
    const { layout, reviewPaneId } = layoutWithReviewAndTerminal();
    const split = movePaneToFrame(layout, "pane-terminal", layout.activeFrameId, "right");
    const closed = closePane(split, "pane-terminal");

    expect(frameIdsInLayout(closed.root)).toHaveLength(1);
    expect(closed.root.kind).toBe("frame");
    expect(frameHoldingPane(closed, reviewPaneId)).not.toBeNull();
    expect(closed.frames[closed.activeFrameId]).toBeDefined();
  });

  it("keeps the last frame even with no tabs left", () => {
    const layout = defaultLayout("session-root");
    const reviewPaneId = findPaneOfKind(layout, "review")!.paneId;
    const closed = closePane(layout, reviewPaneId);

    expect(frameIdsInLayout(closed.root)).toHaveLength(1);
    expect(closed.frames[closed.activeFrameId].paneIds).toHaveLength(0);
    expect(Object.keys(closed.panes)).toHaveLength(0);
  });

  it("moves a tab between frames without losing it", () => {
    const { layout, reviewPaneId } = layoutWithReviewAndTerminal();
    const split = movePaneToFrame(layout, "pane-terminal", layout.activeFrameId, "bottom");
    const terminalFrameId = frameHoldingPane(split, "pane-terminal")!.frameId;
    const reviewFrameId = frameHoldingPane(split, reviewPaneId)!.frameId;
    const moved = movePaneToFrame(split, reviewPaneId, terminalFrameId, "tabs");

    expect(frameHoldingPane(moved, reviewPaneId)!.frameId).toBe(terminalFrameId);
    expect(moved.frames[reviewFrameId]).toBeUndefined();
    expect(frameIdsInLayout(moved.root)).toEqual([terminalFrameId]);
  });

  it("reorders tabs inside their own frame", () => {
    const { layout, reviewPaneId } = layoutWithReviewAndTerminal();
    const reordered = movePaneToFrame(
      layout,
      "pane-terminal",
      layout.activeFrameId,
      "tabs",
      reviewPaneId,
    );

    expect(reordered.frames[reordered.activeFrameId].paneIds).toEqual([
      "pane-terminal",
      reviewPaneId,
    ]);
  });

  it("leaves a lone tab alone when dropped on its own frame's edge", () => {
    const layout = defaultLayout("session-root");
    const reviewPaneId = findPaneOfKind(layout, "review")!.paneId;
    const unchanged = movePaneToFrame(layout, reviewPaneId, layout.activeFrameId, "left");

    expect(unchanged).toBe(layout);
  });

  it("resizes a split and keeps the sizes summing to one", () => {
    const { layout } = layoutWithReviewAndTerminal();
    const split = movePaneToFrame(layout, "pane-terminal", layout.activeFrameId, "right");
    const resized = setSplitSizes(split.root, [], [0.8, 0.2]);

    expect(resized.kind).toBe("split");
    if (resized.kind === "split") {
      expect(resized.sizes.reduce((sum, size) => sum + size, 0)).toBeCloseTo(1);
      expect(resized.sizes[0]).toBeGreaterThan(resized.sizes[1]);
    }
  });

  it("focuses the frame that holds the focused tab", () => {
    const { layout, reviewPaneId } = layoutWithReviewAndTerminal();
    const split = movePaneToFrame(layout, "pane-terminal", layout.activeFrameId, "right");
    const focused: WorkspaceLayout = focusPane(split, reviewPaneId);

    expect(focused.activeFrameId).toBe(frameHoldingPane(split, reviewPaneId)!.frameId);
  });

  it("opens a pane in a frame of its own beside the target", () => {
    const layout = defaultLayout("session-root");
    const reviewPaneId = findPaneOfKind(layout, "review")!.paneId;
    const split = addPaneInNewFrame(layout, layout.activeFrameId, "right", {
      paneId: "pane-terminal",
      kind: "terminal",
      terminalId: "terminal-0",
    });

    expect(frameIdsInLayout(split.root)).toHaveLength(2);
    expect(frameHoldingPane(split, "pane-terminal")!.frameId).toBe(split.activeFrameId);
    expect(frameHoldingPane(split, reviewPaneId)!.paneIds).toEqual([reviewPaneId]);
    if (split.root.kind === "split") {
      // The review keeps the left half.
      expect(split.root.children[0]).toEqual({ kind: "frame", frameId: layout.activeFrameId });
    }
  });

  it("opens a pane in a full-height column on the right", () => {
    const layout = defaultLayout("session-root");
    const withTerminal = addPaneInRightColumn(layout, {
      paneId: "pane-terminal",
      kind: "terminal",
      terminalId: "terminal-0",
    });

    expect(withTerminal.root.kind).toBe("split");
    if (withTerminal.root.kind === "split") {
      expect(withTerminal.root.direction).toBe("row");
      expect(withTerminal.root.children[1]).toEqual({
        kind: "frame",
        frameId: frameHoldingPane(withTerminal, "pane-terminal")!.frameId,
      });
      expect(withTerminal.root.sizes.reduce((sum, size) => sum + size, 0)).toBeCloseTo(1);
    }
  });

  it("finds the frame a kind of pane already lives in", () => {
    const layout = defaultLayout("session-root");
    const withTerminal = addPaneInRightColumn(layout, {
      paneId: "pane-terminal",
      kind: "terminal",
      terminalId: "terminal-0",
    });
    const terminalFrameId = frameHoldingPane(withTerminal, "pane-terminal")!.frameId;

    // Asked for from the review frame, the shell still joins the shells.
    expect(frameHoldingKind(withTerminal, "terminal", layout.activeFrameId)).toBe(terminalFrameId);
    expect(frameHoldingKind(withTerminal, "terminal", terminalFrameId)).toBe(terminalFrameId);
    expect(frameHoldingKind(layout, "terminal", layout.activeFrameId)).toBeNull();
    // And a review asked for from the shell frame joins the reviews.
    expect(frameHoldingKind(withTerminal, "review", terminalFrameId)).toBe(layout.activeFrameId);
  });

  it("rejects a stored layout whose panes predate what they must carry", () => {
    const layout = defaultLayout("session-root");
    const reviewPaneId = findPaneOfKind(layout, "review")!.paneId;
    const staleReview = {
      ...layout,
      panes: { [reviewPaneId]: { paneId: reviewPaneId, kind: "review" } },
    } as unknown as WorkspaceLayout;

    expect(isCoherentLayout(staleReview)).toBe(false);
  });

  it("puts the review back when a restored layout has none", () => {
    const layout = defaultLayout("session-root");
    const withoutReview = closePane(layout, findPaneOfKind(layout, "review")!.paneId);
    const restored = withReviewPane(withoutReview, "session-root");

    expect(findPaneOfKind(restored, "review")).not.toBeNull();
    expect(restored.frames[restored.activeFrameId].paneIds).toHaveLength(1);
  });

  it("leaves a restored layout that already has the review alone", () => {
    const layout = defaultLayout("session-root");
    expect(withReviewPane(layout, "session-root")).toBe(layout);
  });

  it("rejects a stored layout whose tree and frames disagree", () => {
    const layout = defaultLayout("session-root");
    expect(isCoherentLayout(layout)).toBe(true);
    expect(isCoherentLayout({ ...layout, frames: {} })).toBe(false);
    expect(isCoherentLayout({ ...layout, panes: {} })).toBe(false);
    expect(isCoherentLayout({ ...layout, activeFrameId: "gone" })).toBe(false);
  });

  it("starts empty with a single frame and no tabs", () => {
    const layout = emptyLayout();
    expect(Object.keys(layout.panes)).toHaveLength(0);
    expect(frameIdsInLayout(layout.root)).toEqual([layout.activeFrameId]);
  });
});
