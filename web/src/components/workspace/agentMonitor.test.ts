import { describe, expect, it } from "vitest";
import { AgentRunStage, agentRunsForStage, buildAgentRuns } from "./agentMonitor";
import { COMMENT_DISPATCH_STATUS } from "../../types";
import type { CommentDispatchStatus, ReviewComment, SessionState } from "../../types";

function makeReviewComment(
  status: CommentDispatchStatus,
  overrides: Partial<ReviewComment> = {},
): ReviewComment {
  return {
    hunk_id: `hunk-${status}`,
    comment_index: 0,
    file_path: "src/example.ts",
    header: "@@ -1,1 +1,1 @@",
    selection: "const a = 1;",
    comment: `comment for ${status}`,
    resolved: false,
    jumpable: true,
    dispatch: {
      key: `hunk-${status}:key`,
      status,
      detail: "detail",
      agent: "claude",
      can_cancel:
        status === COMMENT_DISPATCH_STATUS.queued || status === COMMENT_DISPATCH_STATUS.running,
      has_log: status === COMMENT_DISPATCH_STATUS.running,
    },
    ...overrides,
  };
}

function makeSession(reviewComments: ReviewComment[]): SessionState {
  return {
    repo_name: "repo",
    branch_name: "main",
    commit_base: "origin/main",
    commits: [],
    history_commits: [],
    history_has_more: false,
    local_change_summary: { modified: 0, added: 0, deleted: 0 },
    repo_path: "/repo",
    read_only: false,
    patch_preview_line_limit: 200,
    available_agents: [{ kind: "claude", label: "Claude", available: true }],
    selected_agent: "claude",
    hunks: [],
    review_comments: reviewComments,
    export_text: "",
  };
}

describe("buildAgentRuns", () => {
  it("groups every dispatch status into a monitor stage", () => {
    const session = makeSession([
      makeReviewComment(COMMENT_DISPATCH_STATUS.running),
      makeReviewComment(COMMENT_DISPATCH_STATUS.queued),
      makeReviewComment(COMMENT_DISPATCH_STATUS.idle),
      makeReviewComment(COMMENT_DISPATCH_STATUS.batched),
      makeReviewComment(COMMENT_DISPATCH_STATUS.completed),
      makeReviewComment(COMMENT_DISPATCH_STATUS.failed),
      makeReviewComment(COMMENT_DISPATCH_STATUS.canceled),
    ]);

    const runs = buildAgentRuns(session);

    expect(agentRunsForStage(runs, AgentRunStage.InProgress).map((run) => run.status)).toEqual([
      "running",
      "queued",
    ]);
    expect(agentRunsForStage(runs, AgentRunStage.NotStarted).map((run) => run.status)).toEqual([
      "idle",
      "batched",
    ]);
    expect(agentRunsForStage(runs, AgentRunStage.Completed).map((run) => run.status)).toEqual([
      "completed",
      "failed",
      "canceled",
    ]);
  });

  it("carries the agent label and the stop and log affordances of each run", () => {
    const session = makeSession([
      makeReviewComment(COMMENT_DISPATCH_STATUS.running),
      makeReviewComment(COMMENT_DISPATCH_STATUS.idle),
    ]);

    const [running, idle] = buildAgentRuns(session);

    expect(running).toMatchObject({
      agentLabel: "Claude",
      statusLabel: "running",
      canStop: true,
      hasLog: true,
      dispatchKey: "hunk-running:key",
    });
    expect(idle).toMatchObject({ statusLabel: "not sent", canStop: false, hasLog: false });
  });

  it("keeps a run addressable by hunk and comment index so it can be stopped", () => {
    const session = makeSession([
      makeReviewComment(COMMENT_DISPATCH_STATUS.running, { hunk_id: "abc", comment_index: 2 }),
    ]);

    expect(buildAgentRuns(session)[0]).toMatchObject({ hunkId: "abc", commentIndex: 2 });
  });
});
