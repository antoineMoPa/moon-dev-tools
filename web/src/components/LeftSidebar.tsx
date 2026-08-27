import { useEffect, useMemo, useState } from "react";
import { fetchCommitHistory } from "../api";
import { ReviewView, useReviewStore } from "../reviewStore";
import { COMMENT_DISPATCH_STATUS } from "../types";
import type { CommentDispatchStatus, Commit, ReviewComment, SessionState } from "../types";
import { SidebarCommentsSection } from "./SidebarCommentsSection";
import { SidebarFilesSection } from "./SidebarFilesSection";
import { buildSidebarFiles, FILE_STAGE_STATUS, localChangesSummary } from "./sidebarFiles";
import { useReviewScroll } from "./workspace/reviewScroll";
import type { SidebarFileItem } from "./sidebarFiles";

const HISTORY_COMMIT_PAGE_SIZE = 30;

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
  return data.review_comments.map((comment, index) => buildSidebarCommentItem(comment, index));
}

function buildSidebarCommentItem(comment: ReviewComment, index: number): SidebarCommentItem {
  return {
    id: `${comment.hunk_id}:${comment.comment_index}:${index}`,
    hunkId: comment.hunk_id,
    commentAnchorId: comment.jumpable ? `comment-${comment.hunk_id}-${comment.comment_index}` : null,
    filePath: comment.file_path,
    comment: comment.comment,
    selection: comment.selection,
    resolved: comment.resolved,
    statusLabel: statusLabel(comment.resolved, comment.dispatch.status),
  };
}

function SidebarShortcutsHint() {
  const {
    state: { activeHunkId, data },
  } = useReviewStore();
  const activeHunk = data?.hunks.find((hunk) => hunk.id === activeHunkId) ?? null;

  // Staging is the only thing the keys do, and a review with no index behind it - a commit,
  // a comparison - cannot stage.
  if (!activeHunk || data?.read_only) {
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

function SidebarCommitsSection({
  activeCommit,
  base,
  commits,
  historyCommits,
  historyHasMore,
  localSummary,
  onSelectCommit,
}: {
  activeCommit?: string | null;
  base?: string | null;
  commits: Commit[];
  historyCommits: Commit[];
  historyHasMore: boolean;
  localSummary: string;
  onSelectCommit: (commit: string | null) => void;
}) {
  const {
    state: { sessionId },
  } = useReviewStore();
  const [loadedHistoryCommits, setLoadedHistoryCommits] = useState<Commit[]>(historyCommits);
  const [historyHasMorePages, setHistoryHasMorePages] = useState(historyHasMore);
  const [loadingHistory, setLoadingHistory] = useState(false);

  useEffect(() => {
    setLoadedHistoryCommits(historyCommits);
    setHistoryHasMorePages(historyHasMore);
  }, [historyCommits, historyHasMore]);

  function renderCommit(commit: Commit) {
    return (
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
        </p>
      </div>
    );
  }

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
        ) : commits.map(renderCommit)}
        {loadedHistoryCommits.length > 0 ? (
          <div className="sidebar-commit-history">
            <div className="sidebar-commit-history-head">
              <p>History</p>
              <span>{loadedHistoryCommits.length}</span>
            </div>
            {loadedHistoryCommits.map(renderCommit)}
            {historyHasMorePages ? (
              <button
                className="sidebar-commit-history-more"
                type="button"
                disabled={loadingHistory}
                onClick={() => {
                  setLoadingHistory(true);
                  void fetchCommitHistory(sessionId, loadedHistoryCommits.length, HISTORY_COMMIT_PAGE_SIZE)
                    .then((page) => {
                      setLoadedHistoryCommits((current) => [...current, ...page.commits]);
                      setHistoryHasMorePages(page.has_more);
                    })
                    .finally(() => setLoadingHistory(false));
                }}
              >
                {loadingHistory ? "Loading..." : "Show more"}
              </button>
            ) : null}
          </div>
        ) : null}
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
  const { scrollToTop } = useReviewScroll();
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
                scrollToTop();
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
        onJumpToFile={onJumpToFile}
        onToggleFileStage={(file) => {
          const shouldUnstage = file.status === FILE_STAGE_STATUS.staged;
          if (!shouldUnstage) {
            onStageWholeFile?.(file);
          }
          void actions.toggleStageFile(file.filePath, shouldUnstage);
        }}
      />
      <SidebarCommitsSection
        activeCommit={data.active_commit}
        base={data.commit_base}
        commits={data.commits}
        historyCommits={data.history_commits}
        historyHasMore={data.history_has_more}
        localSummary={localChangesSummary(data.local_change_summary)}
        onSelectCommit={(commit) => {
          scrollToTop();
          void actions.setActiveCommit(commit);
        }}
      />
      <SidebarCommentsSection comments={sidebarComments} onJumpToComment={onJumpToComment} />
      <SidebarShortcutsHint />
    </aside>
  );
}
