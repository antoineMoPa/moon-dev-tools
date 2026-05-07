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

export function HunkCard({
  hunk,
  agents,
  selectedAgent,
  onAgentChange,
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
  const [commentValue, setCommentValue] = useState(hunk.comment);
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

  function confirmDiscardHunk() {
    if (!window.confirm("Discard this hunk?")) {
      return;
    }

    void actions.discardHunk(hunk.id);
  }


  return (
    <article
      id={`hunk-${hunk.id}`}
      className={`panel hunk ${activeHunkId === hunk.id ? "hunk-active" : ""}`.trim()}
      data-hunk-id={hunk.id}
      ref={hunkRef}
    >
      <div className="hunk-actions">
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
        </div>
      </HunkCommentContextProvider>
    </article>
  );
}
