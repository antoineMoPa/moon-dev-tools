import { createContext, useContext, useEffect, useMemo, useReducer } from "react";
import { toast } from "sonner";
import { ApiError, getSessionId } from "./api";
import {
  discardHunk as discardHunkRequest,
  fetchSessionState,
  saveComment as saveCommentRequest,
  sendCommentBatch as sendCommentBatchRequest,
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

type ReviewStoreState = {
  data: SessionState | null;
  activeView: ReviewView;
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
    toggleStage: (hunkId: string, staged: boolean) => Promise<void>;
    toggleStageFile: (filePath: string, staged: boolean) => Promise<void>;
    stageSelection: (hunkId: string, selection: string) => Promise<void>;
    discardHunk: (hunkId: string) => Promise<void>;
    updateDraftComment: (hunkId: string, comment: string) => void;
    upsertDraftComment: (draft: DraftComment) => void;
    removeDraftComment: (draftId: string) => void;
    saveComment: (hunkId: string, comment: string, batch?: boolean) => Promise<void>;
    setBatchDraftComments: (value: boolean) => void;
    sendCommentBatch: () => Promise<void>;
    setAgent: (agent: AgentKind) => Promise<void>;
    setActiveView: (view: ReviewView) => void;
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
  | { type: "active_view_set"; view: ReviewView };

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
      return {
        ...state,
        activeView: action.view,
      };
    default:
      return state;
  }
}

function initialReviewStoreState(): ReviewStoreState {
  return {
    data: null,
    activeView: ReviewView.All,
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
      return;
    }

    dispatch({ type: "request_started" });
    try {
      await request();
      await loadState();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Request failed.");
    } finally {
      dispatch({ type: "request_finished" });
    }
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
        toggleStage: async (hunkId, staged) => mutate(() => toggleStageRequest(hunkId, staged)),
        toggleStageFile: async (filePath, staged) => mutate(() => toggleStageFileRequest(filePath, staged)),
        stageSelection: async (hunkId, selection) => mutate(() => stageSelectionRequest(hunkId, selection)),
        discardHunk: async (hunkId) => mutate(() => discardHunkRequest(hunkId)),
        updateDraftComment,
        upsertDraftComment,
        removeDraftComment,
        saveComment: async (hunkId, comment, batch) => mutate(() => saveCommentRequest(hunkId, comment, batch)),
        setBatchDraftComments,
        sendCommentBatch: async () => mutate(() => sendCommentBatchRequest()),
        setAgent: async (agent) => mutate(() => updateAgentRequest(agent)),
        setActiveView,
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
