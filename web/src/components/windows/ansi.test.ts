import { describe, expect, it } from "vitest";
import { parseAnsiSpans } from "./ansi";

const ESC = "\x1b";

describe("parseAnsiSpans", () => {
  it("keeps plain output as a single unstyled span", () => {
    expect(parseAnsiSpans("build finished\n")).toEqual([{ text: "build finished\n", style: {} }]);
  });

  it("colours text between an SGR colour and its reset", () => {
    const spans = parseAnsiSpans(`before ${ESC}[32mgreen${ESC}[0m after`);

    expect(spans.map((span) => span.text)).toEqual(["before ", "green", " after"]);
    expect(spans[1].style.color).toBe("#7ee787");
    expect(spans[2].style).toEqual({});
  });

  it("reads bold, dim and bright colours", () => {
    const spans = parseAnsiSpans(`${ESC}[1;91mloud${ESC}[22m${ESC}[2mquiet`);

    expect(spans[0].style).toEqual({ fontWeight: "bold", color: "#ffa198" });
    expect(spans[1].style).toEqual({ color: "#ffa198", opacity: 0.72 });
  });

  it("reads 256-colour and truecolour parameters", () => {
    expect(parseAnsiSpans(`${ESC}[38;5;208morange`)[0].style.color).toBe("rgb(255, 135, 0)");
    expect(parseAnsiSpans(`${ESC}[38;2;12;34;56mexact`)[0].style.color).toBe("rgb(12, 34, 56)");
  });

  it("drops sequences that only move the cursor or set the window title", () => {
    const spans = parseAnsiSpans(`${ESC}[2K${ESC}[1A${ESC}]0;title${ESC}\\done${ESC}[?25h`);

    expect(spans).toEqual([{ text: "done", style: {} }]);
  });

  it("drops carriage returns and other control characters", () => {
    expect(parseAnsiSpans("one\r\ntwo\rthree")).toEqual([{ text: "one\ntwothree", style: {} }]);
  });

  it("reproduces the escape soup an agent prints around its status lines", () => {
    const spans = parseAnsiSpans(
      `${ESC}[0m\n> build · gpt-5.6-sol\n${ESC}[0m\nI will inspect the run stage type.\n${ESC}[0m→`,
    );

    expect(spans.map((span) => span.text).join("")).toBe(
      "\n> build · gpt-5.6-sol\n\nI will inspect the run stage type.\n→",
    );
  });
});
