import type { ReactNode } from "react";
import { EMPTY_LINE_DIFF_STATS } from "./diffStats";
import type { LineDiffStats } from "./diffStats";
import type { SidebarFileItem } from "./sidebarFiles";

type SidebarSectionProps = {
  title: string;
  addedCount?: number;
  removedCount?: number;
  children: ReactNode;
};

function SidebarSection({ title, addedCount, removedCount, children }: SidebarSectionProps) {
  return (
    <section className="sidebar-section">
      <div className="sidebar-section-head">
        <p>{title}</p>
        {typeof addedCount === "number" && typeof removedCount === "number" ? (
          <div className="diff-stats-summary" aria-label="Diff stats">
            <span className="diff-stat diff-stat-added">++{addedCount}</span>
            <span className="diff-stat diff-stat-removed">--{removedCount}</span>
          </div>
        ) : null}
      </div>
      <div className="sidebar-list">{children}</div>
    </section>
  );
}

type SidebarFileButtonProps = {
  file: SidebarFileItem;
  active: boolean;
  readOnly: boolean;
  busy: boolean;
  onJumpToFile: (filePath: string) => void;
  onToggleFileStage: (file: SidebarFileItem) => void;
};

function statusLabel(file: SidebarFileItem) {
  if (file.status === "partial") {
    return "Partial";
  }
  return file.status === "staged" ? "Staged" : "Unstaged";
}

function filePrefix(file: SidebarFileItem) {
  if (file.changeKind === "added") {
    return "+";
  }
  if (file.changeKind === "deleted") {
    return "-";
  }
  return "";
}

function SidebarFileButton({
  file,
  active,
  readOnly,
  busy,
  onJumpToFile,
  onToggleFileStage,
}: SidebarFileButtonProps) {
  return (
    <div className="sidebar-link" title={file.filePath}>
      <button
        className={`sidebar-link-action ${active ? "sidebar-link-active" : ""}`.trim()}
        type="button"
        onClick={() => onJumpToFile(file.filePath)}
      >
        <span
          className={[
            "sidebar-link-name",
            `sidebar-link-name-${file.changeKind}`,
            file.snoozed ? "sidebar-link-name-snoozed" : "",
          ]
            .filter(Boolean)
            .join(" ")}
        >
          <span className={`sidebar-link-prefix sidebar-link-prefix-${file.changeKind}`.trim()}>
            {filePrefix(file)}
          </span>
          {file.fileName}
        </span>
      </button>
      <span className="sidebar-link-meta">
        <button
          className={`badge sidebar-file-status sidebar-file-status-${file.status}`.trim()}
          type="button"
          title="toggle file stage"
          disabled={readOnly || busy}
          onClick={(event) => {
            event.stopPropagation();
            onToggleFileStage(file);
          }}
        >
          {statusLabel(file)}
        </button>
      </span>
    </div>
  );
}

type SidebarFileGroupProps = {
  title: string;
  files: SidebarFileItem[];
  addedCount: number;
  removedCount: number;
  activeFilePath?: string | null;
  readOnly: boolean;
  busy: boolean;
  onJumpToFile: (filePath: string) => void;
  onToggleFileStage: (file: SidebarFileItem) => void;
};

function SidebarFileGroup({
  title,
  files,
  addedCount,
  removedCount,
  activeFilePath,
  readOnly,
  busy,
  onJumpToFile,
  onToggleFileStage,
}: SidebarFileGroupProps) {
  if (files.length === 0) {
    return null;
  }

  return (
    <div className="sidebar-file-group">
      <div className="sidebar-file-group-head">
        <p className="sidebar-file-group-title">{title}</p>
        <div className="diff-stats-summary diff-stats-summary-muted" aria-label={`${title} diff stats`}>
          <span className="diff-stat diff-stat-normal diff-stat-muted diff-stat-added">++{addedCount}</span>
          <span className="diff-stat diff-stat-normal diff-stat-muted diff-stat-removed">--{removedCount}</span>
        </div>
      </div>
      <div className="sidebar-list">
        {files.map((file) => (
          <SidebarFileButton
            key={file.filePath}
            file={file}
            active={file.filePath === activeFilePath}
            readOnly={readOnly}
            busy={busy}
            onJumpToFile={onJumpToFile}
            onToggleFileStage={onToggleFileStage}
          />
        ))}
      </div>
    </div>
  );
}

