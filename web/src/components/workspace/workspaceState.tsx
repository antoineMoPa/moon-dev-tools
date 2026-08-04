import { createContext, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  closeTerminal,
  createTerminal,
  fetchSubmodules,
  fetchTerminalIds,
  getRootSessionId,
  openSession,
} from "../../api";
import {
  addPane,
  addPaneInNewFrame,
  addPaneInRightColumn,
  closePane,
  frameHoldingKind,
  defaultLayout,
  findPaneOfKind,
  findReviewPane,
  focusPane,
  isCoherentLayout,
  makeId,
  movePaneToFrame,
  primaryFrameId,
  replacePane,
  setSplitSizes,
  withReviewPane,
} from "./layout";
import type { OpenPaneRequest, Pane, PaneKind, DropSide, WorkspaceLayout } from "./layout";

const storageKey = `moonreview:workspace:${getRootSessionId()}`;

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
  return stored
    ? withReviewPane(stored, getRootSessionId())
    : defaultLayout(getRootSessionId());
}

/// Every submodule with changes gets a review tab beside the main review, on every load —
/// they are not offered anywhere else. Closing one hides it until the page is loaded again.
/// The main review stays in front.
function withSubmoduleReviewTabs(
  layout: WorkspaceLayout,
  submodules: SubmoduleReview[],
): WorkspaceLayout {
  const rootReviewPane = findReviewPane(layout, getRootSessionId());
  const withTabs = submodules.reduce(
    (current, submodule) =>
      findReviewPane(current, submodule.sessionId)
        ? current
        : openPaneInLayout(current, primaryFrameId(current.root), {
            paneId: makeId("pane"),
            kind: "review",
            sessionId: submodule.sessionId,
            title: submodule.name,
          }),
    layout,
  );

  return rootReviewPane ? focusPane(withTabs, rootReviewPane.paneId) : withTabs;
}

/// Where each kind of pane lands when it is opened.
///
/// Reviews stack as tabs next to the other reviews: one review at a time, in full, is
/// easier to read than several squeezed side by side. Shells go down the right of the
/// workspace, joining the ones already there. The agent monitor takes a frame beside the
/// review rather than hiding the diff behind a tab.
const mapPaneKindToPlacement: {
  [Kind in PaneKind]: (
    layout: WorkspaceLayout,
    targetFrameId: string,
    pane: Extract<Pane, { kind: Kind }>,
  ) => WorkspaceLayout;
} = {
  review: (layout, targetFrameId, pane) =>
    addPane(layout, frameHoldingKind(layout, "review", targetFrameId) ?? targetFrameId, pane),
  terminal: (layout, targetFrameId, pane) => {
    const terminalFrameId = frameHoldingKind(layout, "terminal", targetFrameId);
    return terminalFrameId
      ? addPane(layout, terminalFrameId, pane)
      : addPaneInRightColumn(layout, pane);
  },
  agents: (layout, targetFrameId, pane) => {
    const targetFrame = layout.frames[targetFrameId];
    const holdsOnlyTheReview =
      targetFrame.paneIds.length === 1 && layout.panes[targetFrame.paneIds[0]].kind === "review";
    return holdsOnlyTheReview
      ? addPaneInNewFrame(layout, targetFrameId, "right", pane)
      : addPane(layout, targetFrameId, pane);
  },
};

function openPaneInLayout(
  layout: WorkspaceLayout,
  frameId: string,
  pane: Pane,
): WorkspaceLayout {
  const targetFrameId = layout.frames[frameId] ? frameId : layout.activeFrameId;
  const place = mapPaneKindToPlacement[pane.kind] as (
    layout: WorkspaceLayout,
    targetFrameId: string,
    pane: Pane,
  ) => WorkspaceLayout;

  return place(layout, targetFrameId, pane);
}

/// A submodule of the reviewed repo that has its own changes, and the session that
/// reviews it. Resolved once at startup so the window menu knows what it can open.
export type SubmoduleReview = {
  name: string;
  sessionId: string;
};

