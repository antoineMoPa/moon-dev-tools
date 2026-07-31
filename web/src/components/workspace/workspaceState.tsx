import { createContext, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { closeTerminal, createTerminal, fetchTerminalIds, getSessionId } from "../../api";
import {
  addPane,
  addPaneInNewFrame,
  closePane,
  defaultLayout,
  findPaneOfKind,
  focusPane,
  isCoherentLayout,
  makeId,
  mapPaneKindToMultiplicity,
  movePaneToFrame,
  replacePane,
  setSplitSizes,
  withReviewPane,
} from "./layout";
import type { DropSide, Pane, PaneKind, WorkspaceLayout } from "./layout";

const storageKey = `moonreview:workspace:${getSessionId()}`;

function readStoredLayout(): WorkspaceLayout | null {
  const stored = window.localStorage.getItem(storageKey);
  if (!stored) {
    return null;
  }

  try {
    const layout = JSON.parse(stored) as WorkspaceLayout;
    return isCoherentLayout(layout) ? layout : null;
  } catch {
    return null;
  }
}

function initialLayout(): WorkspaceLayout {
  const stored = readStoredLayout();
  return stored ? withReviewPane(stored) : defaultLayout();
}

/// A frame showing nothing but the review is the whole window: a new pane takes a frame of
/// its own on the right rather than hiding the diff behind a tab.
function openPaneInLayout(
  layout: WorkspaceLayout,
  frameId: string,
  pane: Pane,
): WorkspaceLayout {
  const targetFrameId = layout.frames[frameId] ? frameId : layout.activeFrameId;
  const targetFrame = layout.frames[targetFrameId];
  const holdsOnlyTheReview =
    targetFrame.paneIds.length === 1 && layout.panes[targetFrame.paneIds[0]].kind === "review";

  return holdsOnlyTheReview
    ? addPaneInNewFrame(layout, targetFrameId, "right", pane)
    : addPane(layout, targetFrameId, pane);
}

type WorkspaceValue = {
  layout: WorkspaceLayout;
  draggedPaneId: string | null;
  openPane: (kind: PaneKind, frameId?: string) => void;
  closePaneById: (paneId: string) => void;
  focusPaneById: (paneId: string) => void;
  movePane: (
    paneId: string,
    targetFrameId: string,
    side: DropSide,
    beforePaneId?: string | null,
  ) => void;
  restartTerminalPane: (paneId: string) => void;
  resizeSplit: (path: number[], sizes: number[]) => void;
  setDraggedPaneId: (paneId: string | null) => void;
};

const WorkspaceContext = createContext<WorkspaceValue | null>(null);

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const [layout, setLayout] = useState<WorkspaceLayout>(initialLayout);
  const [draggedPaneId, setDraggedPaneId] = useState<string | null>(null);

  useEffect(() => {
    window.localStorage.setItem(storageKey, JSON.stringify(layout));
  }, [layout]);

  // Shells outlive the browser tab, but not a server restart: drop tabs whose shell is gone.
  useEffect(() => {
    let cancelled = false;
    void fetchTerminalIds()
      .then(({ terminal_ids }) => {
        if (cancelled) {
          return;
        }
        const liveTerminalIds = new Set(terminal_ids);
        setLayout((current) => {
          const deadPaneIds = Object.values(current.panes)
            .filter((pane) => pane.kind === "terminal" && !liveTerminalIds.has(pane.terminalId))
            .map((pane) => pane.paneId);
          return deadPaneIds.reduce((next, paneId) => closePane(next, paneId), current);
        });
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, []);

  function openPane(kind: PaneKind, frameId?: string) {
    if (mapPaneKindToMultiplicity[kind] === "single") {
      const existingPane = findPaneOfKind(layout, kind);
      if (existingPane) {
        // Opening a pane that already exists just brings its tab forward.
        setLayout((current) => focusPane(current, existingPane.paneId));
        return;
      }
    }

    const targetFrameId = frameId ?? layout.activeFrameId;
    if (kind !== "terminal") {
      setLayout((current) => openPaneInLayout(current, targetFrameId, { paneId: makeId("pane"), kind }));
      return;
    }

    void createTerminal().then(({ terminal_id }) => {
      setLayout((current) =>
        openPaneInLayout(current, targetFrameId, {
          paneId: makeId("pane"),
          kind: "terminal",
          terminalId: terminal_id,
        }),
      );
    });
  }

  function closePaneById(paneId: string) {
    const pane = layout.panes[paneId];
    if (pane?.kind === "terminal") {
      void closeTerminal(pane.terminalId).catch(() => undefined);
    }
    setLayout((current) => closePane(current, paneId));
  }

  /// The shell behind a terminal tab is gone (it exited, or the server restarted): give the
  /// tab a fresh shell instead of making the user close and reopen it.
  function restartTerminalPane(paneId: string) {
    void createTerminal().then(({ terminal_id }) => {
      setLayout((current) => {
        const pane = current.panes[paneId];
        if (pane?.kind !== "terminal") {
          return current;
        }
        return replacePane(current, { ...pane, terminalId: terminal_id });
      });
    });
  }

  const value = useMemo<WorkspaceValue>(
    () => ({
      layout,
      draggedPaneId,
      openPane,
      closePaneById,
      focusPaneById: (paneId) => setLayout((current) => focusPane(current, paneId)),
      movePane: (paneId, targetFrameId, side, beforePaneId) =>
        setLayout((current) => movePaneToFrame(current, paneId, targetFrameId, side, beforePaneId)),
      restartTerminalPane,
      resizeSplit: (path, sizes) =>
        setLayout((current) => ({ ...current, root: setSplitSizes(current.root, path, sizes) })),
      setDraggedPaneId,
    }),
    [draggedPaneId, layout],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace() {
  const value = useContext(WorkspaceContext);
  if (!value) {
    throw new Error("useWorkspace must be used within WorkspaceProvider");
  }
  return value;
}

export function useOptionalWorkspace() {
  return useContext(WorkspaceContext);
}
