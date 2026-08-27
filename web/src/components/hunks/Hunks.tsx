import { useEffect, useMemo, useRef, useState } from "react";
import { ReviewView, useReviewStore } from "../../reviewStore";
import type { AgentKind, AgentOption, Hunk } from "../../types";
import { EMPTY_LINE_DIFF_STATS, lineDiffReducer } from "../diffStats";
import { HunkCard } from "./HunkCard";
import { useReviewScroll } from "../workspace/reviewScroll";

type HunksProps = {
  hunks: Hunk[];
  agents: AgentOption[];
  selectedAgent: AgentKind;
  onAgentChange: (agent: AgentKind) => void;
  onSnoozeFile: (filePath: string) => void;
  onJumpToHunk: (target: { filePath: string; hunkId: string; elementId: string }) => void;
  selectedFilePath?: string | null;
  targetFilePath?: string | null;
  targetHunkId?: string | null;
  /// Several reviews can be on screen at once; only the one in front takes the shortcuts.
  shortcutsEnabled: boolean;
};

type FileGroup = {
  filePath: string;
  hunks: Hunk[];
};

function groupByFile(hunks: Hunk[]): FileGroup[] {
  const grouped = new Map<string, Hunk[]>();
  for (const hunk of hunks) {
    const existing = grouped.get(hunk.file_path) ?? [];
    existing.push(hunk);
    grouped.set(hunk.file_path, existing);
  }

  return [...grouped.entries()].map(([filePath, fileHunks]) => ({
    filePath,
    hunks: fileHunks,
  }));
}

function flattenGroups(groups: FileGroup[]): Hunk[] {
  return groups.flatMap((group) => group.hunks);
}

function isEditableShortcutTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  const tagName = target.tagName.toLowerCase();
  return tagName === "input" || tagName === "textarea" || tagName === "select" || target.isContentEditable;
}

/// The hunk the keyboard shortcuts act on is the one nearest this line in the review pane.
function hunkAnchorTop(paneTop: number, paneHeight: number) {
  return paneTop + Math.min(180, paneHeight * 0.28);
}

function useActiveHunkFromViewport(interactiveHunks: Hunk[]) {
  const { actions } = useReviewStore();
  const { scrollElement } = useReviewScroll();

  useEffect(() => {
    if (interactiveHunks.length === 0) {
      actions.setActiveHunkId(null);
      return;
    }

    const pane = scrollElement();
    if (!pane) {
      return;
    }

    let animationFrame = 0;

    function updateActiveHunkFromViewport() {
      window.cancelAnimationFrame(animationFrame);
      animationFrame = window.requestAnimationFrame(() => {
        let nextHunkId: string | null = null;
        let bestDistance = Number.POSITIVE_INFINITY;
        const paneRect = pane!.getBoundingClientRect();
        const anchorTop = hunkAnchorTop(paneRect.top, paneRect.height);

        for (const hunk of interactiveHunks) {
          const element = document.getElementById(`hunk-${hunk.id}`);
          if (!element) {
            continue;
          }

          const rect = element.getBoundingClientRect();
          if (rect.bottom < paneRect.top || rect.top > paneRect.bottom) {
            continue;
          }

          const distance = Math.abs(rect.top - anchorTop);
          if (distance < bestDistance) {
            bestDistance = distance;
            nextHunkId = hunk.id;
          }
        }

        actions.setActiveHunkId(nextHunkId ?? interactiveHunks[0]?.id ?? null);
      });
    }

    updateActiveHunkFromViewport();
    pane.addEventListener("scroll", updateActiveHunkFromViewport, { passive: true });
    window.addEventListener("resize", updateActiveHunkFromViewport);
    return () => {
      window.cancelAnimationFrame(animationFrame);
      pane.removeEventListener("scroll", updateActiveHunkFromViewport);
      window.removeEventListener("resize", updateActiveHunkFromViewport);
    };
  }, [actions, interactiveHunks, scrollElement]);
}

