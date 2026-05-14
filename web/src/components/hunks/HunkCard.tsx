import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import hljs from "highlight.js/lib/core";
import diff from "highlight.js/lib/languages/diff";
import { buildFullFileUrl, fetchHunkPatch } from "../../api";
import {
  buildAnchoredCommentValue,
  parseAnchoredComments,
  type AnchoredComment,
} from "../../anchoredComments";
import { useReviewStore } from "../../reviewStore";
import type { AgentKind, AgentOption, Hunk } from "../../types";
import { splitDiffIntoSegments } from "./diffSegments";
import { HunkCommentContextProvider } from "./HunkCommentContext";
import { InlineCommentCard } from "./InlineCommentCard";
import { LineActions } from "./LineActions";
import { SelectionComposer } from "./SelectionComposer";
import { useHunkComments } from "./useHunkComments";

hljs.registerLanguage("diff", diff);

type HunkCardProps = {
  hunk: Hunk;
  agents: AgentOption[];
  selectedAgent: AgentKind;
  onAgentChange: (agent: AgentKind) => void;
  onJumpToHunk: (target: { filePath: string; hunkId: string; elementId: string }) => void;
};

function selectionLivesWithin(container: Node, selection: Selection): boolean {
  if (selection.rangeCount === 0) {
    return false;
  }

  return container.contains(selection.getRangeAt(0).commonAncestorContainer);
}

function readSelection(container: Node): Selection | null {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) {
    return null;
  }

  return selectionLivesWithin(container, selection) ? selection : null;
}

function selectionPositionFromRect(rect: DOMRect) {
  return {
    top: rect.bottom + window.scrollY + 8,
    left: Math.min(rect.left + window.scrollX, window.scrollX + window.innerWidth - 280),
  };
}

type FloatingPosition = {
  top: number;
  left: number;
};

type DiffLine = {
  text: string;
  newLineNumber: number | null;
  commentable: boolean;
  highlightedHtml: string;
};

type MoveDiffView = {
  sourceHunkId: string;
  targetHunkId: string;
  lines: string[];
};

type WordPart = {
  text: string;
  changed: boolean;
};

function parseHunkHeader(line: string): { oldStart: number; newStart: number } | null {
  const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
  if (!match) {
    return null;
  }

  return {
    oldStart: Number.parseInt(match[1], 10),
    newStart: Number.parseInt(match[2], 10),
  };
}

function hunkStartLine(header: string): number | null {
  return parseHunkHeader(header)?.newStart ?? null;
}

function moveHintLabel(filePath: string, header: string) {
  const line = hunkStartLine(header);
  return line === null ? filePath : `${filePath}:${line}`;
}

function moveHintTitle(score: number) {
  return `Similarity ${(score * 100).toFixed(0)}%`;
}

function changedLines(patch: string, prefix: "+" | "-", metadataPrefix: "+++" | "---") {
  return patch
    .split("\n")
    .filter((line) => line.startsWith(prefix) && !line.startsWith(metadataPrefix))
    .map((line) => line.slice(1));
}

function tokenizeForWordDiff(line: string) {
  return line.match(/[A-Za-z0-9_]+|\s+|[^\sA-Za-z0-9_]/g) ?? [];
}

function changedWordIndexes(oldTokens: string[], newTokens: string[]) {
  const lengths = Array.from({ length: oldTokens.length + 1 }, () =>
    Array.from({ length: newTokens.length + 1 }, () => 0),
  );

  for (let oldIndex = oldTokens.length - 1; oldIndex >= 0; oldIndex -= 1) {
    for (let newIndex = newTokens.length - 1; newIndex >= 0; newIndex -= 1) {
      lengths[oldIndex][newIndex] = oldTokens[oldIndex] === newTokens[newIndex]
        ? lengths[oldIndex + 1][newIndex + 1] + 1
        : Math.max(lengths[oldIndex + 1][newIndex], lengths[oldIndex][newIndex + 1]);
    }
  }

  const unchangedOld = new Set<number>();
  const unchangedNew = new Set<number>();
  let oldIndex = 0;
  let newIndex = 0;
  while (oldIndex < oldTokens.length && newIndex < newTokens.length) {
    if (oldTokens[oldIndex] === newTokens[newIndex]) {
      unchangedOld.add(oldIndex);
      unchangedNew.add(newIndex);
      oldIndex += 1;
      newIndex += 1;
    } else if (lengths[oldIndex + 1][newIndex] >= lengths[oldIndex][newIndex + 1]) {
      oldIndex += 1;
    } else {
      newIndex += 1;
    }
  }

  return {
    oldChanged: new Set(oldTokens.map((_, index) => index).filter((index) => !unchangedOld.has(index))),
    newChanged: new Set(newTokens.map((_, index) => index).filter((index) => !unchangedNew.has(index))),
  };
}

