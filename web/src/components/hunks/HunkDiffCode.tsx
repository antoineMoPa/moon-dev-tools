import { useMemo, type CSSProperties } from "react";
import hljs from "highlight.js/lib/core";
import diff from "highlight.js/lib/languages/diff";
import type { MovedDiffLayout } from "../../reviewStore";
import { parseHunkHeader } from "./hunkHeaders";

hljs.registerLanguage("diff", diff);

type DiffLine = {
  text: string;
  oldLineNumber: number | null;
  newLineNumber: number | null;
  commentable: boolean;
  highlightedHtml: string;
  kind: "header" | "added" | "removed" | "context" | "other";
};

type SideBySideDiffRow = {
  oldLine: DiffLine | null;
  newLine: DiffLine | null;
};

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
        oldLineNumber: null,
        newLineNumber: null,
        commentable: false,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
        kind: "header",
      };
    } else if (line.startsWith("+") && !line.startsWith("+++")) {
      next = {
        text: line,
        oldLineNumber: null,
        newLineNumber,
        commentable: true,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
        kind: "added",
      };
      newLineNumber = newLineNumber === null ? null : newLineNumber + 1;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      next = {
        text: line,
        oldLineNumber,
        newLineNumber: null,
        commentable: true,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
        kind: "removed",
      };
      oldLineNumber = oldLineNumber === null ? null : oldLineNumber + 1;
    } else if (line.startsWith(" ")) {
      next = {
        text: line,
        oldLineNumber,
        newLineNumber,
        commentable: true,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
        kind: "context",
      };
      oldLineNumber = oldLineNumber === null ? null : oldLineNumber + 1;
      newLineNumber = newLineNumber === null ? null : newLineNumber + 1;
    } else {
      next = {
        text: line,
        oldLineNumber: null,
        newLineNumber: null,
        commentable: false,
        highlightedHtml: hljs.highlight(line, { language: "diff" }).value,
        kind: "other",
      };
    }

    return next;
  });
}

function diffLineBody(line: DiffLine | null) {
  if (!line) {
    return "";
  }
  return line.kind === "added" || line.kind === "removed" || line.kind === "context"
    ? line.text.slice(1)
    : line.text;
}

function diffSideBySideRows(lines: DiffLine[]) {
  const rows: SideBySideDiffRow[] = [];

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const nextLine = lines[index + 1];
    if (line.kind === "header" || line.kind === "other" || line.kind === "context") {
      rows.push({ oldLine: line, newLine: line });
    } else if (line.kind === "removed" && nextLine?.kind === "added") {
      rows.push({ oldLine: line, newLine: nextLine });
      index += 1;
    } else if (line.kind === "removed") {
      rows.push({ oldLine: line, newLine: null });
    } else if (line.kind === "added") {
      rows.push({ oldLine: null, newLine: line });
    }
  }

  return rows;
}

function sideBySideCellClass(line: DiffLine | null, side: "old" | "new") {
  if (!line) {
    return "split-diff-cell split-diff-empty";
  }
  if (line.kind === "header" || line.kind === "other") {
    return "split-diff-cell diff-split-line-meta";
  }
  if (line.kind === "context") {
    return "split-diff-cell";
  }
  return side === "old"
    ? "split-diff-cell diff-split-line-removed split-diff-row-removed"
    : "split-diff-cell diff-split-line-added split-diff-row-added";
}

function SideBySideHighlightedCode({
  lines,
  onSelectionStart,
  onSelection,
  onLineNumberClick,
}: {
  lines: DiffLine[];
  onSelectionStart: () => void;
  onSelection: (container: HTMLDivElement) => void;
  onLineNumberClick: (line: string, rect: DOMRect, lineNumber: number) => void;
}) {
  const rows = useMemo(() => diffSideBySideRows(lines), [lines]);
  const maxLineNumber = lines.reduce(
    (max, line) => Math.max(max, line.oldLineNumber ?? 0, line.newLineNumber ?? 0),
    0,
  );
  const gutterChars = Math.max(String(maxLineNumber || 0).length, 2);

  return (
    <div
      className="split-diff"
      style={{ "--move-gutter-ch": gutterChars } as CSSProperties}
      onMouseDown={onSelectionStart}
      onMouseUp={(event) => onSelection(event.currentTarget)}
      onKeyUp={(event) => onSelection(event.currentTarget)}
    >
      {rows.map((row, index) => (
        <div key={`${index}:${row.oldLine?.text ?? ""}:${row.newLine?.text ?? ""}`} className="split-diff-row">
          <button type="button" className="split-diff-gutter" aria-label="Source line">
            {row.oldLine?.oldLineNumber ?? ""}
          </button>
          <div className={sideBySideCellClass(row.oldLine, "old")}>{diffLineBody(row.oldLine)}</div>
          <button
            type="button"
            className={`split-diff-gutter ${row.newLine?.commentable && row.newLine.newLineNumber !== null ? "diff-gutter-button-active" : ""}`.trim()}
            onClick={(event) => {
              if (row.newLine?.commentable && row.newLine.newLineNumber !== null) {
                onLineNumberClick(
                  row.newLine.text,
                  event.currentTarget.getBoundingClientRect(),
                  row.newLine.newLineNumber,
                );
              }
            }}
            aria-label={
              row.newLine?.newLineNumber !== null && row.newLine?.newLineNumber !== undefined
                ? `Add comment on new line ${row.newLine.newLineNumber}`
                : "No destination line"
            }
          >
            {row.newLine?.newLineNumber ?? ""}
          </button>
          <div className={sideBySideCellClass(row.newLine, "new")}>{diffLineBody(row.newLine)}</div>
        </div>
      ))}
    </div>
  );
}

export function HighlightedCode({
  text,
  layout,
  onSelectionStart,
  onSelection,
  onLineNumberClick,
}: {
  text: string;
  layout: MovedDiffLayout;
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

  if (layout === "side-by-side") {
    return (
      <SideBySideHighlightedCode
        lines={lines}
        onSelectionStart={onSelectionStart}
        onSelection={onSelection}
        onLineNumberClick={onLineNumberClick}
      />
    );
  }

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
