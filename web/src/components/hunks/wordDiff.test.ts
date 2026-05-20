import { describe, expect, it } from "vitest";
import { wordDiffParts } from "./wordDiff";

describe("wordDiffParts", () => {
  it("marks inserted characters in the new line", () => {
    const { oldParts, newParts } = wordDiffParts("ABD", "ABCD");

    expect(oldParts).toEqual([{ text: "ABD", changed: false }]);
    expect(newParts).toEqual([
      { text: "AB", changed: false },
      { text: "C", changed: true },
      { text: "D", changed: false },
    ]);
  });

  it("keeps whitespace out of changed spans", () => {
    const { newParts } = wordDiffParts("one two", "one  two");

    expect(newParts).toEqual([
      { text: "one  two", changed: false },
    ]);
  });
});