function wordDiffParts(oldLine: string, newLine: string): { oldParts: WordPart[]; newParts: WordPart[] } {
  const oldTokens = tokenizeForWordDiff(oldLine);
  const newTokens = tokenizeForWordDiff(newLine);
  const { oldChanged, newChanged } = changedWordIndexes(oldTokens, newTokens);

  return {
    oldParts: oldTokens.map((token, index) => ({
      text: token,
      changed: !/^\s+$/.test(token) && oldChanged.has(index),
    })),
    newParts: newTokens.map((token, index) => ({
      text: token,
      changed: !/^\s+$/.test(token) && newChanged.has(index),
    })),
  };
}

function buildMovedCodeDiff(oldLines: string[], newLines: string[]) {
  const lengths = Array.from({ length: oldLines.length + 1 }, () =>
    Array.from({ length: newLines.length + 1 }, () => 0),
  );

  for (let oldIndex = oldLines.length - 1; oldIndex >= 0; oldIndex -= 1) {
    for (let newIndex = newLines.length - 1; newIndex >= 0; newIndex -= 1) {
      lengths[oldIndex][newIndex] = oldLines[oldIndex] === newLines[newIndex]
        ? lengths[oldIndex + 1][newIndex + 1] + 1
        : Math.max(lengths[oldIndex + 1][newIndex], lengths[oldIndex][newIndex + 1]);
    }
  }

  const lines = ["@@ moved code @@"];
  let oldIndex = 0;
  let newIndex = 0;
  while (oldIndex < oldLines.length && newIndex < newLines.length) {
    if (oldLines[oldIndex] === newLines[newIndex]) {
      lines.push(` ${oldLines[oldIndex]}`);
      oldIndex += 1;
      newIndex += 1;
    } else if (lengths[oldIndex + 1][newIndex] >= lengths[oldIndex][newIndex + 1]) {
      lines.push(`-${oldLines[oldIndex]}`);
      oldIndex += 1;
    } else {
      lines.push(`+${newLines[newIndex]}`);
      newIndex += 1;
    }
  }

  while (oldIndex < oldLines.length) {
    lines.push(`-${oldLines[oldIndex]}`);
    oldIndex += 1;
  }
  while (newIndex < newLines.length) {
    lines.push(`+${newLines[newIndex]}`);
    newIndex += 1;
  }

  return lines;
}

function buildDiffLines(text: string): DiffLine[] {
  let oldLineNumber: number | null = null;
  let newLineNumber: number | null = null;

  return text.split("\n").map((line) => {
    let next: DiffLine;

    if (line.startsWith("@@")) {
      const parsed = parseHunkHeader(line);
      oldLineNumber = parsed?.oldStart ?? null;
      newLineNumber = parsed?.newStart ?? null;
      next = {
        text: line,
        newLineNumber: null,
        commentable: false,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
      };
    } else if (line.startsWith("+") && !line.startsWith("+++")) {
      next = {
        text: line,
        newLineNumber,
        commentable: true,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
      };
      newLineNumber = newLineNumber === null ? null : newLineNumber + 1;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      next = {
        text: line,
        newLineNumber: null,
        commentable: true,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
      };
      oldLineNumber = oldLineNumber === null ? null : oldLineNumber + 1;
    } else if (line.startsWith(" ")) {
      next = {
        text: line,
        newLineNumber,
        commentable: true,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
      };
      oldLineNumber = oldLineNumber === null ? null : oldLineNumber + 1;
      newLineNumber = newLineNumber === null ? null : newLineNumber + 1;
    } else {
      next = {
        text: line,
        newLineNumber: null,
        commentable: false,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
      };
    }

    return next;
  });
}

