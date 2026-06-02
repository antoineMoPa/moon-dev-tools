import { createContext, useContext, useEffect, useMemo, useReducer } from "react";
import { toast } from "sonner";
import { ApiError, getSessionId } from "./api";
import {
  cancelCommentDispatch as cancelCommentDispatchRequest,
  discardHunk as discardHunkRequest,
  discardHunks as discardHunksRequest,
  fetchSessionState,
  saveComment as saveCommentRequest,
  sendCommentBatch as sendCommentBatchRequest,
  setActiveCommit as setActiveCommitRequest,
  setFileReviewed as setFileReviewedRequest,
  setReviewed as setReviewedRequest,
  stageSelection as stageSelectionRequest,
  toggleStage as toggleStageRequest,
  toggleStageFile as toggleStageFileRequest,
  updateAgent as updateAgentRequest,
} from "./api";
import { parseAnchoredComments } from "./anchoredComments";
import { reconcileDraftComments } from "./draftCommentAnchoring";
import { loadDraftComments, persistDraftComments } from "./comments";
import { COMMENT_DISPATCH_STATUS } from "./types";
import type { AgentKind, DraftComment, Hunk, SessionState } from "./types";

export enum ReviewView {
  All = "all",
  File = "file",
}

export type MovedDiffLayout = "unified" | "side-by-side";

const MOVED_DIFF_LAYOUT_STORAGE_KEY = "moonreview:moved-diff-layout";

type ReviewStoreState = {
  data: SessionState | null;
  activeView: ReviewView;
  activeHunkId: string | null;
  movedDiffLayout: MovedDiffLayout;
  draftComments: DraftComment[];
  batchDraftComments: boolean;
  loadError: string;
  busy: boolean;
  pollingStopped: boolean;
  timeoutToastShown: boolean;
};

type ReviewStoreValue = {
  state: ReviewStoreState;
  actions: {
    loadState: () => Promise<void>;
    setActiveCommit: (commit: string | null) => Promise<void>;
    setReviewed: (hunkId: string, reviewed: boolean) => Promise<boolean>;
    setFileReviewed: (filePath: string, reviewed: boolean) => Promise<void>;
    toggleStage: (hunkId: string, staged: boolean) => Promise<boolean>;
    stageHunks: (hunks: Array<{ hunkId: string; staged: boolean }>) => Promise<boolean>;
    toggleStageFile: (filePath: string, staged: boolean) => Promise<void>;
    stageSelection: (hunkId: string, selection: string) => Promise<void>;
    discardHunk: (hunkId: string) => Promise<void>;
    discardHunks: (hunkIds: string[]) => Promise<void>;
    updateDraftComment: (hunkId: string, comment: string) => void;
    upsertDraftComment: (draft: DraftComment) => void;
    removeDraftComment: (draftId: string) => void;
    saveComment: (hunkId: string, comment: string, batch?: boolean) => Promise<void>;
    setBatchDraftComments: (value: boolean) => void;
    sendCommentBatch: () => Promise<void>;
    cancelCommentDispatch: (hunkId: string, commentIndex: number) => Promise<void>;
    setAgent: (agent: AgentKind) => Promise<void>;
    setActiveView: (view: ReviewView) => void;
    setActiveHunkId: (hunkId: string | null) => void;
    setMovedDiffLayout: (layout: MovedDiffLayout) => void;
  };
};

type ReviewStoreAction =
  | { type: "request_started" }
  | { type: "request_finished" }
  | { type: "state_loaded"; data: SessionState }
  | { type: "load_failed"; message: string }
  | { type: "polling_stopped"; message: string }
  | { type: "timeout_toast_shown" }
  | { type: "draft_comment_updated"; hunkId: string; comment: string }
  | { type: "draft_comment_upserted"; draft: DraftComment }
  | { type: "draft_comment_removed"; draftId: string }
  | { type: "batch_draft_comments_set"; value: boolean }
  | { type: "active_view_set"; view: ReviewView }
  | { type: "active_hunk_set"; hunkId: string | null }
  | { type: "moved_diff_layout_set"; layout: MovedDiffLayout };

const ReviewStoreContext = createContext<ReviewStoreValue | null>(null);

function exportServerUrl(): string {
  return window.location.origin.replace("127.0.0.1", "localhost");
}

