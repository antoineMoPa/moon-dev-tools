import type { AgentKind, FileContentPayload, PatchPayload, SessionState } from "./types";

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

export function fetchSessionState(): Promise<SessionState> {
  return request<SessionState>(`/api/session/${sessionId}/state`);
}

export function fetchHunkPatch(hunkId: string): Promise<PatchPayload> {
  return request<PatchPayload>(`/api/session/${sessionId}/hunk/${hunkId}`);
}

export function fetchFileContent(filePath: string): Promise<FileContentPayload> {
  const params = new URLSearchParams({ file_path: filePath });
  return request<FileContentPayload>(`/api/session/${sessionId}/file?${params.toString()}`);
}

export function buildFullFileUrl(filePath: string, lineNumber?: number | null): string {
  const params = new URLSearchParams({ file_path: filePath });
  const hash = lineNumber ? `#L${lineNumber}` : "";
  return `/review/${sessionId}/file?${params.toString()}${hash}`;
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

export function updateAgent(agent: AgentKind): Promise<string> {
  return request<string>(`/api/session/${sessionId}/agent`, {
    method: "POST",
    body: JSON.stringify({ agent }),
  });
}