function HighlightedCode({
  text,
  onSelectionStart,
  onSelection,
  onLineNumberClick,
}: {
  text: string;
  onSelectionStart: () => void;
  onSelection: (container: HTMLDivElement) => void;
  onLineNumberClick: (line: string, rect: DOMRect, lineNumber: number) => void;
}) {
  const lines = useMemo(() => buildDiffLines(text), [text]);
  const gutterChars = useMemo(() => {
    const maxLineNumber = lines.reduce(
      (max, line) => (line.newLineNumber !== null ? Math.max(max, line.newLineNumber) : max),
      0,
    );
    return Math.max(String(maxLineNumber || 0).length, 2);
  }, [lines]);

  return (
    <div
      className="diff-code"
      style={{ "--diff-gutter-ch": gutterChars } as CSSProperties}
      onMouseDown={onSelectionStart}
      onMouseUp={(event) => onSelection(event.currentTarget)}
      onKeyUp={(event) => onSelection(event.currentTarget)}
    >
      {lines.map((line, index) => (
        <div key={`${index}:${line.text}`} className="diff-line">
          <button
            type="button"
            className={`diff-gutter-button ${line.commentable && line.newLineNumber !== null ? "diff-gutter-button-active" : ""}`.trim()}
            onClick={(event) => {
              if (line.commentable && line.newLineNumber !== null) {
                onLineNumberClick(
                  line.text,
                  event.currentTarget.getBoundingClientRect(),
                  line.newLineNumber,
                );
              }
            }}
            aria-label={
              line.newLineNumber !== null
                ? `Add comment on new line ${line.newLineNumber}`
                : "No line number"
            }
          >
            {line.newLineNumber ?? ""}
          </button>
          <div
            className="diff-line-code"
            dangerouslySetInnerHTML={{ __html: line.highlightedHtml || "&nbsp;" }}
          />
        </div>
      ))}
    </div>
  );
}

