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

export type CommitHistoryPage = {
  commits: Commit[];
  has_more: boolean;
};

export type LocalChangeSummary = {
  modified: number;
  added: number;
  deleted: number;
};

export type ExecutiveSummaryItem = {
  file_path: string;
  label: string;
  reason: string;
  byte_size?: number | null;
  line_count?: number | null;
  added_line_count: number;
  removed_line_count: number;
  hunk_count: number;
};

export type ExecutiveSummary = {
  large_files: ExecutiveSummaryItem[];
  large_new_files: ExecutiveSummaryItem[];
  hotspots: ExecutiveSummaryItem[];
  complexity_hints: ExecutiveSummaryItem[];
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
  history_commits: Commit[];
  history_has_more: boolean;
  local_change_summary: LocalChangeSummary;
  executive_summary?: ExecutiveSummary | null;
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
