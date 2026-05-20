import type { ReactNode } from "react";

export type WordPart = {
  text: string;
  changed: boolean;
};

function tokenizeForWordDiff(line: string) {
  return line.match(/[A-Za-z0-9_]|\s+|[^\sA-Za-z0-9_]/g) ?? [];
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

export function wordDiffParts(oldLine: string, newLine: string): { oldParts: WordPart[]; newParts: WordPart[] } {
  const oldTokens = tokenizeForWordDiff(oldLine);
  const newTokens = tokenizeForWordDiff(newLine);
  const { oldChanged, newChanged } = changedWordIndexes(oldTokens, newTokens);

  return {
    oldParts: mergeAdjacentParts(oldTokens.map((token, index) => ({
      text: token,
      changed: !/^\s+$/.test(token) && oldChanged.has(index),
    }))),
    newParts: mergeAdjacentParts(newTokens.map((token, index) => ({
      text: token,
      changed: !/^\s+$/.test(token) && newChanged.has(index),
    }))),
  };
}

function mergeAdjacentParts(parts: WordPart[]) {
  const merged: WordPart[] = [];

  for (const part of parts) {
    const previous = merged[merged.length - 1];
    if (previous?.changed === part.changed) {
      previous.text += part.text;
    } else {
      merged.push({ ...part });
    }
  }

  return merged;
}

export function WordDiffText({
  parts,
  changedClass,
  fallback,
}: {
  parts?: WordPart[];
  changedClass: string;
  fallback: ReactNode;
}) {
  if (!parts) {
    return <>{fallback}</>;
  }

  return (
    <>
      {parts.map((part, partIndex) => (
        <span
          key={`${partIndex}:${part.text}`}
          className={part.changed ? changedClass : undefined}
        >
          {part.text}
        </span>
      ))}
    </>
  );
}
