import { describe, expect, it } from "vitest";
import { buildSidebarFileGroups, orderMovedFilesAdjacently } from "./SidebarFilesSection";
import { buildSidebarFiles, FILE_STAGE_STATUS, localChangesSummary } from "./sidebarFiles";
import type { Hunk, SessionState } from "../types";

function makeHunk(overrides: Partial<Hunk>): Hunk {
  return {
    id: "hunk-1",
    file_path: "src/example.ts",
    change_kind: "modified",
    header: "@@ -1,1 +1,1 @@",
    staged: false,
    comment: "",
    comment_dispatches: [],
    patch_preview: "",
    patch_line_count: 1,
    added_line_count: 0,
    removed_line_count: 0,
    ...overrides,
  };
}

function makeSession(hunks: Hunk[]): SessionState {
  return {
    repo_name: "repo",
    branch_name: "main",
    commit_base: "origin/main",
    commits: [],
    history_commits: [],
    history_has_more: false,
    local_change_summary: { modified: 0, added: 0, deleted: 0 },
    repo_path: "/repo",
    read_only: false,
    patch_preview_line_limit: 200,
    available_agents: [],
    selected_agent: "none",
    hunks,
    review_comments: [],
    export_text: "",
  };
}

describe("buildSidebarFiles", () => {
  it("tracks staged and unstaged line counts separately for partial files", () => {
    const files = buildSidebarFiles(
      makeSession([
        makeHunk({
          id: "unstaged",
          staged: false,
          added_line_count: 3,
          removed_line_count: 1,
        }),
        makeHunk({
          id: "staged",
          staged: true,
          added_line_count: 10,
          removed_line_count: 4,
        }),
      ]),
      new Set(),
    );

    expect(files).toHaveLength(1);
    expect(files[0]).toMatchObject({
      status: FILE_STAGE_STATUS.partial,
      added_line_count: 13,
      removed_line_count: 5,
      unstaged_added_line_count: 3,
      unstaged_removed_line_count: 1,
      staged_added_line_count: 10,
      staged_removed_line_count: 4,
    });
  });

  it("preserves unstaged line counts for snoozed files", () => {
    const files = buildSidebarFiles(
      makeSession([
        makeHunk({
          id: "snoozed",
          file_path: "src/snoozed.ts",
          added_line_count: 2,
          removed_line_count: 6,
        }),
      ]),
      new Set(["src/snoozed.ts"]),
    );

    expect(files[0]).toMatchObject({
      snoozed: true,
      unstaged_added_line_count: 2,
      unstaged_removed_line_count: 6,
    });
  });

  it("marks whole-file moves from deleted and added files", () => {
    const files = buildSidebarFiles(
      makeSession([
        makeHunk({
          id: "old",
          file_path: "src/a.ts",
          change_kind: "deleted",
          removed_line_count: 12,
          moved_to: {
            target_hunk_id: "new",
            target_file_path: "src/b.ts",
            target_header: "@@ -0,0 +1,12 @@",
            score: 0.9,
          },
        }),
        makeHunk({
          id: "other",
          file_path: "src/other.ts",
          change_kind: "modified",
          added_line_count: 1,
        }),
        makeHunk({
          id: "new",
          file_path: "src/b.ts",
          change_kind: "added",
          added_line_count: 12,
          moved_from: {
            target_hunk_id: "old",
            target_file_path: "src/a.ts",
            target_header: "@@ -1,12 +0,0 @@",
            score: 0.9,
          },
        }),
      ]),
      new Set(),
    );

    expect(files.find((file) => file.filePath === "src/a.ts")).toMatchObject({
      movedToFilePath: "src/b.ts",
    });
    expect(files.find((file) => file.filePath === "src/b.ts")).toMatchObject({
      movedFromFilePath: "src/a.ts",
    });
    expect(orderMovedFilesAdjacently(files).map((file) => file.filePath)).toEqual([
      "src/a.ts",
      "src/b.ts",
      "src/other.ts",
    ]);
  });

  it("does not mark moved code blocks as whole-file moves", () => {
    const files = buildSidebarFiles(
      makeSession([
        makeHunk({
          id: "old",
          file_path: "src/a.ts",
          change_kind: "deleted",
          removed_line_count: 12,
          moved_to: {
            target_hunk_id: "new",
            target_file_path: "src/b.ts",
            target_header: "@@ -0,0 +1,12 @@",
            score: 0.9,
          },
        }),
        makeHunk({
          id: "new",
          file_path: "src/b.ts",
          change_kind: "added",
          added_line_count: 12,
          moved_from: {
            target_hunk_id: "old",
            target_file_path: "src/a.ts",
            target_header: "@@ -1,12 +0,0 @@",
            score: 0.9,
          },
        }),
        makeHunk({
          id: "new-extra",
          file_path: "src/b.ts",
          change_kind: "added",
          added_line_count: 1,
        }),
      ]),
      new Set(),
    );

    expect(files.find((file) => file.filePath === "src/a.ts")?.movedToFilePath).toBeUndefined();
    expect(files.find((file) => file.filePath === "src/b.ts")?.movedFromFilePath).toBeUndefined();
  });

  it("sums section stats from remaining unstaged code", () => {
    const files = buildSidebarFiles(
      makeSession([
        makeHunk({
          id: "partial-unstaged",
          file_path: "src/partial.ts",
          staged: false,
          added_line_count: 3,
          removed_line_count: 1,
        }),
        makeHunk({
          id: "partial-staged",
          file_path: "src/partial.ts",
          staged: true,
          added_line_count: 10,
          removed_line_count: 4,
        }),
        makeHunk({
          id: "snoozed",
          file_path: "src/snoozed.ts",
          staged: false,
          added_line_count: 2,
          removed_line_count: 6,
        }),
      ]),
      new Set(["src/snoozed.ts"]),
    );

    const groups = buildSidebarFileGroups(files);

    expect(groups.unstagedDiffStats).toEqual({ added: 3, removed: 1 });
    expect(groups.snoozedDiffStats).toEqual({ added: 2, removed: 6 });
    expect(groups.remainingUnstagedDiffStats).toEqual({ added: 5, removed: 7 });
    expect(groups.totalDiffStats).toEqual({ added: 15, removed: 11 });
  });

  it("keeps unstaged section stats separate from the full files total for a partial file", () => {
    const files = buildSidebarFiles(
      makeSession([
        makeHunk({
          id: "large-partial-unstaged",
          file_path: "src/large-change.ts",
          staged: false,
          added_line_count: 9,
          removed_line_count: 6,
        }),
        makeHunk({
          id: "large-partial-staged",
          file_path: "src/large-change.ts",
          staged: true,
          added_line_count: 331,
          removed_line_count: 19,
        }),
      ]),
      new Set(),
    );

    const groups = buildSidebarFileGroups(files);

    expect(groups.unstagedDiffStats).toEqual({ added: 9, removed: 6 });
    expect(groups.remainingUnstagedDiffStats).toEqual({ added: 9, removed: 6 });
    expect(groups.totalDiffStats).toEqual({ added: 340, removed: 25 });
  });
});

describe("localChangesSummary", () => {
  it("formats local working tree counts", () => {
    expect(localChangesSummary({ modified: 4, added: 1, deleted: 2 })).toBe(
      "4 modified files, 1 new file, 2 deleted files",
    );
  });

  it("describes an empty local working tree", () => {
    expect(localChangesSummary({ modified: 0, added: 0, deleted: 0 })).toBe("no unstaged changes");
  });
});
