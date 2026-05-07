import { useMemo } from "react";
import { ReviewView, useReviewStore } from "../reviewStore";
import { COMMENT_DISPATCH_STATUS } from "../types";
import type { CommentDispatchStatus, SessionState, SidebarComment } from "../types";
import { SidebarCommentsSection } from "./SidebarCommentsSection";
import { SidebarFilesSection } from "./SidebarFilesSection";
import { buildSidebarFiles, FILE_STAGE_STATUS } from "./sidebarFiles";
import type { SidebarFileItem } from "./sidebarFiles";

export type SidebarCommentItem = {
  id: string;
  hunkId: string;
  commentAnchorId: string | null;
  filePath: string;
  comment: string;
  selection: string;
  resolved: boolean;
  statusLabel: string;
};

type LeftSidebarProps = {
  data: SessionState;
  snoozedFiles: Set<string>;
  onJumpToFile: (filePath: string) => void;
  onJumpToComment: (target: { filePath: string; hunkId: string; elementId: string }) => void;
  activeFilePath?: string | null;
  onStageWholeFile?: (file: SidebarFileItem) => void;
};

function statusLabel(resolved: boolean, status: CommentDispatchStatus) {
  if (status === COMMENT_DISPATCH_STATUS.completed) {
    return "complete";
  }
  if (status === COMMENT_DISPATCH_STATUS.failed) {
    return "failed";
  }
  if (status === COMMENT_DISPATCH_STATUS.running) {
    return "running";
  }
  if (status === COMMENT_DISPATCH_STATUS.queued) {
    return "queued";
  }
  if (status === COMMENT_DISPATCH_STATUS.batched) {
    return "batched";
  }
  return resolved ? "resolved" : "open";
}

function buildSidebarComments(data: SessionState): SidebarCommentItem[] {
  return data.sidebar_comments.map((comment, index) => buildSidebarCommentItem(comment, index));
}

function buildSidebarCommentItem(comment: SidebarComment, index: number): SidebarCommentItem {
  return {
    id: `${comment.hunk_id}:${comment.comment_index}:${index}`,
    hunkId: comment.hunk_id,
    commentAnchorId: comment.jumpable ? `comment-${comment.hunk_id}-${comment.comment_index}` : null,
    filePath: comment.file_path,
    comment: comment.comment,
    selection: comment.selection,
    resolved: comment.resolved,
    statusLabel: statusLabel(comment.resolved, comment.dispatch_status),
  };
}

type SidebarSummaryProps = {
  commentCount: number;
  fileCount: number;
};

function SidebarSummary({ commentCount, fileCount }: SidebarSummaryProps) {
  return (
    <div className="left-sidebar-head">
      <p className="sidebar-eyebrow meta">
        {commentCount} comments across {fileCount} files
      </p>
    </div>
  );
}

function SidebarShortcutsHint() {
  const {
    state: { activeHunkId, data },
  } = useReviewStore();
  const activeHunk = data?.hunks.find((hunk) => hunk.id === activeHunkId) ?? null;

  if (!activeHunk) {
    return null;
  }

  return (
    <div className="sidebar-shortcuts">
      <div className="sidebar-shortcuts-list">
        {activeHunk.staged ? (
          <p><kbd>u</kbd> unstage current hunk</p>
        ) : (
          <p><kbd>s</kbd> stage current hunk</p>
        )}
      </div>
    </div>
  );
}

export function LeftSidebar({
  data,
  snoozedFiles,
  onJumpToFile,
  onJumpToComment,
  activeFilePath,
  onStageWholeFile,
}: LeftSidebarProps) {
  const {
    state: { activeView, busy },
    actions,
  } = useReviewStore();
  const isViewingAll = activeView === ReviewView.All;
  const sidebarFiles = useMemo(() => buildSidebarFiles(data, snoozedFiles), [data, snoozedFiles]);
  const sidebarComments = useMemo(() => buildSidebarComments(data), [data]);

  return (
    <aside className="left-sidebar">
      <SidebarSummary commentCount={sidebarComments.length} fileCount={sidebarFiles.length} />
      <section className="sidebar-section sidebar-view-section">
        <div className="sidebar-list">
          <div className="sidebar-link">
            <button
              className={`sidebar-link-action ${isViewingAll ? "sidebar-link-active" : ""}`.trim()}
              type="button"
              onClick={() => {
                window.scrollTo({ top: 0, left: 0, behavior: "auto" });
                actions.setActiveView(ReviewView.All);
              }}
            >
              <span className="sidebar-link-name">all</span>
            </button>
          </div>
        </div>
      </section>
      <SidebarFilesSection
        files={sidebarFiles}
        activeFilePath={isViewingAll ? null : activeFilePath}
        readOnly={data.read_only}
        busy={busy}
        onJumpToFile={onJumpToFile}
        onToggleFileStage={(file) => {
          const shouldUnstage = file.status === FILE_STAGE_STATUS.staged;
          if (!shouldUnstage) {
            onStageWholeFile?.(file);
          }
          void actions.toggleStageFile(file.filePath, shouldUnstage);
        }}
      />
      <SidebarCommentsSection comments={sidebarComments} onJumpToComment={onJumpToComment} />
      <SidebarShortcutsHint />
    </aside>
  );
}