type WorkspaceValue = {
  layout: WorkspaceLayout;
  draggedPaneId: string | null;
  submoduleReviews: SubmoduleReview[];
  /// False until the submodule sessions have been (re)opened on the server. A review of a
  /// submodule cannot load before then, since the server only knows a session once asked.
  submodulesResolved: boolean;
  /// The review the header and the agent monitor follow: the last one worked in.
  focusedReviewSessionId: string;
  openPane: (request: OpenPaneRequest, frameId?: string) => void;
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
  const [submoduleReviews, setSubmoduleReviews] = useState<SubmoduleReview[]>([]);
  const [submodulesResolved, setSubmodulesResolved] = useState(false);
  const [focusedReviewPaneId, setFocusedReviewPaneId] = useState<string | null>(null);

  const reviewPanes = Object.values(layout.panes).filter((pane) => pane.kind === "review");
  const focusedReviewPane =
    reviewPanes.find((pane) => pane.paneId === focusedReviewPaneId) ?? reviewPanes[0] ?? null;
  const focusedReviewSessionId =
    focusedReviewPane?.kind === "review" ? focusedReviewPane.sessionId : getRootSessionId();

  /// Focusing a review is what makes the header and the agent monitor follow it.
  function focusPaneById(paneId: string) {
    if (layout.panes[paneId]?.kind === "review") {
      setFocusedReviewPaneId(paneId);
    }
    setLayout((current) => focusPane(current, paneId));
  }

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

  function openPane(request: OpenPaneRequest, frameId?: string) {
    // A review of a session already on screen, or the agent monitor, is brought forward
    // rather than opened twice.
    const existingPane =
      request.kind === "review"
        ? findReviewPane(layout, request.sessionId)
        : request.kind === "agents"
          ? findPaneOfKind(layout, "agents")
          : null;
    if (existingPane) {
      setLayout((current) => focusPane(current, existingPane.paneId));
      return;
    }

    const targetFrameId = frameId ?? layout.activeFrameId;
    if (request.kind !== "terminal") {
      const pane: Pane = { paneId: makeId("pane"), ...request };
      if (pane.kind === "review") {
        setFocusedReviewPaneId(pane.paneId);
      }
      setLayout((current) => openPaneInLayout(current, targetFrameId, pane));
      return;
    }

    // Shells start where the user is reading, which for a submodule review is its folder.
    void createTerminal(focusedReviewSessionId, request.command).then(({ terminal_id }) => {
      setLayout((current) =>
        openPaneInLayout(current, targetFrameId, {
          paneId: makeId("pane"),
          kind: "terminal",
          terminalId: terminal_id,
          command: request.command,
        }),
      );
    });
  }

  // The submodules with changes of their own, each resolved to the session that reviews it.
  useEffect(() => {
    let cancelled = false;
    void fetchSubmodules(getRootSessionId())
      .then(async (submodules) =>
        Promise.all(
          submodules.map(async (submodule) => ({
            name: submodule.name,
            sessionId: (await openSession(submodule.repo_path)).session_id,
          })),
        ),
      )
      .then((reviews) => {
        if (cancelled) {
          return;
        }
        setSubmoduleReviews(reviews);
        setSubmodulesResolved(true);
        setLayout((current) => withSubmoduleReviewTabs(current, reviews));
      })
      .catch(() => {
        if (!cancelled) {
          setSubmodulesResolved(true);
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

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
    const pane = layout.panes[paneId];
    const command = pane?.kind === "terminal" ? pane.command : undefined;
    void createTerminal(focusedReviewSessionId, command).then(({ terminal_id }) => {
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
      submoduleReviews,
      submodulesResolved,
      focusedReviewSessionId,
      openPane,
      closePaneById,
      focusPaneById,
      movePane: (paneId, targetFrameId, side, beforePaneId) =>
        setLayout((current) => movePaneToFrame(current, paneId, targetFrameId, side, beforePaneId)),
      restartTerminalPane,
      resizeSplit: (path, sizes) =>
        setLayout((current) => ({ ...current, root: setSplitSizes(current.root, path, sizes) })),
      setDraggedPaneId,
    }),
    [draggedPaneId, focusedReviewSessionId, layout, submoduleReviews, submodulesResolved],
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
