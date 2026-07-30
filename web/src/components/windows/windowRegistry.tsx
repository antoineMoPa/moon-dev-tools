import type { ReactNode } from "react";
import type { DockWindowId } from "./dockWindowState";
import { AgentMonitorWindow } from "./AgentMonitorWindow";
import { TerminalWindow } from "./TerminalWindow";

type DockWindowDefinition = {
  /// Shown on the dock tab and on the header button that opens the window.
  title: string;
  render: () => ReactNode;
};

export const mapWindowIdToDefinition = {
  terminal: {
    title: "terminal",
    render: () => <TerminalWindow />,
  },
  agents: {
    title: "agents",
    render: () => <AgentMonitorWindow />,
  },
} satisfies Record<DockWindowId, DockWindowDefinition>;

export const dockWindowIds = Object.keys(mapWindowIdToDefinition) as DockWindowId[];
