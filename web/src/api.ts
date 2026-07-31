import type {
  AgentKind,
  AgentLogPayload,
  CommitHistoryPage,
  FileContentPayload,
  PatchPayload,
  SessionState,
} from "./types";

function parseSessionId(pathname: string): string {
  const segments = pathname.split("/").filter(Boolean);
  const reviewIndex = segments.indexOf("review");
  if (reviewIndex === -1) {
    return "";
  }

  return segments[reviewIndex + 1] ?? "";
}

const sessionId = parseSessionId(window.location.pathname);

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

export function getSessionId(): string {
  return sessionId;
}

export function terminalSocketUrl(terminalId: string): string {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}/api/session/${sessionId}/terminals/${terminalId}/socket`;
}

export function createTerminal(): Promise<{ terminal_id: string }> {
  return request<{ terminal_id: string }>(`/api/session/${sessionId}/terminals`, {
    method: "POST",
  });
}

export function fetchTerminalIds(): Promise<{ terminal_ids: string[] }> {
  return request<{ terminal_ids: string[] }>(`/api/session/${sessionId}/terminals`);
}

export function closeTerminal(terminalId: string): Promise<{ terminal_ids: string[] }> {
  return request<{ terminal_ids: string[] }>(
    `/api/session/${sessionId}/terminals/${terminalId}`,
    { method: "DELETE" },
  );
}

export function fetchSessionState(): Promise<SessionState> {
  return request<SessionState>(`/api/session/${sessionId}/state`);
}

export function fetchCommitHistory(offset: number, limit = 30): Promise<CommitHistoryPage> {
  const params = new URLSearchParams({ offset: String(offset), limit: String(limit) });
  return request<CommitHistoryPage>(`/api/session/${sessionId}/history?${params.toString()}`);
}

export function fetchHunkPatch(hunkId: string): Promise<PatchPayload> {
  return request<PatchPayload>(`/api/session/${sessionId}/hunk/${hunkId}`);
}

export function fetchFileContent(filePath: string): Promise<FileContentPayload> {
  const params = new URLSearchParams({ file_path: filePath });
  return request<FileContentPayload>(`/api/session/${sessionId}/file?${params.toString()}`);
}

export function toggleReviewed(hunkId: string): Promise<string> {
  return request<string>(`/api/session/${sessionId}/reviewed`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId }),
  });
}

export function setReviewed(hunkId: string, reviewed: boolean): Promise<string> {
  return request<string>(`/api/session/${sessionId}/reviewed`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId, reviewed }),
  });
}

export function setFileReviewed(filePath: string, reviewed: boolean): Promise<string> {
  return request<string>(`/api/session/${sessionId}/reviewed-file`, {
    method: "POST",
    body: JSON.stringify({ file_path: filePath, reviewed }),
  });
}

export function setActiveCommit(commit: string | null): Promise<string> {
  return request<string>(`/api/session/${sessionId}/commit`, {
    method: "POST",
    body: JSON.stringify({ commit }),
  });
}

export function toggleStage(hunkId: string, staged: boolean): Promise<string> {
  return request<string>(`/api/session/${sessionId}/${staged ? "unstage" : "stage"}`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId }),
  });
}

export function toggleStageFile(filePath: string, staged: boolean): Promise<string> {
  return request<string>(`/api/session/${sessionId}/${staged ? "unstage-file" : "stage-file"}`, {
    method: "POST",
    body: JSON.stringify({ file_path: filePath }),
  });
}

export function stageSelection(hunkId: string, selection: string): Promise<string> {
  return request<string>(`/api/session/${sessionId}/stage-selection`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId, selection }),
  });
}

export function discardHunk(hunkId: string): Promise<string> {
  return request<string>(`/api/session/${sessionId}/discard`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId }),
  });
}

export function discardHunks(hunkIds: string[]): Promise<string> {
  return request<string>(`/api/session/${sessionId}/discard-batch`, {
    method: "POST",
    body: JSON.stringify({ hunk_ids: hunkIds }),
  });
}

export function saveComment(hunkId: string, comment: string, batch = false): Promise<string> {
  return request<string>(`/api/session/${sessionId}/comment`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId, comment, batch }),
  });
}

export function sendCommentBatch(): Promise<string> {
  return request<string>(`/api/session/${sessionId}/comment-batch`, {
    method: "POST",
  });
}

export function cancelCommentDispatch(hunkId: string, commentIndex: number): Promise<string> {
  return request<string>(`/api/session/${sessionId}/comment-dispatch/cancel`, {
    method: "POST",
    body: JSON.stringify({ hunk_id: hunkId, comment_index: commentIndex }),
  });
}

export function fetchAgentLog(dispatchKey: string): Promise<AgentLogPayload> {
  const params = new URLSearchParams({ dispatch_key: dispatchKey });
  return request<AgentLogPayload>(`/api/session/${sessionId}/agent-dispatch/log?${params.toString()}`);
}

export function updateAgent(agent: AgentKind): Promise<string> {
  return request<string>(`/api/session/${sessionId}/agent`, {
    method: "POST",
    body: JSON.stringify({ agent }),
  });
}