function FileAccordion({
  filePath,
  hunks,
  agents,
  selectedAgent,
  onAgentChange,
  onSnoozeFile,
  onJumpToHunk,
}: {
  filePath: string;
  hunks: Hunk[];
  agents: AgentOption[];
  selectedAgent: AgentKind;
  onAgentChange: (agent: AgentKind) => void;
  onSnoozeFile: (filePath: string) => void;
  onJumpToHunk: (target: { filePath: string; hunkId: string; elementId: string }) => void;
}) {
  const {
    state: { activeView, data },
    actions,
  } = useReviewStore();
  const canStageWholeFile = activeView !== ReviewView.All;
  const staged = hunks.every((hunk) => hunk.staged);
  const status = staged ? "Staged" : "Unstaged";
  const diffStats = hunks.reduce(lineDiffReducer, EMPTY_LINE_DIFF_STATS);
  const readOnly = data?.read_only ?? false;
  const isCommitReview = Boolean(data?.active_commit);

  return (
    <div id={`file-${encodeURIComponent(filePath)}`} className="file-accordion">
      <div className="file-accordion-head">
        <div className="file-accordion-toggle">
          <span>{filePath}</span>
        </div>
        <span className="file-accordion-meta">
          <span className="diff-stats-summary">
            <span className="diff-stat diff-stat-added">++{diffStats.added}</span>
            <span className="diff-stat diff-stat-removed">--{diffStats.removed}</span>
          </span>
          {readOnly ? null : (
            <span className={`badge ${staged ? "staged" : "unstaged"}`.trim()}>{status}</span>
          )}
          <span className="muted">{hunks.length}</span>
          {canStageWholeFile && !readOnly && !staged ? (
            <button type="button" onClick={() => void actions.toggleStageFile(filePath, staged)}>
              {staged ? "Unstage File" : "Stage File"}
            </button>
          ) : null}
        </span>
      </div>
      <div className="collapsible-content">
        {hunks.map((hunk) => (
          <HunkCard
            key={hunk.id}
            hunk={hunk}
            agents={agents}
            selectedAgent={selectedAgent}
            onAgentChange={onAgentChange}
            onJumpToHunk={onJumpToHunk}
          />
        ))}
        {!readOnly ? (
          <div className="file-accordion-footer">
            <span className="file-accordion-meta file-accordion-meta-footer">
              {canStageWholeFile && !staged ? (
                <button type="button" onClick={() => onSnoozeFile(filePath)}>
                  Snooze
                </button>
              ) : null}
              {canStageWholeFile ? (
                <button type="button" onClick={() => void actions.toggleStageFile(filePath, staged)}>
                  {staged ? "Unstage File" : "Stage File"}
                </button>
              ) : null}
            </span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function Hunks({
  hunks,
  agents,
  selectedAgent,
  onAgentChange,
  onSnoozeFile,
  onJumpToHunk,
  selectedFilePath,
  targetFilePath,
  targetHunkId,
  shortcutsEnabled,
}: HunksProps) {
  const {
    state: { activeHunkId, activeView, busy, data },
    actions,
  } = useReviewStore();
  const { scrollToTop } = useReviewScroll();
  const isViewingAll = activeView === ReviewView.All;
  const readOnly = data?.read_only ?? false;
  const isCommitReview = Boolean(data?.active_commit);
  const [unstagedOpen, setUnstagedOpen] = useState(true);
  const unstagedGroups = useMemo(() => groupByFile(hunks.filter((hunk) => !hunk.staged)), [hunks]);
  const stagedGroups = useMemo(() => groupByFile(hunks.filter((hunk) => hunk.staged)), [hunks]);
  // A commit review has no index behind it, so its hunks are one list rather than a staged
  // and an unstaged half.
  const allGroups = useMemo(() => groupByFile(hunks), [hunks]);
  const [stagedOpen, setStagedOpen] = useState(
    () => stagedGroups.length > 0 && unstagedGroups.length === 0,
  );
  const stagedDefaultKey = useRef<string | null>(null);
  const firstSectionTitle = isCommitReview ? "Changed" : "Unstaged";
  const secondSectionTitle = "Staged";

  useEffect(() => {
    if (isCommitReview) {
      setUnstagedOpen(true);
    }
  }, [isCommitReview, data?.active_commit]);

  const activeFilePath = useMemo(() => {
    if (selectedFilePath && hunks.some((hunk) => hunk.file_path === selectedFilePath)) {
      return selectedFilePath;
    }
    return unstagedGroups[0]?.filePath ?? stagedGroups[0]?.filePath ?? null;
  }, [hunks, selectedFilePath, stagedGroups, unstagedGroups]);
  const emptyFirstSectionText = isCommitReview
    ? "No changes in this commit."
    : activeFilePath
      ? `No unstaged hunks in ${activeFilePath}.`
      : "No unstaged hunks.";
  const emptySecondSectionText = "No staged hunks.";
  const visibleUnstagedGroups = useMemo(
    () => {
      if (isViewingAll) {
        return isCommitReview ? allGroups : unstagedGroups;
      }
      if (isCommitReview) {
        return allGroups.filter((group) => group.filePath === activeFilePath);
      }
      return unstagedGroups.filter((group) => group.filePath === activeFilePath);
    },
    [activeFilePath, allGroups, isCommitReview, isViewingAll, unstagedGroups],
  );
  const visibleStagedGroups = useMemo(
    () => {
      if (isCommitReview) {
        return [];
      }
      if (isViewingAll) {
        return stagedGroups;
      }
      return stagedGroups.filter((group) => group.filePath === activeFilePath);
    },
    [activeFilePath, isCommitReview, isViewingAll, stagedGroups],
  );
  const hasVisibleUnstagedGroups = visibleUnstagedGroups.length > 0;
  const hasVisibleStagedGroups = visibleStagedGroups.length > 0;

  useEffect(() => {
    if (isCommitReview) {
      return;
    }

    const nextDefaultKey = [
      isViewingAll ? "all" : activeFilePath ?? "",
      hasVisibleUnstagedGroups ? "has-unstaged" : "no-unstaged",
      hasVisibleStagedGroups ? "has-staged" : "no-staged",
    ].join(":");
    if (stagedDefaultKey.current === nextDefaultKey) {
      return;
    }
    stagedDefaultKey.current = nextDefaultKey;

    if (hasVisibleUnstagedGroups && hasVisibleStagedGroups) {
      setStagedOpen(false);
    }
  }, [activeFilePath, hasVisibleStagedGroups, hasVisibleUnstagedGroups, isCommitReview, isViewingAll]);

  useEffect(() => {
    if (!activeFilePath || isViewingAll) {
      return;
    }

    if (visibleUnstagedGroups.length === 0 && visibleStagedGroups.length > 0) {
      setStagedOpen(true);
    }
  }, [activeFilePath, isViewingAll, visibleStagedGroups.length, visibleUnstagedGroups.length]);

  const visibleUnstagedHunks = useMemo(() => flattenGroups(visibleUnstagedGroups), [visibleUnstagedGroups]);
  const visibleStagedHunks = useMemo(() => flattenGroups(visibleStagedGroups), [visibleStagedGroups]);
  const interactiveUnstagedHunks = useMemo(
    () => (unstagedOpen ? visibleUnstagedHunks : []),
    [unstagedOpen, visibleUnstagedHunks],
  );
  const interactiveStagedHunks = useMemo(
    () => (stagedOpen ? visibleStagedHunks : []),
    [stagedOpen, visibleStagedHunks],
  );
  const interactiveHunks = useMemo(
    () => [...interactiveUnstagedHunks, ...interactiveStagedHunks],
    [interactiveStagedHunks, interactiveUnstagedHunks],
  );
  const activeHunk = useMemo(
    () => hunks.find((hunk) => hunk.id === activeHunkId) ?? null,
    [activeHunkId, hunks],
  );
  const hunkTargets = useMemo(
    () =>
      new Map(
        hunks.map((hunk) => [
          hunk.id,
          {
            filePath: hunk.file_path,
            staged: hunk.staged,
          },
        ]),
      ),
    [hunks],
  );

  useEffect(() => {
    const target = targetHunkId ? hunkTargets.get(targetHunkId) : null;
    const nextFilePath = target?.filePath ?? targetFilePath;
    if (!nextFilePath) {
      return;
    }

    if (target) {
      if (isCommitReview) {
        setUnstagedOpen(true);
      } else if (target.staged) {
        setStagedOpen(true);
      } else {
        setUnstagedOpen(true);
      }
      return;
    }

    const fileHunks = hunks.filter((hunk) => hunk.file_path === nextFilePath);
    if (fileHunks.some((hunk) => !hunk.staged)) {
      setUnstagedOpen(true);
    }
  }, [hunkTargets, hunks, isCommitReview, targetFilePath, targetHunkId]);

  useActiveHunkFromViewport(interactiveHunks);

  useEffect(() => {
    if (!shortcutsEnabled) {
      return;
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (isEditableShortcutTarget(event.target)) {
        return;
      }

      const key = event.key.toLowerCase();
      if (event.metaKey || event.ctrlKey || event.altKey || busy || !activeHunk) {
        return;
      }

      if (!readOnly && key === "s" && !activeHunk.staged) {
        event.preventDefault();
        const shouldScrollToTop = interactiveUnstagedHunks[0]?.id === activeHunk.id;
        void actions.toggleStage(activeHunk.id, activeHunk.staged).then((changed) => {
          if (changed && shouldScrollToTop) {
            scrollToTop();
          }
        });
      } else if (!readOnly && key === "u" && activeHunk.staged) {
        event.preventDefault();
        void actions.toggleStage(activeHunk.id, activeHunk.staged);
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [actions, activeHunk, busy, interactiveUnstagedHunks, isCommitReview, readOnly, shortcutsEnabled]);

  return (
    <div className="hunk-sections">
      <section className="panel panel-plain hunk-section">
        <button className="hunk-section-toggle hunk-section-toggle-large" onClick={() => setUnstagedOpen((open) => !open)}>
          <h2>{firstSectionTitle}</h2>
          <span className="muted">{visibleUnstagedGroups.reduce((sum, group) => sum + group.hunks.length, 0)}</span>
        </button>
        <div className={`collapsible-content ${unstagedOpen ? "" : "collapsible-content-collapsed"}`.trim()}>
          {visibleUnstagedGroups.length > 0 ? (
            visibleUnstagedGroups.map((group) => (
              <FileAccordion
                key={group.filePath}
                filePath={group.filePath}
                hunks={group.hunks}
                agents={agents}
                selectedAgent={selectedAgent}
                onAgentChange={onAgentChange}
                onSnoozeFile={onSnoozeFile}
                onJumpToHunk={onJumpToHunk}
              />
            ))
          ) : (
            <div className="empty-section muted">{emptyFirstSectionText}</div>
          )}
        </div>
      </section>

      {isCommitReview ? null : (
        <section className="panel panel-plain hunk-section">
          <button className="hunk-section-toggle hunk-section-toggle-large" onClick={() => setStagedOpen((open) => !open)}>
            <h2>{secondSectionTitle}</h2>
            <span className="muted">{visibleStagedGroups.reduce((sum, group) => sum + group.hunks.length, 0)}</span>
          </button>
          <div className={`collapsible-content ${stagedOpen ? "" : "collapsible-content-collapsed"}`.trim()}>
            {visibleStagedGroups.length > 0 ? (
              visibleStagedGroups.map((group) => (
                <FileAccordion
                  key={group.filePath}
                  filePath={group.filePath}
                  hunks={group.hunks}
                  agents={agents}
                  selectedAgent={selectedAgent}
                  onAgentChange={onAgentChange}
                  onSnoozeFile={onSnoozeFile}
                  onJumpToHunk={onJumpToHunk}
                />
              ))
            ) : (
              <div className="empty-section muted">{emptySecondSectionText}</div>
            )}
          </div>
        </section>
      )}
    </div>
  );
}