function unstagedLineDiffReducer(sum: LineDiffStats, file: SidebarFileItem): LineDiffStats {
  return {
    added: sum.added + file.unstaged_added_line_count,
    removed: sum.removed + file.unstaged_removed_line_count,
  };
}

function stagedLineDiffReducer(sum: LineDiffStats, file: SidebarFileItem): LineDiffStats {
  return {
    added: sum.added + file.staged_added_line_count,
    removed: sum.removed + file.staged_removed_line_count,
  };
}

function totalLineDiffReducer(sum: LineDiffStats, file: SidebarFileItem): LineDiffStats {
  return {
    added: sum.added + file.added_line_count,
    removed: sum.removed + file.removed_line_count,
  };
}

type SidebarFilesSectionProps = {
  files: SidebarFileItem[];
  activeFilePath?: string | null;
  readOnly: boolean;
  busy: boolean;
  onJumpToFile: (filePath: string) => void;
  onToggleFileStage: (file: SidebarFileItem) => void;
};

export function buildSidebarFileGroups(files: SidebarFileItem[]) {
  const unstagedFiles = files.filter((file) => file.status !== "staged" && !file.snoozed);
  const stagedFiles = files.filter((file) => file.status === "staged");
  const snoozedFiles = files.filter((file) => file.snoozed);
  const unstagedDiffStats = unstagedFiles.reduce(unstagedLineDiffReducer, EMPTY_LINE_DIFF_STATS);
  const stagedDiffStats = stagedFiles.reduce(stagedLineDiffReducer, EMPTY_LINE_DIFF_STATS);
  const snoozedDiffStats = snoozedFiles.reduce(unstagedLineDiffReducer, EMPTY_LINE_DIFF_STATS);
  const totalDiffStats = files.reduce(totalLineDiffReducer, EMPTY_LINE_DIFF_STATS);
  const remainingUnstagedDiffStats = [...unstagedFiles, ...snoozedFiles].reduce(
    unstagedLineDiffReducer,
    EMPTY_LINE_DIFF_STATS,
  );

  return {
    unstagedFiles,
    stagedFiles,
    snoozedFiles,
    unstagedDiffStats,
    stagedDiffStats,
    snoozedDiffStats,
    totalDiffStats,
    remainingUnstagedDiffStats,
  };
}

export function SidebarFilesSection({
  files,
  activeFilePath,
  readOnly,
  busy,
  onJumpToFile,
  onToggleFileStage,
}: SidebarFilesSectionProps) {
  const {
    unstagedFiles,
    stagedFiles,
    snoozedFiles,
    unstagedDiffStats,
    stagedDiffStats,
    snoozedDiffStats,
    totalDiffStats,
  } = buildSidebarFileGroups(files);

  return (
    <SidebarSection
      title="Files"
      addedCount={totalDiffStats.added}
      removedCount={totalDiffStats.removed}
    >
      <SidebarFileGroup
        title="Unstaged"
        files={unstagedFiles}
        addedCount={unstagedDiffStats.added}
        removedCount={unstagedDiffStats.removed}
        activeFilePath={activeFilePath}
        readOnly={readOnly}
        busy={busy}
        onJumpToFile={onJumpToFile}
        onToggleFileStage={onToggleFileStage}
      />
      <SidebarFileGroup
        title="Staged"
        files={stagedFiles}
        addedCount={stagedDiffStats.added}
        removedCount={stagedDiffStats.removed}
        activeFilePath={activeFilePath}
        readOnly={readOnly}
        busy={busy}
        onJumpToFile={onJumpToFile}
        onToggleFileStage={onToggleFileStage}
      />
      <SidebarFileGroup
        title="Snoozed"
        files={snoozedFiles}
        addedCount={snoozedDiffStats.added}
        removedCount={snoozedDiffStats.removed}
        activeFilePath={activeFilePath}
        readOnly={readOnly}
        busy={busy}
        onJumpToFile={onJumpToFile}
        onToggleFileStage={onToggleFileStage}
      />
    </SidebarSection>
  );
}
