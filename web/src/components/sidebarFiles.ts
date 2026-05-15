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
  hunk_count: number;
  reviewed_hunk_count: number;
  reviewed: boolean;
  movedFromFilePath?: string;
  movedToFilePath?: string;
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
      existing.hunk_count += 1;
      if (hunk.reviewed) {
        existing.reviewed_hunk_count += 1;
      }
      existing.reviewed = existing.reviewed_hunk_count === existing.hunk_count;
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
      hunk_count: 1,
      reviewed_hunk_count: hunk.reviewed ? 1 : 0,
      reviewed: hunk.reviewed,
    });
  }

  const files = [...grouped.values()];
  annotateMovedFiles(files, data);
  return files;
}

function annotateMovedFiles(files: SidebarFileItem[], data: SessionState) {
  const fileByPath = new Map(files.map((file) => [file.filePath, file]));
  const hunksByFilePath = new Map<string, SessionState["hunks"]>();
  const moveCounts = new Map<string, number>();

  for (const hunk of data.hunks) {
    const fileHunks = hunksByFilePath.get(hunk.file_path);
    if (fileHunks) {
      fileHunks.push(hunk);
    } else {
      hunksByFilePath.set(hunk.file_path, [hunk]);
    }
  }

  for (const hunk of data.hunks) {
    const targetFilePath = hunk.moved_to?.target_file_path;
    if (!targetFilePath || hunk.file_path === targetFilePath) {
      continue;
    }

    const sourceFile = fileByPath.get(hunk.file_path);
    const targetFile = fileByPath.get(targetFilePath);
    if (sourceFile?.changeKind !== "deleted" || targetFile?.changeKind !== "added") {
      continue;
    }

    const key = `${hunk.file_path}\0${targetFilePath}`;
    moveCounts.set(key, (moveCounts.get(key) ?? 0) + 1);
  }

  const movePairs = [...moveCounts.entries()].sort((left, right) => right[1] - left[1]);
  for (const [key] of movePairs) {
    const [sourceFilePath, targetFilePath] = key.split("\0");
    const sourceFile = fileByPath.get(sourceFilePath);
    const targetFile = fileByPath.get(targetFilePath);
    if (!sourceFile || !targetFile || sourceFile.movedToFilePath || targetFile.movedFromFilePath) {
      continue;
    }
    const sourceHunks = hunksByFilePath.get(sourceFilePath) ?? [];
    const targetHunks = hunksByFilePath.get(targetFilePath) ?? [];
    if (!isWholeFileMove(sourceFilePath, targetFilePath, sourceFile, targetFile, sourceHunks, targetHunks)) {
      continue;
    }

    sourceFile.movedToFilePath = targetFilePath;
    targetFile.movedFromFilePath = sourceFilePath;
  }
}

function isWholeFileMove(
  sourceFilePath: string,
  targetFilePath: string,
  sourceFile: SidebarFileItem,
  targetFile: SidebarFileItem,
  sourceHunks: SessionState["hunks"],
  targetHunks: SessionState["hunks"],
) {
  if (
    sourceHunks.length === 0 ||
    targetHunks.length === 0 ||
    sourceFile.added_line_count > 0 ||
    targetFile.removed_line_count > 0 ||
    sourceFile.removed_line_count !== targetFile.added_line_count
  ) {
    return false;
  }

  return (
    sourceHunks.every((hunk) => hunk.moved_to?.target_file_path === targetFilePath) &&
    targetHunks.every((hunk) => hunk.moved_from?.target_file_path === sourceFilePath)
  );
}
