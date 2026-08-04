import { describe, expect, it } from "vitest";
import { filterUiCommands, uiCommandsFor } from "./commands";
import { addPane, defaultLayout, emptyLayout } from "./layout";

describe("uiCommandsFor", () => {
  it("only offers panes that can currently be opened", () => {
    const rootSessionId = "root";
    const withReview = defaultLayout(rootSessionId);

    expect(uiCommandsFor(withReview, rootSessionId).map((command) => command.title)).toEqual([
      "comment agents",
      "terminal",
    ]);

    const withAgents = addPane(withReview, withReview.activeFrameId, {
      paneId: "agents-pane",
      kind: "agents",
    });
    expect(uiCommandsFor(withAgents, rootSessionId).map((command) => command.title)).toEqual([
      "terminal",
    ]);
  });

  it("offers the main review when it is closed", () => {
    expect(uiCommandsFor(emptyLayout(), "root").map((command) => command.title)).toEqual([
      "review",
      "comment agents",
      "terminal",
    ]);
  });

  it("offers direct terminals only for installed agents", () => {
    const commands = uiCommandsFor(emptyLayout(), "root", [
      { kind: "none", label: "None", available: true },
      { kind: "claude", label: "Claude", available: false },
      { kind: "codex", label: "Codex", available: true },
      { kind: "opencode", label: "OpenCode", available: true },
    ]);

    expect(commands.slice(-2)).toEqual([
      {
        id: "open-opencode",
        title: "opencode",
        description: "Open OpenCode in a terminal",
        request: { kind: "terminal", command: "opencode" },
      },
      {
        id: "open-codex",
        title: "codex",
        description: "Open Codex in a terminal",
        request: { kind: "terminal", command: "codex" },
      },
    ]);
  });
});

describe("filterUiCommands", () => {
  const commands = uiCommandsFor(emptyLayout(), "root");

  it("matches command descriptions", () => {
    expect(filterUiCommands(commands, "sh").map((command) => command.title)).toEqual(["terminal"]);
  });

  it("matches case-insensitive terms across the searchable text", () => {
    expect(filterUiCommands(commands, "OPEN AGENT").map((command) => command.title)).toEqual([
      "comment agents",
    ]);
  });

  it("returns all commands for a blank query and none for an unknown query", () => {
    expect(filterUiCommands(commands, "  ")).toEqual(commands);
    expect(filterUiCommands(commands, "missing")).toEqual([]);
  });
});
