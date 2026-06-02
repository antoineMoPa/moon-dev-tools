import { useEffect, useMemo, useState } from "react";
import {
  buildAnchoredCommentValue,
  parseAnchoredComments,
  type AnchoredComment,
} from "../../anchoredComments";
import { useReviewStore } from "../../reviewStore";
import type { AgentKind, AgentOption, DraftComment, Hunk } from "../../types";
import { splitDiffIntoSegments } from "./diffSegments";

type FloatingPosition = {
  top: number;
  left: number;
};

type UseHunkCommentsArgs = {
  hunk: Hunk;
  visiblePatch: string;
  commentValue: string;
  setCommentValue: (value: string) => void;
  agents: AgentOption[];
  selectedAgent: AgentKind;
  onAgentChange: (agent: AgentKind) => void;
  clearSelectionUi: () => void;
  setSelectedText: (value: string) => void;
  setComposerOpen: (value: boolean) => void;
  setLockedSelectionPosition: (value: FloatingPosition | null) => void;
};

export function useHunkComments({
  hunk,
  visiblePatch,
  commentValue,
  setCommentValue,
  agents,
  selectedAgent,
  onAgentChange,
  clearSelectionUi,
  setSelectedText,
  setComposerOpen,
  setLockedSelectionPosition,
}: UseHunkCommentsArgs) {
  const {
    state: { batchDraftComments, draftComments },
    actions,
  } = useReviewStore();
  const [activeDraftId, setActiveDraftId] = useState<string | null>(null);
  const [editingCommentIndex, setEditingCommentIndex] = useState<number | null>(null);
  const [editingCommentValue, setEditingCommentValue] = useState("");

  const parsedComments = useMemo(() => parseAnchoredComments(commentValue), [commentValue]);
  const attachedDrafts = useMemo(
    () => draftComments.filter((draft) => draft.hunkId === hunk.id),
    [draftComments, hunk.id],
  );
  const activeDraft = attachedDrafts.find((draft) => draft.id === activeDraftId) ?? null;
  const inlineDrafts = useMemo(
    () => attachedDrafts.filter((draft) => draft.id !== activeDraftId),
    [activeDraftId, attachedDrafts],
  );
  const diffSegments = useMemo(
    () => splitDiffIntoSegments(visiblePatch, parsedComments, inlineDrafts),
    [inlineDrafts, parsedComments, visiblePatch],
  );

  useEffect(() => {
    if (activeDraftId && !attachedDrafts.some((draft) => draft.id === activeDraftId)) {
      setActiveDraftId(null);
      setLockedSelectionPosition(null);
    }
  }, [activeDraftId, attachedDrafts, setLockedSelectionPosition]);

  function createDraftId() {
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
      return crypto.randomUUID();
    }
    return `draft-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  }

  function getAttachedDraft(draftId: string) {
    return attachedDrafts.find((draft) => draft.id === draftId) ?? null;
  }

  function openSelectionDraft(selection: string, anchorPosition?: FloatingPosition, lineNumberHint?: number) {
    const nextSelection = selection.trim();
    if (!nextSelection) {
      return;
    }

    const existingDraft = attachedDrafts.find(
      (draft) => draft.selectedText === nextSelection && draft.lineNumberHint === lineNumberHint,
    );
    const draft: DraftComment = existingDraft ?? {
      id: createDraftId(),
      hunkId: hunk.id,
      filePath: hunk.file_path,
      header: hunk.header,
      selectedText: nextSelection,
      note: "",
      lineNumberHint,
    };

    actions.upsertDraftComment({
      ...draft,
      hunkId: hunk.id,
      filePath: hunk.file_path,
      header: hunk.header,
      selectedText: nextSelection,
      lineNumberHint,
    });
    setActiveDraftId(draft.id);
    setLockedSelectionPosition(anchorPosition ?? null);
    setComposerOpen(true);
  }

  function closeSelectionComposer(draft: DraftComment) {
    if (draft.note.trim() && !window.confirm("Discard this comment?")) {
      return;
    }

    actions.removeDraftComment(draft.id);
    if (draft.id === activeDraftId) {
      setActiveDraftId(null);
    }
    clearSelectionUi();
  }

  function addAnchoredComment(draft: DraftComment) {
    const activeSelectedText = draft.selectedText;
    const note = draft.note;
    if (!activeSelectedText.trim() || !note.trim()) {
      return;
    }

    const next = buildAnchoredCommentValue([
      { selection: activeSelectedText, comment: note, resolved: false },
      ...parsedComments,
    ]).trim();

    setCommentValue(next);
    actions.updateDraftComment(hunk.id, next);
    actions.removeDraftComment(draft.id);
    if (draft.id === activeDraftId) {
      setActiveDraftId(null);
    }
    setSelectedText("");
    setComposerOpen(false);
    void actions.saveComment(hunk.id, next, batchDraftComments);
  }

  function persistAnchoredComments(nextAnchored: AnchoredComment[]) {
    const next = buildAnchoredCommentValue(nextAnchored).trim();
    setCommentValue(next);
    actions.updateDraftComment(hunk.id, next);
    void actions.saveComment(hunk.id, next);
  }

  function startEditingComment(index: number) {
    setEditingCommentIndex(index);
    setEditingCommentValue(parsedComments[index]?.comment ?? "");
  }

  function saveEditedComment(index: number) {
    const nextAnchored = parsedComments.map((entry, entryIndex) =>
      entryIndex === index ? { ...entry, comment: editingCommentValue } : entry,
    );
    persistAnchoredComments(nextAnchored);
    setEditingCommentIndex(null);
    setEditingCommentValue("");
  }

  function deleteComment(index: number) {
    const nextAnchored = parsedComments.filter((_, entryIndex) => entryIndex !== index);
    persistAnchoredComments(nextAnchored);
    setEditingCommentIndex(null);
    setEditingCommentValue("");
  }

  function toggleCommentResolved(index: number) {
    const nextAnchored = parsedComments.map((entry, entryIndex) =>
      entryIndex === index ? { ...entry, resolved: !entry.resolved } : entry,
    );
    persistAnchoredComments(nextAnchored);
  }

  function cancelCommentDispatch(index: number) {
    void actions.cancelCommentDispatch(hunk.id, index);
  }

  const commentContextValue = useMemo(
    () => ({
      agents,
      selectedAgent,
      onAgentChange,
      getDispatch: (index: number) => hunk.comment_dispatches[index],
      editingCommentIndex,
      editingCommentValue,
      onToggleResolved: toggleCommentResolved,
      onStartEditing: startEditingComment,
      onSave: saveEditedComment,
      onDelete: deleteComment,
      onCancelDispatch: cancelCommentDispatch,
      onEditingCommentValueChange: setEditingCommentValue,
      getDraft: getAttachedDraft,
      onDraftNoteChange: (draftId: string, value: string) => {
        const draft = getAttachedDraft(draftId);
        if (!draft) {
          return;
        }
        actions.upsertDraftComment({
          ...draft,
          note: value,
          hunkId: hunk.id,
          filePath: hunk.file_path,
          header: hunk.header,
        });
      },
      batchDraftComments,
      onBatchDraftCommentsChange: actions.setBatchDraftComments,
      onDraftAdd: (draftId: string) => {
        const draft = getAttachedDraft(draftId);
        if (draft) {
          addAnchoredComment(draft);
        }
      },
      onDraftClear: (draftId: string) => {
        const draft = getAttachedDraft(draftId);
        if (draft) {
          closeSelectionComposer(draft);
        }
      },
    }),
    [
      actions,
      agents,
      deleteComment,
      editingCommentIndex,
      editingCommentValue,
      batchDraftComments,
      hunk.comment_dispatches,
      hunk.id,
      hunk.file_path,
      hunk.header,
      onAgentChange,
      saveEditedComment,
      selectedAgent,
      startEditingComment,
      toggleCommentResolved,
    ],
  );

  return {
    activeDraft,
    diffSegments,
    openSelectionDraft,
    commentContextValue,
  };
}
