import { describe, expect, it } from "vitest";
import { reconcileDraftComments } from "./draftCommentAnchoring";
import type { DraftComment, Hunk } from "./types";

function makeHunk(overrides: Partial<Hunk>): Hunk {
  return {
    id: "hunk-1",
    file_path: "src/example.ts",
    change_kind: "modified",
    header: "@@ -1,5 +10,5 @@",
    staged: false,
    comment: "",
    comment_dispatches: [],
    patch_preview: "@@ -1,5 +10,5 @@\n export class Example {\n   value() {}\n }\n",
    patch_line_count: 4,
    added_line_count: 0,
    removed_line_count: 0,
    ...overrides,
  };
}

function makeDraft(overrides: Partial<DraftComment> = {}): DraftComment {
  return {
    id: "draft-1",
    hunkId: "old-hunk",
    filePath: "src/example.ts",
    header: "@@ -10,8 +20,12 @@",
    selectedText: "export class Example {\n  value() {}\n}",
    note: "keep this draft",
    ...overrides,
  };
}

describe("reconcileDraftComments", () => {
  it("keeps drafts whose original hunk still exists", () => {
    const draft = makeDraft({ hunkId: "hunk-1" });
    const hunk = makeHunk({ id: "hunk-1" });

    expect(reconcileDraftComments([draft], [hunk])).toEqual([draft]);
  });

  it("reattaches multiple drafts independently", () => {
    const first = makeDraft({ id: "draft-1", selectedText: "export class Example {" });
    const second = makeDraft({
      id: "draft-2",
      filePath: "src/other.ts",
      selectedText: "function helper() {",
    });
    const matchingFirst = makeHunk({ id: "new-hunk-1" });
    const matchingSecond = makeHunk({
      id: "new-hunk-2",
      file_path: "src/other.ts",
      patch_preview: "@@ -2,4 +40,5 @@\n function helper() {\n   return true;\n }\n",
      header: "@@ -2,4 +40,5 @@",
    });

    expect(reconcileDraftComments([first, second], [matchingFirst, matchingSecond])).toEqual([
      { ...first, hunkId: "new-hunk-1", header: matchingFirst.header },
      { ...second, hunkId: "new-hunk-2", header: matchingSecond.header },
    ]);
  });

  it("uses line number hints to choose the right hunk in the same file", () => {
    const draft = makeDraft({
      lineNumberHint: 81,
      selectedText: "return status;",
    });
    const nearMatch = makeHunk({
      id: "near-hunk",
      header: "@@ -1,5 +80,6 @@",
      patch_preview: "@@ -1,5 +80,6 @@\n   return status;\n }\n",
    });
    const farMatch = makeHunk({
      id: "far-hunk",
      header: "@@ -1,5 +10,6 @@",
      patch_preview: "@@ -1,5 +10,6 @@\n   return status;\n }\n",
    });

    expect(reconcileDraftComments([draft], [farMatch, nearMatch])).toEqual([
      { ...draft, hunkId: "near-hunk", header: nearMatch.header },
    ]);
  });

  it("drops drafts when no plausible match remains", () => {
    const draft = makeDraft();
    const other = makeHunk({
      id: "other",
      file_path: "src/other.ts",
      patch_preview: "@@ -1,2 +1,2 @@\n const x = 1;\n",
    });

    expect(reconcileDraftComments([draft], [other])).toEqual([]);
  });
});
