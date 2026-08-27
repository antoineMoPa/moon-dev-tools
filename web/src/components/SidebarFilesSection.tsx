import { createContext, useContext, type ReactNode } from "react";
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
};

type SidebarFilesContextValue = {
  activeFilePath?: string | null;
  readOnly: boolean;
  busy: boolean;
  onJumpToFile: (filePath: string) => void;
  onToggleFileStage: (file: SidebarFileItem) => void;
};

const SidebarFilesContext = createContext<SidebarFilesContextValue | null>(null);

function useSidebarFilesContext() {
  const context = useContext(SidebarFilesContext);
  if (!context) {
    throw new Error("Sidebar file controls must be rendered inside SidebarFilesSection.");
  }
  return context;
}

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

function SidebarFileButton({ file }: SidebarFileButtonProps) {
  const { activeFilePath, readOnly, busy, onJumpToFile, onToggleFileStage } =
    useSidebarFilesContext();
  const active = file.filePath === activeFilePath;
  const statusClass = file.status;

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
      {readOnly ? null : (
        <span className="sidebar-link-meta">
          <button
            className={`badge sidebar-file-status sidebar-file-status-${statusClass}`.trim()}
            type="button"
            title="toggle file stage"
            disabled={busy}
            onClick={(event) => {
              event.stopPropagation();
              onToggleFileStage(file);
            }}
          >
            {statusLabel(file)}
          </button>
        </span>
      )}
    </div>
  );
}

type SidebarFileGroupProps = {
  title: string;
  files: SidebarFileItem[];
  addedCount: number;
  removedCount: number;
};

export function orderMovedFilesAdjacently(files: SidebarFileItem[]) {
  const remaining = new Set(files.map((file) => file.filePath));
  const fileByPath = new Map(files.map((file) => [file.filePath, file]));
  const ordered: SidebarFileItem[] = [];

  for (const file of files) {
    if (!remaining.has(file.filePath) || file.movedFromFilePath) {
      continue;
    }

    ordered.push(file);
    remaining.delete(file.filePath);

    if (file.movedToFilePath && remaining.has(file.movedToFilePath)) {
      const targetFile = fileByPath.get(file.movedToFilePath);
      if (targetFile) {
        ordered.push(targetFile);
        remaining.delete(targetFile.filePath);
      }
    }
  }

  for (const file of files) {
    if (remaining.has(file.filePath)) {
      ordered.push(file);
    }
  }

  return ordered;
}

function showsMovedMarker(file: SidebarFileItem, previousFile?: SidebarFileItem) {
  return (
    file.changeKind === "added" &&
    previousFile?.changeKind === "deleted" &&
    file.movedFromFilePath === previousFile.filePath &&
    previousFile.movedToFilePath === file.filePath
  );
}

function SidebarFileGroup({
  title,
  files,
  addedCount,
  removedCount,
}: SidebarFileGroupProps) {
  if (files.length === 0) {
    return null;
  }

  const orderedFiles = orderMovedFilesAdjacently(files);

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
        {orderedFiles.map((file, index) => (
          <div className="sidebar-file-row-wrap" key={file.filePath}>
            {showsMovedMarker(file, orderedFiles[index - 1]) ? (
              <span className="sidebar-link-moved-marker" aria-hidden="true">↓</span>
            ) : null}
            <SidebarFileButton file={file} />
          </div>
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

  const controls = {
    activeFilePath,
    readOnly,
    busy,
    onJumpToFile,
    onToggleFileStage,
  };

  if (readOnly) {
    return (
      <SidebarFilesContext.Provider value={controls}>
        <SidebarSection
          title="Files"
          addedCount={totalDiffStats.added}
          removedCount={totalDiffStats.removed}
        >
          <SidebarFileGroup
            title="Changed"
            files={files}
            addedCount={totalDiffStats.added}
            removedCount={totalDiffStats.removed}
          />
        </SidebarSection>
      </SidebarFilesContext.Provider>
    );
  }

  return (
    <SidebarFilesContext.Provider value={controls}>
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
        />
        <SidebarFileGroup
          title="Staged"
          files={stagedFiles}
          addedCount={stagedDiffStats.added}
          removedCount={stagedDiffStats.removed}
        />
        <SidebarFileGroup
          title="Snoozed"
          files={snoozedFiles}
          addedCount={snoozedDiffStats.added}
          removedCount={snoozedDiffStats.removed}
        />
      </SidebarSection>
    </SidebarFilesContext.Provider>
  );
}
