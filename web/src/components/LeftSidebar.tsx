import { useMemo } from "react";
import { ReviewView, useReviewStore } from "../reviewStore";
import { COMMENT_DISPATCH_STATUS } from "../types";
import type { CommentDispatchStatus, Commit, SessionState, SidebarComment } from "../types";
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
  onShowAll: () => void;
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

function SidebarShortcutsHint() {
  const {
    state: { activeHunkId, data },
  } = useReviewStore();
  const activeHunk = data?.hunks.find((hunk) => hunk.id === activeHunkId) ?? null;
  const isCommitReview = Boolean(data?.active_commit);

  if (!activeHunk) {
    return null;
  }

  return (
    <div className="sidebar-shortcuts">
      <div className="sidebar-shortcuts-list">
        {isCommitReview ? (
          activeHunk.reviewed ? (
            <p><kbd>u</kbd> mark current hunk unreviewed</p>
          ) : (
            <p><kbd>s</kbd> mark current hunk reviewed</p>
          )
        ) : activeHunk.staged ? (
          <p><kbd>u</kbd> unstage current hunk</p>
        ) : (
          <p><kbd>s</kbd> stage current hunk</p>
        )}
      </div>
    </div>
  );
}

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

function localChangesSummary(files: SidebarFileItem[]) {
  const unstagedFiles = files.filter((file) => file.status !== FILE_STAGE_STATUS.staged);
  if (unstagedFiles.length === 0) {
    return "no unstaged changes";
  }

  const modified = unstagedFiles.filter((file) => file.changeKind === "modified").length;
  const added = unstagedFiles.filter((file) => file.changeKind === "added").length;
  const deleted = unstagedFiles.filter((file) => file.changeKind === "deleted").length;
  return [
    modified > 0 ? pluralize(modified, "modified file") : null,
    added > 0 ? pluralize(added, "new file") : null,
    deleted > 0 ? pluralize(deleted, "deleted file") : null,
  ].filter(Boolean).join(", ");
}

function SidebarCommitsSection({
  activeCommit,
  base,
  commits,
  localSummary,
  onSelectCommit,
}: {
  activeCommit?: string | null;
  base?: string | null;
  commits: Commit[];
  localSummary: string;
  onSelectCommit: (commit: string | null) => void;
}) {
  return (
    <section className="sidebar-section sidebar-commits-section">
      <div className="sidebar-section-head">
        <p>Commits</p>
        {base ? <span className="sidebar-section-meta">{base}</span> : null}
      </div>
      <div className="sidebar-list">
        <div className="sidebar-commit" title="local changes">
          <div className="sidebar-commit-topline">
            <button
              className={`sidebar-commit-subject ${!activeCommit ? "sidebar-commit-active" : ""}`.trim()}
              type="button"
              onClick={() => onSelectCommit(null)}
            >
              local changes
            </button>
          </div>
          <p className="sidebar-commit-meta">{localSummary}</p>
        </div>
        {commits.length === 0 ? (
          <p className="sidebar-empty">{base ? `No commits since ${base}.` : "No default branch found."}</p>
        ) : commits.map((commit) => (
          <div className="sidebar-commit" key={commit.sha} title={`${commit.short_sha} ${commit.subject}`}>
            <div className="sidebar-commit-topline">
              <button
                className={`sidebar-commit-subject ${activeCommit === commit.sha ? "sidebar-commit-active" : ""}`.trim()}
                type="button"
                onClick={() => onSelectCommit(commit.sha)}
              >
                {commit.subject}
              </button>
              <span className="sidebar-commit-sha">{commit.short_sha}</span>
            </div>
            <p className="sidebar-commit-meta">
              <span>{commit.author}</span>
              <span className={`sidebar-commit-review-status ${commit.review_status}`.trim()}>
                {commit.review_status}
              </span>
            </p>
          </div>
        ))}
      </div>
    </section>
  );
}

export function LeftSidebar({
  data,
  snoozedFiles,
  onJumpToFile,
  onJumpToComment,
  onShowAll,
  activeFilePath,
  onStageWholeFile,
}: LeftSidebarProps) {
  const {
    state: { activeView, busy },
    actions,
  } = useReviewStore();
  const isViewingAll = activeView === ReviewView.All;
  const isCommitReview = Boolean(data.active_commit);
  const sidebarFiles = useMemo(() => buildSidebarFiles(data, snoozedFiles), [data, snoozedFiles]);
  const sidebarComments = useMemo(() => buildSidebarComments(data), [data]);

  return (
    <aside className="left-sidebar">
      <section className="sidebar-section sidebar-view-section">
        <div className="sidebar-list">
          <div className="sidebar-link">
            <button
              className={`sidebar-link-action ${isViewingAll ? "sidebar-link-active" : ""}`.trim()}
              type="button"
              onClick={() => {
                window.scrollTo({ top: 0, left: 0, behavior: "auto" });
                onShowAll();
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
        reviewMode={isCommitReview}
        onJumpToFile={onJumpToFile}
        onToggleFileStage={(file) => {
          const shouldUnstage = file.status === FILE_STAGE_STATUS.staged;
          if (!shouldUnstage) {
            onStageWholeFile?.(file);
          }
          void actions.toggleStageFile(file.filePath, shouldUnstage);
        }}
        onToggleFileReviewed={(file) => {
          void actions.setFileReviewed(file.filePath, !file.reviewed);
        }}
      />
      <SidebarCommitsSection
        activeCommit={data.active_commit}
        base={data.commit_base}
        commits={data.commits}
        localSummary={localChangesSummary(sidebarFiles)}
        onSelectCommit={(commit) => {
          window.scrollTo({ top: 0, left: 0, behavior: "auto" });
          void actions.setActiveCommit(commit);
        }}
      />
      <SidebarCommentsSection comments={sidebarComments} onJumpToComment={onJumpToComment} />
      <SidebarShortcutsHint />
    </aside>
  );
}