function buildExportText(hunks: Hunk[]): string {
  const lines = ["Moon Review notes", "=================", "Please fix these code issues and mark as resolved:", ""];
  const sessionId = getSessionId();

  for (const hunk of hunks.filter((item) => item.comment.trim())) {
    const anchored = parseAnchoredComments(hunk.comment);
    const unresolved = anchored.filter((entry) => !entry.resolved);
    if (unresolved.length === 0) {
      continue;
    }

    lines.push(`${hunk.file_path} ${hunk.header}`);
    for (const entry of unresolved) {
      const commentIndex = anchored.indexOf(entry);
      lines.push(`Selected code: ${entry.selection}`);
      lines.push(`Issue: ${entry.comment}`);
      lines.push(
        `Poke this url when done: ${exportServerUrl()}/api/session/${sessionId}/resolve/${hunk.id}/${commentIndex}`,
      );
      lines.push("");
    }
  }

  if (lines.join("\n").trim() === "Moon Review notes\n=================\nPlease fix these code issues and mark as resolved:") {
    lines.push("No review notes yet.");
  }

  return lines.join("\n");
}

function updateHunkComment(data: SessionState, hunkId: string, comment: string): SessionState {
  const hunks = data.hunks.map((hunk) => (hunk.id === hunkId ? { ...hunk, comment } : hunk));
  return {
    ...data,
    hunks,
    export_text: buildExportText(hunks),
  };
}

function loadMovedDiffLayout(): MovedDiffLayout {
  return window.localStorage.getItem(MOVED_DIFF_LAYOUT_STORAGE_KEY) === "side-by-side"
    ? "side-by-side"
    : "unified";
}

function reviewStoreReducer(state: ReviewStoreState, action: ReviewStoreAction): ReviewStoreState {
  switch (action.type) {
    case "request_started":
      return { ...state, busy: true };
    case "request_finished":
      return { ...state, busy: false };
    case "state_loaded":
      return {
        ...state,
        data: action.data,
        draftComments: reconcileDraftComments(state.draftComments, action.data.hunks),
        loadError: "",
        pollingStopped: false,
        timeoutToastShown: false,
      };
    case "load_failed":
      return {
        ...state,
        loadError: action.message,
      };
    case "polling_stopped":
      return {
        ...state,
        loadError: action.message,
        pollingStopped: true,
      };
    case "timeout_toast_shown":
      return {
        ...state,
        timeoutToastShown: true,
      };
    case "draft_comment_updated":
      if (!state.data) {
        return state;
      }
      return {
        ...state,
        data: updateHunkComment(state.data, action.hunkId, action.comment),
      };
    case "draft_comment_upserted":
      return {
        ...state,
        draftComments: [
          ...state.draftComments.filter((draft) => draft.id !== action.draft.id),
          action.draft,
        ],
      };
    case "draft_comment_removed":
      return {
        ...state,
        draftComments: state.draftComments.filter((draft) => draft.id !== action.draftId),
      };
    case "batch_draft_comments_set":
      return {
        ...state,
        batchDraftComments: action.value,
      };
    case "active_view_set":
      if (state.activeView === action.view) {
        return state;
      }
      return {
        ...state,
        activeView: action.view,
      };
    case "active_hunk_set":
      if (state.activeHunkId === action.hunkId) {
        return state;
      }
      return {
        ...state,
        activeHunkId: action.hunkId,
      };
    case "moved_diff_layout_set":
      if (state.movedDiffLayout === action.layout) {
        return state;
      }
      return {
        ...state,
        movedDiffLayout: action.layout,
      };
    default:
      return state;
  }
}

function initialReviewStoreState(): ReviewStoreState {
  return {
    data: null,
    activeView: ReviewView.All,
    activeHunkId: null,
    movedDiffLayout: loadMovedDiffLayout(),
    draftComments: loadDraftComments(getSessionId()),
    batchDraftComments: false,
    loadError: "",
    busy: false,
    pollingStopped: false,
    timeoutToastShown: false,
  };
}

function isTimeoutError(error: unknown): error is ApiError {
  return error instanceof ApiError && error.isTimeout;
}

function hasActiveDispatches(data: SessionState | null): boolean {
  if (!data) {
    return false;
  }

  return (
    data.hunks.some((hunk) =>
      hunk.comment_dispatches.some(
        (dispatch) =>
          dispatch.status === COMMENT_DISPATCH_STATUS.queued ||
          dispatch.status === COMMENT_DISPATCH_STATUS.running,
      ),
    ) ||
    data.sidebar_comments.some(
      (comment) =>
        comment.dispatch_status === COMMENT_DISPATCH_STATUS.queued ||
        comment.dispatch_status === COMMENT_DISPATCH_STATUS.running,
    )
  );
}

