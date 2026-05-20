import { useMemo, type CSSProperties } from "react";
import type { MovedDiffLayout } from "../../reviewStore";
import { WordDiffText, wordDiffParts, type WordPart } from "./wordDiff";

export type MoveDiffView = {
  sourceHunkId: string;
  targetHunkId: string;
  lines: string[];
};

type SideBySideMoveRow = {
  oldLine: string | null;
  newLine: string | null;
};

export function changedLines(patch: string, prefix: "+" | "-", metadataPrefix: "+++" | "---") {
  return patch
    .split("\n")
    .filter((line) => line.startsWith(prefix) && !line.startsWith(metadataPrefix))
    .map((line) => line.slice(1));
}

export function buildMovedCodeDiff(oldLines: string[], newLines: string[]) {
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

function sideBySideRows(lines: string[]): SideBySideMoveRow[] {
  const rows: SideBySideMoveRow[] = [];

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const nextLine = lines[index + 1];
    if (line.startsWith("@@")) {
      continue;
    }
    if (line.startsWith(" ")) {
      const text = line.slice(1);
      rows.push({ oldLine: text, newLine: text });
    } else if (line.startsWith("-") && nextLine?.startsWith("+")) {
      rows.push({ oldLine: line.slice(1), newLine: nextLine.slice(1) });
      index += 1;
    } else if (line.startsWith("-")) {
      rows.push({ oldLine: line.slice(1), newLine: null });
    } else if (line.startsWith("+")) {
      rows.push({ oldLine: null, newLine: line.slice(1) });
    }
  }

  return rows;
}

function UnifiedMovedDiffCode({ lines }: { lines: string[] }) {
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
        const wordChangedClass = line.startsWith("+")
          ? "move-word-changed move-word-changed-added"
          : line.startsWith("-")
            ? "move-word-changed move-word-changed-removed"
            : "move-word-changed";
        const prefix = line.startsWith("+") || line.startsWith("-") || line.startsWith(" ")
          ? line.slice(0, 1)
          : "";
        const text = prefix ? line.slice(1) : line;
        const lineClass = line.startsWith("+")
          ? "diff-line-code diff-split-line-added"
          : line.startsWith("-")
            ? "diff-line-code diff-split-line-removed"
            : line.startsWith("@@")
              ? "diff-line-code diff-split-line-meta"
              : "diff-line-code";
        return (
          <div key={`${index}:${line}`} className="diff-line">
            <button type="button" className="diff-gutter-button" aria-label="No line number" />
            <div className={lineClass}>
              {prefix}
              <WordDiffText
                parts={parts}
                changedClass={wordChangedClass}
                fallback={text}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}

function SideBySideMovedDiffCode({ lines }: { lines: string[] }) {
  const rows = useMemo(() => sideBySideRows(lines), [lines]);
  const gutterChars = Math.max(String(rows.length).length, 2);

  return (
    <div
      className="split-diff"
      style={{ "--move-gutter-ch": gutterChars } as CSSProperties}
    >
      {rows.map((row, index) => {
        const pairedChange = row.oldLine !== null && row.newLine !== null && row.oldLine !== row.newLine;
        const wordParts = pairedChange && row.oldLine !== null && row.newLine !== null
          ? wordDiffParts(row.oldLine, row.newLine)
          : null;
        const rowClass = pairedChange
          ? "split-diff-row-changed"
          : row.oldLine === null
            ? "split-diff-row-added"
            : row.newLine === null
              ? "split-diff-row-removed"
              : "";
        return (
          <div key={`${index}:${row.oldLine ?? ""}:${row.newLine ?? ""}`} className="split-diff-row">
            <div className={`split-diff-gutter ${rowClass}`.trim()}>
              {row.oldLine === null ? "" : index + 1}
            </div>
            <div className={`split-diff-cell ${rowClass} ${row.oldLine === null ? "split-diff-empty" : "diff-split-line-removed"}`.trim()}>
              {row.oldLine === null ? null : (
                <WordDiffText
                  parts={wordParts?.oldParts}
                  changedClass="move-word-changed move-word-changed-removed"
                  fallback={row.oldLine}
                />
              )}
            </div>
            <div className={`split-diff-gutter ${rowClass}`.trim()}>
              {row.newLine === null ? "" : index + 1}
            </div>
            <div className={`split-diff-cell ${rowClass} ${row.newLine === null ? "split-diff-empty" : "diff-split-line-added"}`.trim()}>
              {row.newLine === null ? null : (
                <WordDiffText
                  parts={wordParts?.newParts}
                  changedClass="move-word-changed move-word-changed-added"
                  fallback={row.newLine}
                />
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export function MovedDiffCode({ lines, layout }: { lines: string[]; layout: MovedDiffLayout }) {
  return layout === "side-by-side"
    ? <SideBySideMovedDiffCode lines={lines} />
    : <UnifiedMovedDiffCode lines={lines} />;
}
