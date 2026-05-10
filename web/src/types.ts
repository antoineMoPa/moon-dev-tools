export type AgentKind = "none" | "claude" | "codex";

export const COMMENT_DISPATCH_STATUS = {
  idle: "idle",
  batched: "batched",
  queued: "queued",
  running: "running",
  completed: "completed",
  failed: "failed",
} as const;

export type CommentDispatchStatus =
  (typeof COMMENT_DISPATCH_STATUS)[keyof typeof COMMENT_DISPATCH_STATUS];

export type FileChangeKind = "added" | "deleted" | "modified";
export type CommitReviewStatus = "reviewed" | "partial" | "unreviewed";

export type AgentOption = {
  kind: AgentKind;
  label: string;
  available: boolean;
};

export type CommentDispatch = {
  status: CommentDispatchStatus;
  detail: string;
  can_cancel?: boolean;
};

export type Commit = {
  sha: string;
  short_sha: string;
  subject: string;
  author: string;
  review_status: CommitReviewStatus;
};

export type Hunk = {
  id: string;
  file_path: string;
  change_kind: FileChangeKind;
  header: string;
  staged: boolean;
  reviewed: boolean;
  comment: string;
  comment_dispatches: CommentDispatch[];
  patch_preview: string;
  patch_line_count: number;
  added_line_count: number;
  removed_line_count: number;
};

export type DraftComment = {
  id: string;
  hunkId: string;
  filePath: string;
  header: string;
  selectedText: string;
  note: string;
  lineNumberHint?: number;
};

export type SidebarComment = {
  hunk_id: string;
  comment_index: number;
  file_path: string;
  header: string;
  selection: string;
  comment: string;
  resolved: boolean;
  dispatch_status: CommentDispatchStatus;
  jumpable: boolean;
};

export type SessionState = {
  repo_name: string;
  branch_name?: string | null;
  commit_base?: string | null;
  commits: Commit[];
  active_commit?: string | null;
  repo_path: string;
  read_only: boolean;
  patch_preview_line_limit: number;
  available_agents: AgentOption[];
  selected_agent: AgentKind;
  hunks: Hunk[];
  sidebar_comments: SidebarComment[];
  export_text: string;
};

export type PatchPayload = {
  patch: string;
};

export type FileContentPayload = {
  file_path: string;
  content: string;
};
