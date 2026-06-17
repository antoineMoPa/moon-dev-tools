import type { CSSProperties } from "react";

type LineActionsProps = {
  onAddComment: () => void;
  onClose: () => void;
  onStageLines?: () => void;
  style?: CSSProperties;
};

export function LineActions({ onAddComment, onClose, onStageLines, style }: LineActionsProps) {
  return (
    <div className="line-actions" style={style}>
      <button className="primary" onClick={onAddComment}>
        Add Comment
      </button>
      {onStageLines ? <button onClick={onStageLines}>Stage Lines</button> : null}
      <button type="button" className="popup-close-button" onClick={onClose} aria-label="Close popup">
        X
      </button>
    </div>
  );
}
