import type { ReactNode } from "react";
import { AgentMonitorPane } from "./AgentMonitorPane";
import { ReviewPane } from "./ReviewPane";
import { TerminalPane } from "./TerminalPane";
import type { Pane, PaneKind } from "./layout";
import type { SessionState } from "../../types";

type PaneDefinition<Kind extends PaneKind> = {
  /// Shown on the tab. A review names the repo it shows, which is only known once its
  /// state has loaded; until then it falls back to the name it was opened under.
  title: (pane: Extract<Pane, { kind: Kind }>, reviewData: SessionState | null) => string;
  render: (pane: Extract<Pane, { kind: Kind }>) => ReactNode;
};

function repoLabel(data: SessionState): string {
  return data.branch_name ? `${data.repo_name} / ${data.branch_name}` : data.repo_name;
}

const mapPaneKindToDefinition: { [Kind in PaneKind]: PaneDefinition<Kind> } = {
  review: {
    title: (pane, reviewData) => (reviewData ? repoLabel(reviewData) : pane.title),
    render: (pane) => <ReviewPane sessionId={pane.sessionId} />,
  },
  agents: {
    title: () => "comment agents",
    render: () => <AgentMonitorPane />,
  },
  terminal: {
    title: (pane) => pane.command ?? "terminal",
    render: (pane) => (
      <TerminalPane paneId={pane.paneId} terminalId={pane.terminalId} command={pane.command} />
    ),
  },
};

function definitionFor(pane: Pane) {
  return mapPaneKindToDefinition[pane.kind] as PaneDefinition<PaneKind> & {
    title: (value: Pane, reviewData: SessionState | null) => string;
    render: (value: Pane) => ReactNode;
  };
}

export function paneTitle(pane: Pane, reviewData: SessionState | null = null): string {
  return definitionFor(pane).title(pane, reviewData);
}

export function renderPane(pane: Pane): ReactNode {
  return definitionFor(pane).render(pane);
}
