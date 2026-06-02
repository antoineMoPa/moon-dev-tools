import { COMMENT_DISPATCH_STATUS } from "../../types";
import { AgentSelect } from "../AgentSelect";
import { useHunkCommentContext, type InlineCommentSegment } from "./HunkCommentContext";

type InlineCommentCardProps = {
  id?: string;
  segment: InlineCommentSegment;
};

const HIDDEN_DISPATCH_STATUS = COMMENT_DISPATCH_STATUS.idle;

export function InlineCommentCard({ id, segment }: InlineCommentCardProps) {
  const {
    agents,
    selectedAgent,
    onAgentChange,
    getDispatch,
    editingCommentIndex,
    editingCommentValue,
    onToggleResolved,
    onStartEditing,
    onSave,
    onDelete,
    onCancelDispatch,
    onEditingCommentValueChange,
  } = useHunkCommentContext();
  const dispatch = getDispatch(segment.index);
  const editing = editingCommentIndex === segment.index;
  const resolvedClassName = segment.resolved ? "resolved" : "";
  const showDispatch = dispatch && dispatch.status !== HIDDEN_DISPATCH_STATUS;
  const canStopDispatch = dispatch?.can_cancel ?? false;

  return (
    <div id={id} className="inline-comment-card">
      <div className="inline-comment-head">
        <div className={`inline-comment-label ${resolvedClassName}`.trim()}>
          {segment.resolved ? "Resolved" : "Comment"}
        </div>
        <div className="toolbar">
          <button onClick={() => onToggleResolved(segment.index)}>
            {segment.resolved ? "Reopen" : "Resolve"}
          </button>
          {editing ? (
            <button onClick={() => onSave(segment.index)}>Save</button>
          ) : (
            <button onClick={() => onStartEditing(segment.index)}>Edit</button>
          )}
          {canStopDispatch ? (
            <button
              type="button"
              aria-label={`Stop ${dispatch?.status ?? "active"} agent dispatch`}
              onClick={() => onCancelDispatch(segment.index)}
            >
              Stop agent
            </button>
          ) : (
            <button onClick={() => onDelete(segment.index)}>Delete comment</button>
          )}
        </div>
      </div>
      {showDispatch ? (
        <div className={`dispatch-status dispatch-status-${dispatch.status}`.trim()}>
          <strong>{dispatch.status}</strong>
          {dispatch.detail ? <span>{dispatch.detail}</span> : null}
        </div>
      ) : null}
      <pre className="selection-preview">{segment.selection}</pre>
      {editing ? (
        <>
          <AgentSelect
            agents={agents}
            selectedAgent={selectedAgent}
            onAgentChange={onAgentChange}
            className="agent-picker agent-picker-inline"
          />
          <textarea
            value={editingCommentValue}
            onChange={(event) => onEditingCommentValueChange(event.target.value)}
            spellCheck={false}
          />
        </>
      ) : (
        <div className={`inline-comment-body ${resolvedClassName}`.trim()}>{segment.comment}</div>
      )}
    </div>
  );
}
