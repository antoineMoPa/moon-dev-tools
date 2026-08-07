import { useEffect, useMemo, useRef, useState } from "react";
import { fetchHunkPatch } from "../../api";
import {
  buildAnchoredCommentValue,
  parseAnchoredComments,
  type AnchoredComment,
} from "../../anchoredComments";
import { useReviewStore } from "../../reviewStore";
import type { AgentKind, AgentOption, Hunk } from "../../types";
import { FullFileModal } from "../FullFileModal";
import { splitDiffIntoSegments } from "./diffSegments";
import { HighlightedCode } from "./HunkDiffCode";
import { HunkCommentContextProvider } from "./HunkCommentContext";
import { hunkStartLine } from "./hunkHeaders";
import { InlineCommentCard } from "./InlineCommentCard";
import { LineActions } from "./LineActions";
import {
  buildMovedCodeDiff,
  changedLines,
  MovedDiffCode,
  type MoveDiffView,
} from "./MovedDiffCode";
import { SelectionComposer } from "./SelectionComposer";
import { useHunkComments } from "./useHunkComments";

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

function expandedPatchSelection(container: Node, selection: Selection) {
  const root = container instanceof Element ? container : container.parentElement;
  if (!root || selection.rangeCount === 0) {
    return selection.toString().trim();
  }

  const range = selection.getRangeAt(0);
  const selectedLines = Array.from(root.querySelectorAll<HTMLElement>("[data-patch-line]"))
    .filter((element) => range.intersectsNode(element))
    .map((element) => element.dataset.patchLine ?? "")
    .filter(Boolean);

  return selectedLines.length > 0 ? selectedLines.join("\n") : selection.toString().trim();
}

type FloatingPosition = {
  top: number;
  left: number;
};

function moveHintLabel(filePath: string, header: string) {
  const line = hunkStartLine(header);
  return line === null ? filePath : `${filePath}:${line}`;
}

function moveHintTitle(score: number) {
  return `Similarity ${(score * 100).toFixed(0)}%`;
}

function ImageDiffPreview({ beforeSrc, afterSrc }: { beforeSrc?: string | null; afterSrc?: string | null }) {
  return (
    <div className="image-diff" aria-label="Image before and after comparison">
      <figure className="image-diff-pane">
        <figcaption>Before</figcaption>
        {beforeSrc ? (
          <img src={beforeSrc} alt="Before change" />
        ) : (
          <div className="image-diff-empty">No image</div>
        )}
      </figure>
      <figure className="image-diff-pane">
        <figcaption>After</figcaption>
        {afterSrc ? (
          <img src={afterSrc} alt="After change" />
        ) : (
          <div className="image-diff-empty">No image</div>
        )}
      </figure>
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
    state: { activeHunkId, data, movedDiffLayout, sessionId },
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
  const [moveDiffDismissed, setMoveDiffDismissed] = useState(false);
  const [fullFileOpen, setFullFileOpen] = useState(false);
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
  const canDiscardMove = Boolean(moveDiffView && !readOnly && moveDiffSourceHunk && moveDiffTargetHunk);
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

      const text = expandedPatchSelection(container, selection);
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
        const payload = await fetchHunkPatch(sessionId, hunk.id);
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

    const payload = await fetchHunkPatch(sessionId, hunkId);
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

  useEffect(() => {
    if (moveDiffView || moveDiffDismissed || loadingMoveDiff) {
      return;
    }

    if (movedTo) {
      void showMoveDiff(hunk.id, movedTo.target_hunk_id);
      return;
    }

    if (!movedFrom) {
      return;
    }

    if (document.getElementById(`hunk-${movedFrom.target_hunk_id}`)) {
      return;
    }

    void showMoveDiff(movedFrom.target_hunk_id, hunk.id);
  }, [hunk.id, loadingMoveDiff, moveDiffDismissed, moveDiffView, movedFrom, movedTo]);

  async function stageMove() {
    if (!moveDiffSourceHunk || !moveDiffTargetHunk) {
      return;
    }

    await actions.stageHunks([
      { hunkId: moveDiffSourceHunk.id, staged: moveDiffSourceHunk.staged },
      { hunkId: moveDiffTargetHunk.id, staged: moveDiffTargetHunk.staged },
    ]);
  }

  function confirmDiscardMove() {
    if (!moveDiffSourceHunk || !moveDiffTargetHunk) {
      return;
    }

    if (!window.confirm("Discard source and destination hunks?")) {
      return;
    }

    void actions.discardHunks([moveDiffSourceHunk.id, moveDiffTargetHunk.id]);
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
        {isCommitReview ? (
          <button
            className="hunk-reviewed-toggle"
            onClick={() => void actions.setReviewed(hunk.id, !hunk.reviewed)}
          >
            [{hunk.reviewed ? "mark unreviewed" : "mark reviewed"}]
          </button>
        ) : null}
        {!readOnly && !moveDiffView ? (
          <>
            <button onClick={() => void actions.toggleStage(hunk.id, hunk.staged)}>
              [{hunk.staged ? "unstage hunk" : "stage hunk"}]
            </button>
            <button onClick={confirmDiscardHunk}>[discard hunk]</button>
          </>
        ) : null}
        {isLong ? (
          <button onClick={() => void toggleExpanded()}>
            {expanded
              ? "[collapse diff]"
              : loadingPatch
                ? "[loading diff...]"
                : `[expand diff (${hunk.patch_line_count} lines)]`}
          </button>
        ) : null}
        <button
          className="hunk-full-file-link"
          type="button"
          onClick={() => setFullFileOpen(true)}
        >
          [view file]
        </button>
      </div>

      {fullFileOpen ? (
        <FullFileModal
          filePath={hunk.file_path}
          lineNumber={hunkStartLine(hunk.header)}
          onClose={() => setFullFileOpen(false)}
        />
      ) : null}

      {selectedText && !composerOpen && selectionPosition ? (
        <LineActions
          style={{ top: selectionPosition.top, left: selectionPosition.left }}
          onClose={clearSelectionUi}
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
                  {canDiscardMove ? (
                    <button
                      type="button"
                      className="hunk-inline-link"
                      onClick={confirmDiscardMove}
                    >
                      [discard source and destination]
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className="hunk-inline-link"
                    onClick={() => {
                      setMoveDiffDismissed(true);
                      setMoveDiffView(null);
                    }}
                  >
                    [back to per-file view]
                  </button>
                </>
              ) : null}
            </div>
          ) : null}
          {moveDiffView ? (
            <div className="diff-stack">
              <MovedDiffCode lines={moveDiffView.lines} layout={movedDiffLayout} />
            </div>
          ) : (
            <div className="diff-stack">
              {hunk.image_diff ? (
                <ImageDiffPreview
                  beforeSrc={hunk.image_diff.before_src}
                  afterSrc={hunk.image_diff.after_src}
                />
              ) : null}
              {diffSegments.map((segment, index) =>
                segment.type === "code" ? (
                  <HighlightedCode
                    key={`code-${index}`}
                    text={segment.text}
                    layout={movedDiffLayout}
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
