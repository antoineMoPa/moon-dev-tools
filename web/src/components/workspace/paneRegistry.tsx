import type { ReactNode } from "react";
import { AgentMonitorPane } from "./AgentMonitorPane";
import { ReviewPane } from "./ReviewPane";
import { TerminalPane } from "./TerminalPane";
import type { Pane, PaneKind } from "./layout";

type PaneDefinition<Kind extends PaneKind> = {
  /// Shown on the tab and on the header button that opens the pane.
  title: string;
  render: (pane: Extract<Pane, { kind: Kind }>) => ReactNode;
};

export const mapPaneKindToDefinition: { [Kind in PaneKind]: PaneDefinition<Kind> } = {
  review: {
    title: "review",
    render: () => <ReviewPane />,
  },
  agents: {
    title: "agents",
    render: () => <AgentMonitorPane />,
  },
  terminal: {
    title: "terminal",
    render: (pane) => <TerminalPane paneId={pane.paneId} terminalId={pane.terminalId} />,
  },
};

export const paneKinds = Object.keys(mapPaneKindToDefinition) as PaneKind[];

export function renderPane(pane: Pane): ReactNode {
  const definition = mapPaneKindToDefinition[pane.kind] as PaneDefinition<PaneKind> & {
    render: (value: Pane) => ReactNode;
  };
  return definition.render(pane);
}
