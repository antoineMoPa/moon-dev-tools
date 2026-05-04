import type { FileChangeKind, SessionState } from "../types";

export const FILE_STAGE_STATUS = {
  staged: "staged",
  unstaged: "unstaged",
  partial: "partial",
} as const;

export type FileStageStatus = (typeof FILE_STAGE_STATUS)[keyof typeof FILE_STAGE_STATUS];

export type SidebarFileItem = {
  filePath: string;
  fileName: string;
  changeKind: FileChangeKind;
  snoozed: boolean;
  status: FileStageStatus;
  added_line_count: number;
  removed_line_count: number;
  unstaged_added_line_count: number;
  unstaged_removed_line_count: number;
  staged_added_line_count: number;
  staged_removed_line_count: number;
};

function fileNameFromPath(filePath: string) {
  const segments = filePath.split("/");
  return segments[segments.length - 1] || filePath;
}

function mergeFileChangeKind(left: FileChangeKind, right: FileChangeKind): FileChangeKind {
  return left === right ? left : "modified";
}

export function buildSidebarFiles(data: SessionState, snoozedFiles: Set<string>): SidebarFileItem[] {
  const grouped = new Map<string, SidebarFileItem>();
  for (const hunk of data.hunks) {
    const existing = grouped.get(hunk.file_path);
    if (existing) {
      existing.changeKind = mergeFileChangeKind(existing.changeKind, hunk.change_kind);
      existing.added_line_count += hunk.added_line_count;
      existing.removed_line_count += hunk.removed_line_count;
      if (hunk.staged) {
        existing.staged_added_line_count += hunk.added_line_count;
        existing.staged_removed_line_count += hunk.removed_line_count;
      } else {
        existing.unstaged_added_line_count += hunk.added_line_count;
        existing.unstaged_removed_line_count += hunk.removed_line_count;
      }
      if (
        (existing.status === FILE_STAGE_STATUS.staged && !hunk.staged) ||
        (existing.status === FILE_STAGE_STATUS.unstaged && hunk.staged)
      ) {
        existing.status = FILE_STAGE_STATUS.partial;
      }
      continue;
    }
    grouped.set(hunk.file_path, {
      filePath: hunk.file_path,
      fileName: fileNameFromPath(hunk.file_path),
      changeKind: hunk.change_kind,
      snoozed: snoozedFiles.has(hunk.file_path),
      status: hunk.staged ? FILE_STAGE_STATUS.staged : FILE_STAGE_STATUS.unstaged,
      added_line_count: hunk.added_line_count,
      removed_line_count: hunk.removed_line_count,
      unstaged_added_line_count: hunk.staged ? 0 : hunk.added_line_count,
      unstaged_removed_line_count: hunk.staged ? 0 : hunk.removed_line_count,
      staged_added_line_count: hunk.staged ? hunk.added_line_count : 0,
      staged_removed_line_count: hunk.staged ? hunk.removed_line_count : 0,
    });
  }

  return [...grouped.values()];
}
