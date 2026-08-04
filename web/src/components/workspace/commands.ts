import { findPaneOfKind, findReviewPane } from "./layout";
import type { AgentOption } from "../../types";
import type { OpenPaneRequest, TerminalCommand, WorkspaceLayout } from "./layout";

export type UiCommand = {
  id: string;
  title: string;
  description: string;
  request: OpenPaneRequest;
};

type CommandDefinition = Omit<UiCommand, "request"> & {
  request: (rootSessionId: string) => OpenPaneRequest;
  available: (layout: WorkspaceLayout, rootSessionId: string) => boolean;
};

const commandDefinitions: CommandDefinition[] = [
  {
    id: "open-review",
    title: "review",
    description: "Open the main review",
    request: (rootSessionId) => ({ kind: "review", sessionId: rootSessionId, title: "review" }),
    available: (layout, rootSessionId) => !findReviewPane(layout, rootSessionId),
  },
  {
    id: "open-agents",
    title: "comment agents",
    description: "Open the comment agent monitor",
    request: () => ({ kind: "agents" }),
    available: (layout) => !findPaneOfKind(layout, "agents"),
  },
  {
    id: "open-terminal",
    title: "terminal",
    description: "Open a new shell",
    request: () => ({ kind: "terminal" }),
    available: () => true,
  },
];

const agentCommandDefinitions = [
  {
    command: "opencode",
    title: "opencode",
    description: "Open OpenCode in a terminal",
  },
  {
    command: "claude",
    title: "claude",
    description: "Open Claude in a terminal",
  },
  {
    command: "codex",
    title: "codex",
    description: "Open Codex in a terminal",
  },
] satisfies Array<{ command: TerminalCommand; title: string; description: string }>;

/// Commands currently available to the workspace. Keeping their labels and requests here
/// lets compact menus and searchable palettes expose the same actions.
export function uiCommandsFor(
  layout: WorkspaceLayout,
  rootSessionId: string,
  agents: AgentOption[] = [],
): UiCommand[] {
  const availableAgentKinds = new Set(
    agents.filter((agent) => agent.available).map((agent) => agent.kind),
  );
  const paneCommands = commandDefinitions
    .filter((command) => command.available(layout, rootSessionId))
    .map((command) => ({
      id: command.id,
      title: command.title,
      description: command.description,
      request: command.request(rootSessionId),
    }));

  return [
    ...paneCommands,
    ...agentCommandDefinitions
      .filter((command) => availableAgentKinds.has(command.command))
      .map((command) => ({
        id: `open-${command.command}`,
        title: command.title,
        description: command.description,
        request: { kind: "terminal" as const, command: command.command },
      })),
  ];
}

export function filterUiCommands(commands: UiCommand[], query: string): UiCommand[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) {
    return commands;
  }

  return commands.filter((command) => {
    const searchable = `${command.title} ${command.description}`.toLowerCase();
    return terms.every((term) => searchable.includes(term));
  });
}
