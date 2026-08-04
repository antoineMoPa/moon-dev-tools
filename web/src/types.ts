export type AgentKind = "none" | "claude" | "codex" | "opencode";
export type TerminalCommand = Exclude<AgentKind, "none">;

export const COMMENT_DISPATCH_STATUS = {
  idle: "idle",
  batched: "batched",
  queued: "queued",
  running: "running",
  canceled: "canceled",
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
  key: string;
  status: CommentDispatchStatus;
  detail: string;
  agent: AgentKind;
  can_cancel: boolean;
  has_log: boolean;
};

export type Commit = {
  sha: string;
  short_sha: string;
  subject: string;
  author: string;
  review_status: CommitReviewStatus;
};

export type CommitHistoryPage = {
  commits: Commit[];
  has_more: boolean;
};

export type LocalChangeSummary = {
  modified: number;
  added: number;
  deleted: number;
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
  moved_from?: HunkMoveHint | null;
  moved_to?: HunkMoveHint | null;
  image_diff?: ImageDiff | null;
};

export type ImageDiff = {
  before_src?: string | null;
  after_src?: string | null;
};

export type HunkMoveHint = {
  target_hunk_id: string;
  target_file_path: string;
  target_header: string;
  score: number;
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

export type ReviewComment = {
  hunk_id: string;
  comment_index: number;
  file_path: string;
  header: string;
  selection: string;
  comment: string;
  resolved: boolean;
  dispatch: CommentDispatch;
  jumpable: boolean;
};

export type SessionState = {
  repo_name: string;
  branch_name?: string | null;
  commit_base?: string | null;
  commits: Commit[];
  history_commits: Commit[];
  history_has_more: boolean;
  local_change_summary: LocalChangeSummary;
  active_commit?: string | null;
  repo_path: string;
  read_only: boolean;
  patch_preview_line_limit: number;
  available_agents: AgentOption[];
  selected_agent: AgentKind;
  full_file_path?: string | null;
  hunks: Hunk[];
  review_comments: ReviewComment[];
  export_text: string;
};

export type PatchPayload = {
  patch: string;
};

export type FileContentPayload = {
  file_path: string;
  content: string;
};

export type AgentLogPayload = {
  dispatch_key: string;
  text: string;
};

/// A changed submodule of a reviewed repo, offered as another review to open.
export type Submodule = {
  repo_path: string;
  name: string;
};
