import type {
  AgentKind,
  AgentLogPayload,
  CommitHistoryPage,
  FileContentPayload,
  PatchPayload,
  SessionState,
  Submodule,
  TerminalCommand,
} from "./types";

function parseSessionId(pathname: string): string {
  const segments = pathname.split("/").filter(Boolean);
  const reviewIndex = segments.indexOf("review");
  if (reviewIndex === -1) {
    return "";
  }

  return segments[reviewIndex + 1] ?? "";
}

/// The review the page was opened on. Other reviews (submodules) get their own ids.
const rootSessionId = parseSessionId(window.location.pathname);

export class ApiError extends Error {
  readonly isTimeout: boolean;

  constructor(message: string, options?: { isTimeout?: boolean }) {
    super(message);
    this.name = "ApiError";
    this.isTimeout = options?.isTimeout ?? false;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      headers: { "content-type": "application/json" },
      ...init,
    });
  } catch (error) {
    throw new ApiError("Server probably went to sleep; launch moonreview again.", { isTimeout: true });
  }

  if (!response.ok) {
    const text = await response.text();
    throw new ApiError(text || `Request failed: ${response.status}`);
  }
  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    return response.json() as Promise<T>;
  }
  return response.text() as Promise<T>;
}

export function getRootSessionId(): string {
  return rootSessionId;
}

export function terminalSocketUrl(terminalId: string): string {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}/api/session/${rootSessionId}/terminals/${terminalId}/socket`;
}

/// The shell starts in the repo of the session it is created against, so a shell opened
/// while reviewing a submodule starts in that submodule.
export function createTerminal(
  sessionId: string,
  command?: TerminalCommand,
): Promise<{ terminal_id: string }> {
  return request<{ terminal_id: string }>(`/api/session/${sessionId}/terminals`, {
    method: "POST",
    body: JSON.stringify({ command: command ?? null }),
  });
}

export function fetchTerminalIds(): Promise<{ terminal_ids: string[] }> {
  return request<{ terminal_ids: string[] }>(`/api/session/${rootSessionId}/terminals`);
}

export function closeTerminal(terminalId: string): Promise<{ terminal_ids: string[] }> {
  return request<{ terminal_ids: string[] }>(
    `/api/session/${rootSessionId}/terminals/${terminalId}`,
    { method: "DELETE" },
  );
}

/// Every submodule of a reviewed repo, with how many files are changed in each.
export function fetchSubmodules(sessionId: string): Promise<Submodule[]> {
  return request<Submodule[]>(`/api/session/${sessionId}/submodules`);
}

/// Opening the same repo and target twice returns the same session, so this doubles as a
/// lookup for a submodule's session id.
export function openSession(repoPath: string): Promise<{ session_id: string }> {
  return request<{ session_id: string }>(`/api/session/open`, {
    method: "POST",
    body: JSON.stringify({ repo_path: repoPath }),
  });
}

export function fetchSessionState(sessionId: string): Promise<SessionState> {
  return request<SessionState>(`/api/session/${sessionId}/state`);
}

export function fetchCommitHistory(
  sessionId: string,
  offset: number,
  limit = 30,
): Promise<CommitHistoryPage> {
  const params = new URLSearchParams({ offset: String(offset), limit: String(limit) });
  return request<CommitHistoryPage>(`/api/session/${sessionId}/history?${params.toString()}`);
}

export function fetchHunkPatch(sessionId: string, hunkId: string): Promise<PatchPayload> {
  return request<PatchPayload>(`/api/session/${sessionId}/hunk/${hunkId}`);
}

export function fetchFileContent(sessionId: string, filePath: string): Promise<FileContentPayload> {
  const params = new URLSearchParams({ file_path: filePath });
  return request<FileContentPayload>(`/api/session/${sessionId}/file?${params.toString()}`);
}

export function setActiveCommit(sessionId: string, commit: string | null): Promise<string> {
  return request<string>(`/api/session/${sessionId}/commit`, {
    method: "POST",
    body: JSON.stringify({ commit }),
  });
}

export function toggleStage(sessionId: string, hunkId: string, staged: boolean): Promise<string> {
  return request<string>(`/api/session/${sessionId}/${staged ? "unstage" : "stage"}`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId }),
  });
}

export function toggleStageFile(
  sessionId: string,
  filePath: string,
  staged: boolean,
): Promise<string> {
  return request<string>(`/api/session/${sessionId}/${staged ? "unstage-file" : "stage-file"}`, {
    method: "POST",
    body: JSON.stringify({ file_path: filePath }),
  });
}

export function stageSelection(
  sessionId: string,
  hunkId: string,
  selection: string,
): Promise<string> {
  return request<string>(`/api/session/${sessionId}/stage-selection`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId, selection }),
  });
}

export function discardHunk(sessionId: string, hunkId: string): Promise<string> {
  return request<string>(`/api/session/${sessionId}/discard`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId }),
  });
}

export function discardHunks(sessionId: string, hunkIds: string[]): Promise<string> {
  return request<string>(`/api/session/${sessionId}/discard-batch`, {
    method: "POST",
    body: JSON.stringify({ hunk_ids: hunkIds }),
  });
}

export function saveComment(
  sessionId: string,
  hunkId: string,
  comment: string,
  batch = false,
): Promise<string> {
  return request<string>(`/api/session/${sessionId}/comment`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId, comment, batch }),
  });
}

export function sendCommentBatch(sessionId: string): Promise<string> {
  return request<string>(`/api/session/${sessionId}/comment-batch`, {
    method: "POST",
  });
}

export function cancelCommentDispatch(
  sessionId: string,
  hunkId: string,
  commentIndex: number,
): Promise<string> {
  return request<string>(`/api/session/${sessionId}/comment-dispatch/cancel`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId, comment_index: commentIndex }),
  });
}

export function fetchAgentLog(sessionId: string, dispatchKey: string): Promise<AgentLogPayload> {
  const params = new URLSearchParams({ dispatch_key: dispatchKey });
  return request<AgentLogPayload>(
    `/api/session/${sessionId}/agent-dispatch/log?${params.toString()}`,
  );
}

export function updateAgent(sessionId: string, agent: AgentKind): Promise<string> {
  return request<string>(`/api/session/${sessionId}/agent`, {
    method: "POST",
    body: JSON.stringify({ agent }),
  });
}
