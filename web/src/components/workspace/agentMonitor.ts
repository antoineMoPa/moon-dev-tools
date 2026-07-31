import { COMMENT_DISPATCH_STATUS } from "../../types";
import type { CommentDispatchStatus, ReviewComment, SessionState } from "../../types";

export enum AgentRunStage {
  InProgress = "in_progress",
  Completed = "completed",
  NotStarted = "not_started",
}

export const mapStageToTitle = {
  [AgentRunStage.InProgress]: "in progress",
  [AgentRunStage.Completed]: "completed",
  [AgentRunStage.NotStarted]: "not started",
} satisfies Record<AgentRunStage, string>;

/// The stage each dispatch status belongs to in the monitor.
const mapDispatchStatusToStage = {
  [COMMENT_DISPATCH_STATUS.idle]: AgentRunStage.NotStarted,
  [COMMENT_DISPATCH_STATUS.batched]: AgentRunStage.NotStarted,
  [COMMENT_DISPATCH_STATUS.queued]: AgentRunStage.InProgress,
  [COMMENT_DISPATCH_STATUS.running]: AgentRunStage.InProgress,
  [COMMENT_DISPATCH_STATUS.canceled]: AgentRunStage.Completed,
  [COMMENT_DISPATCH_STATUS.completed]: AgentRunStage.Completed,
  [COMMENT_DISPATCH_STATUS.failed]: AgentRunStage.Completed,
} satisfies Record<CommentDispatchStatus, AgentRunStage>;

/// The label shown on a row's status pill.
const mapDispatchStatusToLabel = {
  [COMMENT_DISPATCH_STATUS.idle]: "not sent",
  [COMMENT_DISPATCH_STATUS.batched]: "batched",
  [COMMENT_DISPATCH_STATUS.queued]: "queued",
  [COMMENT_DISPATCH_STATUS.running]: "running",
  [COMMENT_DISPATCH_STATUS.canceled]: "stopped",
  [COMMENT_DISPATCH_STATUS.completed]: "done",
  [COMMENT_DISPATCH_STATUS.failed]: "failed",
} satisfies Record<CommentDispatchStatus, string>;

export const AGENT_RUN_STAGES: AgentRunStage[] = [
  AgentRunStage.InProgress,
  AgentRunStage.NotStarted,
  AgentRunStage.Completed,
];

export type AgentRun = {
  id: string;
  dispatchKey: string;
  hunkId: string;
  commentIndex: number;
  filePath: string;
  header: string;
  comment: string;
  selection: string;
  resolved: boolean;
  jumpable: boolean;
  status: CommentDispatchStatus;
  statusLabel: string;
  stage: AgentRunStage;
  agentLabel: string;
  detail: string;
  canStop: boolean;
  hasLog: boolean;
};

function agentLabel(data: SessionState, comment: ReviewComment): string {
  const option = data.available_agents.find((agent) => agent.kind === comment.dispatch.agent);
  return option ? option.label : comment.dispatch.agent;
}

export function buildAgentRun(data: SessionState, comment: ReviewComment, index: number): AgentRun {
  const { dispatch } = comment;

  return {
    id: `${comment.hunk_id}:${comment.comment_index}:${index}`,
    dispatchKey: dispatch.key,
    hunkId: comment.hunk_id,
    commentIndex: comment.comment_index,
    filePath: comment.file_path,
    header: comment.header,
    comment: comment.comment,
    selection: comment.selection,
    resolved: comment.resolved,
    jumpable: comment.jumpable,
    status: dispatch.status,
    statusLabel: mapDispatchStatusToLabel[dispatch.status],
    stage: mapDispatchStatusToStage[dispatch.status],
    agentLabel: agentLabel(data, comment),
    detail: dispatch.detail,
    canStop: dispatch.can_cancel,
    hasLog: dispatch.has_log,
  };
}

export function buildAgentRuns(data: SessionState): AgentRun[] {
  return data.review_comments.map((comment, index) => buildAgentRun(data, comment, index));
}

export function agentRunsForStage(runs: AgentRun[], stage: AgentRunStage): AgentRun[] {
  return runs.filter((run) => run.stage === stage);
}
