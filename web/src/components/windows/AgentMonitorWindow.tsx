import { useEffect, useMemo, useState } from "react";
import { fetchAgentLog } from "../../api";
import { parseAnsiSpans } from "./ansi";
import { useReviewStore } from "../../reviewStore";
import {
  AGENT_RUN_STAGES,
  AgentRunStage,
  agentRunsForStage,
  buildAgentRuns,
  mapStageToTitle,
} from "./agentMonitor";
import type { AgentRun } from "./agentMonitor";

const LOG_POLL_INTERVAL_MS = 1500;

function fileNameFromPath(filePath: string) {
  const segments = filePath.split("/");
  return segments[segments.length - 1] || filePath;
}

function AgentRunLog({ text }: { text: string }) {
  const spans = useMemo(() => parseAnsiSpans(text), [text]);

  if (spans.length === 0) {
    return <pre className="agent-run-log">No output yet.</pre>;
  }

  return (
    <pre className="agent-run-log">
      {spans.map((span, index) => (
        <span key={index} style={span.style}>
          {span.text}
        </span>
      ))}
    </pre>
  );
}

type AgentRunRowProps = {
  run: AgentRun;
  logOpen: boolean;
  logText: string;
  onToggleLog: () => void;
  onStop: () => void;
};

function AgentRunRow({ run, logOpen, logText, onToggleLog, onStop }: AgentRunRowProps) {
  return (
    <div className="agent-run">
      <div className="agent-run-topline">
        <span className={`agent-run-status agent-run-status-${run.status}`}>{run.statusLabel}</span>
        <span className="agent-run-agent">{run.agentLabel}</span>
        <span className="agent-run-file" title={`${run.filePath} ${run.header}`}>
          {fileNameFromPath(run.filePath)}
        </span>
        <div className="agent-run-actions">
          {run.hasLog ? (
            <button type="button" onClick={onToggleLog}>
              [{logOpen ? "hide work" : "work"}]
            </button>
          ) : null}
          {run.canStop ? (
            <button type="button" onClick={onStop}>
              [stop]
            </button>
          ) : null}
        </div>
      </div>
      <p className="agent-run-comment">{run.comment}</p>
      {run.detail ? <p className="agent-run-detail muted">{run.detail}</p> : null}
      {logOpen ? <AgentRunLog text={logText} /> : null}
    </div>
  );
}

export function AgentMonitorWindow() {
  const {
    state: { data },
    actions,
  } = useReviewStore();
  const [openLogKey, setOpenLogKey] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const runs = useMemo(() => (data ? buildAgentRuns(data) : []), [data]);
  const openLogRun = runs.find((run) => run.dispatchKey === openLogKey) ?? null;
  const openLogIsLive = openLogRun?.stage === AgentRunStage.InProgress;

  // Follow the open run's output while it is still working.
  useEffect(() => {
    if (!openLogKey) {
      setLogText("");
      return;
    }

    let canceled = false;

    async function loadLog() {
      try {
        const payload = await fetchAgentLog(openLogKey!);
        if (!canceled) {
          setLogText(payload.text);
        }
      } catch (error) {
        if (!canceled) {
          setLogText(error instanceof Error ? error.message : "Failed to load agent output.");
        }
      }
    }

    void loadLog();
    if (!openLogIsLive) {
      return () => {
        canceled = true;
      };
    }

    const timer = window.setInterval(() => void loadLog(), LOG_POLL_INTERVAL_MS);
    return () => {
      canceled = true;
      window.clearInterval(timer);
    };
  }, [openLogIsLive, openLogKey]);

  if (!data) {
    return <div className="agent-monitor empty-section muted">Loading review state...</div>;
  }

  if (runs.length === 0) {
    return <div className="agent-monitor empty-section muted">No comments yet.</div>;
  }

  return (
    <div className="agent-monitor">
      {AGENT_RUN_STAGES.map((stage) => {
        const stageRuns = agentRunsForStage(runs, stage);
        if (stageRuns.length === 0) {
          return null;
        }

        return (
          <section key={stage} className="agent-monitor-stage">
            <div className="agent-monitor-stage-head">
              <p>
                {mapStageToTitle[stage]} <span className="muted">({stageRuns.length})</span>
              </p>
            </div>
            {stageRuns.map((run) => (
              <AgentRunRow
                key={run.id}
                run={run}
                logOpen={run.dispatchKey === openLogKey}
                logText={logText}
                onToggleLog={() =>
                  setOpenLogKey((current) => (current === run.dispatchKey ? null : run.dispatchKey))
                }
                onStop={() => void actions.cancelCommentDispatch(run.hunkId, run.commentIndex)}
              />
            ))}
          </section>
        );
      })}
    </div>
  );
}
