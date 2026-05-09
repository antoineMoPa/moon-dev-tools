import { describe, expect, it } from "vitest";
import { buildSidebarFileGroups } from "./SidebarFilesSection";
import { buildSidebarFiles, FILE_STAGE_STATUS } from "./sidebarFiles";
import type { Hunk, SessionState } from "../types";

function makeHunk(overrides: Partial<Hunk>): Hunk {
  return {
    id: "hunk-1",
    file_path: "src/example.ts",
    change_kind: "modified",
    header: "@@ -1,1 +1,1 @@",
    staged: false,
    reviewed: false,
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
    repo_path: "/repo",
    read_only: false,
    patch_preview_line_limit: 200,
    available_agents: [],
    selected_agent: "none",
    hunks,
    sidebar_comments: [],
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