function MovedDiffCode({ lines }: { lines: string[] }) {
  const wordDiffs = useMemo(() => {
    const diffs = new Map<number, WordPart[]>();

    for (let index = 0; index < lines.length - 1; index += 1) {
      const oldLine = lines[index];
      const newLine = lines[index + 1];
      if (oldLine.startsWith("-") && newLine.startsWith("+")) {
        const { oldParts, newParts } = wordDiffParts(oldLine.slice(1), newLine.slice(1));
        diffs.set(index, oldParts);
        diffs.set(index + 1, newParts);
        index += 1;
      }
    }

    return diffs;
  }, [lines]);

  return (
    <div className="diff-code" style={{ "--diff-gutter-ch": 2 } as CSSProperties}>
      {lines.map((line, index) => {
        const parts = wordDiffs.get(index);
        const prefix = line.startsWith("+") || line.startsWith("-") || line.startsWith(" ")
          ? line.slice(0, 1)
          : "";
        const text = prefix ? line.slice(1) : line;
        return (
          <div key={`${index}:${line}`} className="diff-line">
            <button type="button" className="diff-gutter-button" aria-label="No line number" />
            <div className="diff-line-code">
              {prefix}
              {parts
                ? parts.map((part, partIndex) => (
                    <span
                      key={`${partIndex}:${part.text}`}
                      className={part.changed ? "move-word-changed" : undefined}
                    >
                      {part.text}
                    </span>
                  ))
                : text}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export function HunkCard({
  hunk,
  agents,
  selectedAgent,
  onAgentChange,
  onJumpToHunk,
}: HunkCardProps) {
  const {
    state: { activeHunkId, data },
    actions,
  } = useReviewStore();
  const hunkRef = useRef<HTMLElement | null>(null);
  const composerOpenRef = useRef(false);
  const selectionStartedInHunkRef = useRef(false);
  const [expanded, setExpanded] = useState(false);
  const [fullPatch, setFullPatch] = useState<string | null>(null);
  const [loadingPatch, setLoadingPatch] = useState(false);
  const [loadingMoveDiff, setLoadingMoveDiff] = useState(false);
  const [moveDiffView, setMoveDiffView] = useState<MoveDiffView | null>(null);
  const [commentValue, setCommentValue] = useState(hunk.comment);
  const movedFrom = hunk.moved_from;
  const movedTo = hunk.moved_to;
  const [selectedText, setSelectedText] = useState("");
  const [composerOpen, setComposerOpen] = useState(false);
  const [selectionPosition, setSelectionPosition] = useState<{ top: number; left: number } | null>(null);
  const [lockedSelectionPosition, setLockedSelectionPosition] = useState<{ top: number; left: number } | null>(
    null,
  );

  useEffect(() => {
    setCommentValue(hunk.comment);
  }, [hunk.comment]);

  useEffect(() => {
    if (!moveDiffView) {
      return;
    }

    const counterpartHunkId = moveDiffView.sourceHunkId === hunk.id
      ? moveDiffView.targetHunkId
      : moveDiffView.sourceHunkId;
    const counterpart = document.getElementById(`hunk-${counterpartHunkId}`);
    counterpart?.classList.add("hunk-move-counterpart-hidden");

    return () => counterpart?.classList.remove("hunk-move-counterpart-hidden");
  }, [hunk.id, moveDiffView]);

  useEffect(() => {
    composerOpenRef.current = composerOpen;
  }, [composerOpen]);

  useEffect(() => {
    function finalizeSelectionFromRoot() {
      if (composerOpenRef.current) {
        return;
      }

      if (!selectionStartedInHunkRef.current) {
        return;
      }

      const root = hunkRef.current;
      if (!root) {
        selectionStartedInHunkRef.current = false;
        return;
      }

      captureSelection(root);
      selectionStartedInHunkRef.current = false;
    }

    function handleSelectionChange() {
      if (composerOpenRef.current) {
        return;
      }

      if (selectionStartedInHunkRef.current) {
        return;
      }

      const selection = window.getSelection();
      if (!selection || selection.isCollapsed) {
        clearSelectionUi();
        return;
      }

      const root = hunkRef.current;
      if (!root) {
        clearSelectionUi();
        return;
      }

      if (!selectionLivesWithin(root, selection)) {
        clearSelectionUi();
      }
    }

    document.addEventListener("selectionchange", handleSelectionChange);
    window.addEventListener("mouseup", finalizeSelectionFromRoot);
    window.addEventListener("keyup", finalizeSelectionFromRoot);
    return () => {
      document.removeEventListener("selectionchange", handleSelectionChange);
      window.removeEventListener("mouseup", finalizeSelectionFromRoot);
      window.removeEventListener("keyup", finalizeSelectionFromRoot);
    };
  }, []);

  const patchPreviewLineLimit = data?.patch_preview_line_limit ?? 500;
  const isLong = hunk.patch_line_count > patchPreviewLineLimit;
  const visiblePatch = useMemo(() => {
    if (expanded && fullPatch) {
      return fullPatch;
    }
    return hunk.patch_preview;
  }, [expanded, fullPatch, hunk.patch_preview]);

  const parsedComments = useMemo(() => parseAnchoredComments(commentValue), [commentValue]);
  const readOnly = data?.read_only ?? false;
  const isCommitReview = Boolean(data?.active_commit);
  const moveDiffSourceHunk = moveDiffView
    ? data?.hunks.find((candidate) => candidate.id === moveDiffView.sourceHunkId) ?? null
    : null;
  const moveDiffTargetHunk = moveDiffView
    ? data?.hunks.find((candidate) => candidate.id === moveDiffView.targetHunkId) ?? null
    : null;
  const canStageMove = Boolean(
    moveDiffView &&
      !readOnly &&
      moveDiffSourceHunk &&
      moveDiffTargetHunk &&
      (!moveDiffSourceHunk.staged || !moveDiffTargetHunk.staged),
  );
  const { activeDraft, diffSegments, openSelectionDraft, commentContextValue } = useHunkComments({
    hunk,
    visiblePatch,
    commentValue,
    setCommentValue,
    agents,
    selectedAgent,
    onAgentChange,
    clearSelectionUi,
    setSelectedText,
    setComposerOpen,
    setLockedSelectionPosition,
  });

  function captureSelection(container: Node) {
    if (composerOpenRef.current) {
      return;
    }

    window.requestAnimationFrame(() => {
      const selection = readSelection(container);
      if (!selection) {
        return;
      }

      const text = selection.toString().trim();
      if (!text) {
        return;
      }

      const rect = selection.getRangeAt(0).getBoundingClientRect();
      setSelectedText(text);
      setComposerOpen(false);
      setSelectionPosition(selectionPositionFromRect(rect));
    });
  }

  function clearSelectionUi() {
    composerOpenRef.current = false;
    setSelectedText("");
    setComposerOpen(false);
    setSelectionPosition(null);
    setLockedSelectionPosition(null);
  }

  async function toggleExpanded() {
    if (expanded) {
      setExpanded(false);
      return;
    }

    if (fullPatch === null) {
      setLoadingPatch(true);
      try {
        const payload = await fetchHunkPatch(hunk.id);
        setFullPatch(payload.patch);
      } finally {
        setLoadingPatch(false);
      }
    }

    setExpanded(true);
  }

  async function patchForHunk(hunkId: string) {
    if (hunkId === hunk.id && fullPatch) {
      return fullPatch;
    }
    if (hunkId === hunk.id && !isLong) {
      return hunk.patch_preview;
    }

    const payload = await fetchHunkPatch(hunkId);
    if (hunkId === hunk.id) {
      setFullPatch(payload.patch);
    }
    return payload.patch;
  }

  async function showMoveDiff(sourceHunkId: string, targetHunkId: string) {
    setLoadingMoveDiff(true);
    try {
      const [sourcePatch, targetPatch] = await Promise.all([
        patchForHunk(sourceHunkId),
        patchForHunk(targetHunkId),
      ]);
      setMoveDiffView({
        sourceHunkId,
        targetHunkId,
        lines: buildMovedCodeDiff(
          changedLines(sourcePatch, "-", "---"),
          changedLines(targetPatch, "+", "+++"),
        ),
      });
    } finally {
      setLoadingMoveDiff(false);
    }
  }

  async function stageMove() {
    if (!moveDiffSourceHunk || !moveDiffTargetHunk) {
      return;
    }

    await actions.stageHunks([
      { hunkId: moveDiffSourceHunk.id, staged: moveDiffSourceHunk.staged },
      { hunkId: moveDiffTargetHunk.id, staged: moveDiffTargetHunk.staged },
    ]);
  }

  function confirmDiscardHunk() {
    if (!window.confirm("Discard this hunk?")) {
      return;
    }

    void actions.discardHunk(hunk.id);
  }


  return (
    <article
      id={`hunk-${hunk.id}`}
      className={`panel hunk ${activeHunkId === hunk.id ? "hunk-active" : ""} ${hunk.reviewed ? "hunk-reviewed" : ""}`.trim()}
      data-hunk-id={hunk.id}
      ref={hunkRef}
    >
      <div className="hunk-actions">
        {isCommitReview ? (
          <button onClick={() => void actions.setReviewed(hunk.id, !hunk.reviewed)}>
            {hunk.reviewed ? "Mark Unreviewed" : "Mark Reviewed"}
          </button>
        ) : null}
        {!readOnly ? (
          <>
            <button onClick={() => void actions.toggleStage(hunk.id, hunk.staged)}>
              {hunk.staged ? "Unstage Hunk" : "Stage Hunk"}
            </button>
            <button onClick={confirmDiscardHunk}>Discard Hunk</button>
          </>
        ) : null}
        {isLong ? (
          <button onClick={() => void toggleExpanded()}>
            {expanded
              ? "Collapse Diff"
              : loadingPatch
                ? "Loading Diff..."
                : `Expand Diff (${hunk.patch_line_count} lines)`}
          </button>
        ) : null}
        <a
          className="hunk-full-file-link"
          href={buildFullFileUrl(hunk.file_path, hunkStartLine(hunk.header))}
          target="_blank"
          rel="noreferrer"
        >
          View full file
        </a>
      </div>

      {selectedText && !composerOpen && selectionPosition ? (
        <LineActions
          style={{ top: selectionPosition.top, left: selectionPosition.left }}
          onAddComment={() => {
            composerOpenRef.current = true;
            openSelectionDraft(selectedText, selectionPosition);
          }}
          onStageLines={
            readOnly
              ? undefined
              : () => {
                  void actions.stageSelection(hunk.id, selectedText);
                  clearSelectionUi();
                }
          }
        />
      ) : null}

      <HunkCommentContextProvider value={commentContextValue}>
        {activeDraft && composerOpen && lockedSelectionPosition ? (
          <SelectionComposer
            draftId={activeDraft.id}
            style={{ top: lockedSelectionPosition.top + 36, left: lockedSelectionPosition.left }}
          />
        ) : null}

        <div className={`patch-wrap ${!expanded && isLong ? "patch-truncated" : ""}`.trim()}>
          {movedFrom || movedTo ? (
            <div className="hunk-move-hints">
              {movedFrom ? (
                <span>
                  Appears to come from{" "}
                  <a
                    href={`#hunk-${movedFrom.target_hunk_id}`}
                    title={moveHintTitle(movedFrom.score)}
                    onClick={(event) => {
                      event.preventDefault();
                      onJumpToHunk({
                        filePath: movedFrom.target_file_path,
                        hunkId: movedFrom.target_hunk_id,
                        elementId: `hunk-${movedFrom.target_hunk_id}`,
                      });
                    }}
                  >
                    {moveHintLabel(movedFrom.target_file_path, movedFrom.target_header)}
                  </a>{" "}
                  {!moveDiffView ? (
                    <button
                      type="button"
                      className="hunk-inline-link"
                      disabled={loadingMoveDiff}
                      onClick={() => void showMoveDiff(movedFrom.target_hunk_id, hunk.id)}
                    >
                      [diff]
                    </button>
                  ) : null}
                </span>
              ) : null}
              {movedTo ? (
                <span>
                  Appears to have moved to{" "}
                  <a
                    href={`#hunk-${movedTo.target_hunk_id}`}
                    title={moveHintTitle(movedTo.score)}
                    onClick={(event) => {
                      event.preventDefault();
                      onJumpToHunk({
                        filePath: movedTo.target_file_path,
                        hunkId: movedTo.target_hunk_id,
                        elementId: `hunk-${movedTo.target_hunk_id}`,
                      });
                    }}
                  >
                    {moveHintLabel(movedTo.target_file_path, movedTo.target_header)}
                  </a>{" "}
                  {!moveDiffView ? (
                    <button
                      type="button"
                      className="hunk-inline-link"
                      disabled={loadingMoveDiff}
                      onClick={() => void showMoveDiff(hunk.id, movedTo.target_hunk_id)}
                    >
                      [diff]
                    </button>
                  ) : null}
                </span>
              ) : null}
              {moveDiffView ? (
                <>
                  {canStageMove ? (
                    <button
                      type="button"
                      className="hunk-inline-link"
                      onClick={() => void stageMove()}
                    >
                      [stage source and destination]
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className="hunk-inline-link"
                    onClick={() => setMoveDiffView(null)}
                  >
                    [back]
                  </button>
                </>
              ) : null}
            </div>
          ) : null}
          {moveDiffView ? (
            <div className="diff-stack">
              <MovedDiffCode lines={moveDiffView.lines} />
            </div>
          ) : (
            <div className="diff-stack">
              {diffSegments.map((segment, index) =>
                segment.type === "code" ? (
                  <HighlightedCode
                    key={`code-${index}`}
                    text={segment.text}
                    onSelectionStart={() => {
                      selectionStartedInHunkRef.current = true;
                    }}
                    onSelection={captureSelection}
                    onLineNumberClick={(line, rect, lineNumber) =>
                      openSelectionDraft(line, selectionPositionFromRect(rect), lineNumber)
                    }
                  />
                ) : segment.type === "comment" ? (
                  <InlineCommentCard
                    key={`comment-${index}`}
                    id={`comment-${hunk.id}-${segment.index}`}
                    segment={segment}
                  />
                ) : (
                  <SelectionComposer
                    key={`draft-${segment.draftId}`}
                    draftId={segment.draftId}
                  />
                ),
              )}
            </div>
          )}
        </div>
      </HunkCommentContextProvider>
    </article>
  );
}
