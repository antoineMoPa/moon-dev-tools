import type { Hunk, SessionState } from "./types";

export function filePathsInListOrder(hunks: Hunk[]) {
  const seen = new Set<string>();
  const ordered: string[] = [];
  for (const hunk of hunks) {
    if (seen.has(hunk.file_path)) {
      continue;
    }
    seen.add(hunk.file_path);
    ordered.push(hunk.file_path);
  }
  return ordered;
}

export function hasUnstagedHunks(data: SessionState | null) {
  return data?.hunks.some((hunk) => !hunk.staged) ?? false;
}

function fileExists(data: SessionState, filePath: string) {
  return data.hunks.some((hunk) => hunk.file_path === filePath);
}

function fileHasUnstagedHunks(data: SessionState, filePath: string) {
  return data.hunks.some((hunk) => hunk.file_path === filePath && !hunk.staged);
}

export function fullyStagedFilePaths(previousData: SessionState, data: SessionState) {
  return filePathsInListOrder(previousData.hunks).filter((filePath) => (
    fileHasUnstagedHunks(previousData, filePath) &&
    fileExists(data, filePath) &&
    !fileHasUnstagedHunks(data, filePath)
  ));
}