export function ReviewStoreProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(reviewStoreReducer, undefined, initialReviewStoreState);

  async function loadStateInternal(showErrorToast: boolean) {
    try {
      const data = await fetchSessionState();
      dispatch({ type: "state_loaded", data });
    } catch (error) {
      const message = error instanceof Error ? error.message : "Failed to load state.";
      if (isTimeoutError(error)) {
        dispatch({ type: "polling_stopped", message });
        if (showErrorToast && !state.timeoutToastShown) {
          toast.error(message, { id: "server-timeout" });
          dispatch({ type: "timeout_toast_shown" });
        }
        return;
      }

      dispatch({ type: "load_failed", message });
      if (showErrorToast) {
        toast.error(message);
      }
    }
  }

  async function loadState() {
    await loadStateInternal(true);
  }

  async function mutate(request: () => Promise<unknown>) {
    if (state.busy) {
      return false;
    }

    dispatch({ type: "request_started" });
    try {
      await request();
      await loadState();
      return true;
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Request failed.");
      return false;
    } finally {
      dispatch({ type: "request_finished" });
    }
  }

  async function toggleStage(hunkId: string, staged: boolean) {
    return mutate(() => toggleStageRequest(hunkId, staged));
  }

  async function stageHunks(hunks: Array<{ hunkId: string; staged: boolean }>) {
    return mutate(async () => {
      for (const hunk of hunks) {
        if (!hunk.staged) {
          await toggleStageRequest(hunk.hunkId, hunk.staged);
        }
      }
    });
  }

  async function setReviewed(hunkId: string, reviewed: boolean) {
    return mutate(() => setReviewedRequest(hunkId, reviewed));
  }

  function updateDraftComment(hunkId: string, comment: string) {
    dispatch({ type: "draft_comment_updated", hunkId, comment });
  }

  function upsertDraftComment(draft: DraftComment) {
    dispatch({ type: "draft_comment_upserted", draft });
  }

  function removeDraftComment(draftId: string) {
    dispatch({ type: "draft_comment_removed", draftId });
  }

  function setBatchDraftComments(value: boolean) {
    dispatch({ type: "batch_draft_comments_set", value });
  }

  function setActiveView(view: ReviewView) {
    dispatch({ type: "active_view_set", view });
  }

  function setActiveHunkId(hunkId: string | null) {
    dispatch({ type: "active_hunk_set", hunkId });
  }

  function setMovedDiffLayout(layout: MovedDiffLayout) {
    window.localStorage.setItem(MOVED_DIFF_LAYOUT_STORAGE_KEY, layout);
    dispatch({ type: "moved_diff_layout_set", layout });
  }

  useEffect(() => {
    void loadState();
  }, []);

  useEffect(() => {
    persistDraftComments(getSessionId(), state.draftComments);
  }, [state.draftComments]);

  useEffect(() => {
    if (state.pollingStopped || !hasActiveDispatches(state.data)) {
      return;
    }

    const timer = window.setInterval(() => {
      void loadStateInternal(false);
    }, 1500);

    return () => window.clearInterval(timer);
  }, [state.data, state.pollingStopped]);

  const value = useMemo<ReviewStoreValue>(
    () => ({
      state,
      actions: {
        loadState,
        setActiveCommit: async (commit) => {
          dispatch({ type: "active_view_set", view: ReviewView.All });
          await mutate(() => setActiveCommitRequest(commit));
        },
        setReviewed,
        setFileReviewed: async (filePath, reviewed) => {
          await mutate(() => setFileReviewedRequest(filePath, reviewed));
        },
        toggleStage,
        stageHunks,
        toggleStageFile: async (filePath, staged) => {
          await mutate(() => toggleStageFileRequest(filePath, staged));
        },
        stageSelection: async (hunkId, selection) => {
          await mutate(() => stageSelectionRequest(hunkId, selection));
        },
        discardHunk: async (hunkId) => {
          await mutate(() => discardHunkRequest(hunkId));
        },
        discardHunks: async (hunkIds) => {
          await mutate(() => discardHunksRequest(hunkIds));
        },
        updateDraftComment,
        upsertDraftComment,
        removeDraftComment,
        saveComment: async (hunkId, comment, batch) => {
          await mutate(() => saveCommentRequest(hunkId, comment, batch));
        },
        setBatchDraftComments,
        sendCommentBatch: async () => {
          await mutate(() => sendCommentBatchRequest());
        },
        cancelCommentDispatch: async (hunkId, commentIndex) => {
          await mutate(() => cancelCommentDispatchRequest(hunkId, commentIndex));
        },
        setAgent: async (agent) => {
          await mutate(() => updateAgentRequest(agent));
        },
        setActiveView,
        setActiveHunkId,
        setMovedDiffLayout,
      },
    }),
    [state],
  );

  return <ReviewStoreContext.Provider value={value}>{children}</ReviewStoreContext.Provider>;
}

export function useReviewStore() {
  const value = useContext(ReviewStoreContext);
  if (!value) {
    throw new Error("useReviewStore must be used within ReviewStoreProvider");
  }
  return value;
}

export function useOptionalReviewStore() {
  return useContext(ReviewStoreContext);
}
