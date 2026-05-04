import { useEffect, useMemo, useState } from "react";
import { useReviewStore } from "../../reviewStore";
import type { AgentKind, AgentOption, Hunk } from "../../types";
import { EMPTY_LINE_DIFF_STATS, lineDiffReducer } from "../diffStats";
import { HunkCard } from "./HunkCard";

type HunksProps = {
  hunks: Hunk[];
  agents: AgentOption[];
  selectedAgent: AgentKind;
  onAgentChange: (agent: AgentKind) => void;
  onSnoozeFile: (filePath: string) => void;
  selectedFilePath?: string | null;
  targetFilePath?: string | null;
  targetHunkId?: string | null;
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

function FileAccordion({
  filePath,
  hunks,
  agents,
  selectedAgent,
  onAgentChange,
  onSnoozeFile,
}: {
  filePath: string;
  hunks: Hunk[];
  agents: AgentOption[];
  selectedAgent: AgentKind;
  onAgentChange: (agent: AgentKind) => void;
  onSnoozeFile: (filePath: string) => void;
}) {
  const {
    state: { data },
    actions,
  } = useReviewStore();
  const staged = hunks.every((hunk) => hunk.staged);
  const status = staged ? "Staged" : "Unstaged";
  const diffStats = hunks.reduce(lineDiffReducer, EMPTY_LINE_DIFF_STATS);
  const readOnly = data?.read_only ?? false;

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
          <span className={`badge ${staged ? "staged" : "unstaged"}`.trim()}>{status}</span>
          <span className="muted">{hunks.length}</span>
          {!readOnly && !staged ? (
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
          />
        ))}
        {!readOnly ? (
          <div className="file-accordion-footer">
            <span className="file-accordion-meta file-accordion-meta-footer">
              {!staged ? (
                <button type="button" onClick={() => onSnoozeFile(filePath)}>
                  Snooze
                </button>
              ) : null}
              <button type="button" onClick={() => void actions.toggleStageFile(filePath, staged)}>
                {staged ? "Unstage File" : "Stage File"}
              </button>
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
  selectedFilePath,
  targetFilePath,
  targetHunkId,
}: HunksProps) {
  const [unstagedOpen, setUnstagedOpen] = useState(true);
  const unstagedGroups = useMemo(() => groupByFile(hunks.filter((hunk) => !hunk.staged)), [hunks]);
  const stagedGroups = useMemo(() => groupByFile(hunks.filter((hunk) => hunk.staged)), [hunks]);
  const [stagedOpen, setStagedOpen] = useState(
    () => stagedGroups.length > 0 && unstagedGroups.length === 0,
  );
  const activeFilePath = useMemo(() => {
    if (selectedFilePath && hunks.some((hunk) => hunk.file_path === selectedFilePath)) {
      return selectedFilePath;
    }
    return unstagedGroups[0]?.filePath ?? stagedGroups[0]?.filePath ?? null;
  }, [hunks, selectedFilePath, stagedGroups, unstagedGroups]);
  const visibleUnstagedGroups = useMemo(
    () => unstagedGroups.filter((group) => group.filePath === activeFilePath),
    [activeFilePath, unstagedGroups],
  );
  const visibleStagedGroups = useMemo(
    () => stagedGroups.filter((group) => group.filePath === activeFilePath),
    [activeFilePath, stagedGroups],
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
      if (target.staged) {
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
  }, [hunkTargets, hunks, targetFilePath, targetHunkId]);

  return (
    <div className="hunk-sections">
      <section className="panel panel-plain hunk-section">
        <button className="hunk-section-toggle hunk-section-toggle-large" onClick={() => setUnstagedOpen((open) => !open)}>
          <h2>Unstaged</h2>
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
              />
            ))
          ) : (
            <div className="empty-section muted">
              {activeFilePath ? `No unstaged hunks in ${activeFilePath}.` : "No unstaged hunks."}
            </div>
          )}
        </div>
      </section>

      <section className="panel panel-plain hunk-section">
        <button className="hunk-section-toggle hunk-section-toggle-large" onClick={() => setStagedOpen((open) => !open)}>
          <h2>Staged</h2>
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
              />
            ))
          ) : (
            <div className="empty-section muted">No staged hunks.</div>
          )}
        </div>
      </section>
    </div>
  );
}
