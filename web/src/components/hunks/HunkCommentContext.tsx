import { createContext, useContext } from "react";
import type { CSSProperties, Dispatch, SetStateAction } from "react";
import type { AgentKind, AgentOption, CommentDispatch, DraftComment } from "../../types";
import type { DiffSegment } from "./diffSegments";

type InlineCommentSegment = Extract<DiffSegment, { type: "comment" }>;

export type HunkCommentContextValue = {
  agents: AgentOption[];
  selectedAgent: AgentKind;
  onAgentChange: (agent: AgentKind) => void;
  getDispatch: (index: number) => CommentDispatch | undefined;
  editingCommentIndex: number | null;
  editingCommentValue: string;
  onToggleResolved: (index: number) => void;
  onStartEditing: (index: number) => void;
  onSave: (index: number) => void;
  onDelete: (index: number) => void;
  onCancelDispatch: (index: number) => void;
  onEditingCommentValueChange: Dispatch<SetStateAction<string>>;
  getDraft: (draftId: string) => DraftComment | null;
  onDraftNoteChange: (draftId: string, value: string) => void;
  batchDraftComments: boolean;
  onBatchDraftCommentsChange: (value: boolean) => void;
  onDraftAdd: (draftId: string) => void;
  onDraftClear: (draftId: string) => void;
};

const HunkCommentContext = createContext<HunkCommentContextValue | null>(null);

export function HunkCommentContextProvider({
  value,
  children,
}: {
  value: HunkCommentContextValue;
  children: React.ReactNode;
}) {
  return <HunkCommentContext.Provider value={value}>{children}</HunkCommentContext.Provider>;
}

export function useHunkCommentContext(): HunkCommentContextValue {
  const value = useContext(HunkCommentContext);
  if (!value) {
    throw new Error("useHunkCommentContext must be used within HunkCommentContextProvider");
  }
  return value;
}

export type { InlineCommentSegment, CSSProperties };
