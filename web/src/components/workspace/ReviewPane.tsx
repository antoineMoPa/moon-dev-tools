import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { Footer } from "../Footer";
import { LeftSidebar } from "../LeftSidebar";
import { Hunks } from "../hunks/Hunks";
import { ReviewScrollProvider } from "./reviewScroll";
import type { ReviewScrollValue } from "./reviewScroll";
import { ReviewStoreProvider, ReviewView, useReviewStore } from "../../reviewStore";
import { useWorkspace } from "./workspaceState";
import { getRootSessionId } from "../../api";
import { filePathsInListOrder, fullyStagedFilePaths, hasUnstagedHunks } from "../../sessionStateUtils";
import type { AgentKind, Hunk } from "../../types";

const AGENT_STORAGE_KEY = "moonreview:selected-agent";

function fileNameFromPath(filePath: string) {
  const segments = filePath.split("/");
  return segments[segments.length - 1] || filePath;
}

function firstReviewFilePath(hunks: Hunk[], snoozedFiles: Set<string>) {
  const orderedPaths = filePathsInListOrder(hunks);
  const unstagedFiles = new Set(hunks.filter((hunk) => !hunk.staged).map((hunk) => hunk.file_path));

  for (const filePath of orderedPaths) {
    if (unstagedFiles.has(filePath) && !snoozedFiles.has(filePath)) {
      return filePath;
    }
  }

  return null;
}

function nextReviewFilePath(hunks: Hunk[], currentFilePath: string, snoozedFiles: Set<string>) {
  const orderedPaths = filePathsInListOrder(hunks);
  const currentIndex = orderedPaths.indexOf(currentFilePath);
  if (currentIndex === -1) {
    return null;
  }

  const unstagedFiles = new Set(hunks.filter((hunk) => !hunk.staged).map((hunk) => hunk.file_path));

  for (let offset = 1; offset < orderedPaths.length; offset += 1) {
    const candidate = orderedPaths[(currentIndex + offset) % orderedPaths.length];
    if (unstagedFiles.has(candidate) && !snoozedFiles.has(candidate)) {
      return candidate;
    }
  }

  return null;
}

function Success() {
  return (
    <section className="success-panel">
      <h2>All done 🌚</h2>
    </section>
  );
}

/// One review of one repo. The store it needs lives here, so a second review pane simply
/// brings its own.
export function ReviewPane({ sessionId }: { sessionId: string }) {
  const { focusedReviewSessionId, submoduleReviews, submodulesResolved } = useWorkspace();
  // The server forgets its sessions when it restarts, and only the repo the page was opened
  // on is handed back by the CLI. A submodule's session is reopened by the workspace, so
  // its review waits for that rather than asking about a session nobody knows yet.
  const sessionIsOpen =
    sessionId === getRootSessionId() ||
    submoduleReviews.some((review) => review.sessionId === sessionId);

  if (!sessionIsOpen && !submodulesResolved) {
    return <div className="review-pane-opening muted">Opening review...</div>;
  }

  return (
    <ReviewStoreProvider sessionId={sessionId}>
      <ReviewPaneContent focused={focusedReviewSessionId === sessionId} />
    </ReviewStoreProvider>
  );
}

function ReviewPaneContent({ focused }: { focused: boolean }) {
  const {
    state: { activeView, data, loadError },
    actions,
  } = useReviewStore();
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [pendingStageFile, setPendingStageFile] = useState<{ filePath: string; fileName: string } | null>(null);
  const [snoozedFiles, setSnoozedFiles] = useState<string[]>([]);
  const previousDataRef = useRef<typeof data>(null);
  const hadUnstagedHunksRef = useRef(false);
  const [reviewComplete, setReviewComplete] = useState(false);
  const [activeJumpTarget, setActiveJumpTarget] = useState<{
    filePath?: string | null;
    hunkId?: string | null;
    elementId: string;
  } | null>(null);
  const snoozedFileSet = new Set(snoozedFiles);
  const showReviewComplete = reviewComplete && activeView === ReviewView.All && !data?.active_commit;

  const reviewScroll = useMemo<ReviewScrollValue>(
    () => ({
      scrollToTop: () => scrollRef.current?.scrollTo({ top: 0, behavior: "auto" }),
      scrollElement: () => scrollRef.current,
    }),
    [],
  );

  // Clear pending jump targets when returning to the all-files view.
  useEffect(() => {
    if (activeView === ReviewView.All) {
      setActiveJumpTarget(null);
    }
  }, [activeView]);

  // Restore the user's persisted agent choice when it is still available.
  useEffect(() => {
    if (!data) {
      return;
    }

    const storedAgent = window.localStorage.getItem(AGENT_STORAGE_KEY) as AgentKind | null;
    if (!storedAgent || storedAgent === data.selected_agent) {
      return;
    }

    const storedAgentOption = data.available_agents.find((agent) => agent.kind === storedAgent);
    if (!storedAgentOption || !storedAgentOption.available) {
      window.localStorage.removeItem(AGENT_STORAGE_KEY);
      return;
    }

    void actions.setAgent(storedAgent);
  }, [actions, data]);

  // Keep the selected file valid as review data and view mode change.
  useEffect(() => {
    if (!data) {
      return;
    }

    if (activeView === ReviewView.All) {
      if (selectedFilePath && !data.hunks.some((hunk) => hunk.file_path === selectedFilePath)) {
        setSelectedFilePath(null);
      }
      return;
    }

    if (selectedFilePath && data.hunks.some((hunk) => hunk.file_path === selectedFilePath)) {
      return;
    }

    const fallbackFilePath =
      firstReviewFilePath(data.hunks, snoozedFileSet) ?? data.hunks[0]?.file_path ?? null;
    setSelectedFilePath(fallbackFilePath);
  }, [activeView, data, selectedFilePath, snoozedFiles]);

  // Show the completion state once all unstaged hunks have been staged.
  useEffect(() => {
    if (!data) {
      return;
    }
    if (data.active_commit) {
      setReviewComplete(false);
      hadUnstagedHunksRef.current = false;
      return;
    }

    const hasUnstaged = hasUnstagedHunks(data);
    if (hadUnstagedHunksRef.current && !hasUnstaged) {
      setReviewComplete(true);
      actions.setActiveView(ReviewView.All);
    } else if (hasUnstaged) {
      setReviewComplete(false);
    }
    hadUnstagedHunksRef.current = hasUnstaged;
  }, [data]);

  // Drop snoozed files once they no longer have active unstaged hunks.
  useEffect(() => {
    if (!data) {
      return;
    }

    const activePaths = new Set(
      data.hunks.filter((hunk) => !hunk.staged).map((hunk) => hunk.file_path),
    );
    setSnoozedFiles((current) => current.filter((filePath) => activePaths.has(filePath)));
  }, [data]);

  // Scroll to a file or hunk after navigation has rendered the target element.
  useEffect(() => {
    if (!activeJumpTarget) {
      return;
    }

    const timer = window.setTimeout(() => {
      const element = document.getElementById(activeJumpTarget.elementId);
      if (!element) {
        setActiveJumpTarget(null);
        return;
      }
      element.scrollIntoView({ behavior: "smooth", block: "start" });
      setActiveJumpTarget(null);
    }, 40);

    return () => window.clearTimeout(timer);
  }, [activeJumpTarget]);

  // Advance to the next review file after the current file becomes fully staged.
  useEffect(() => {
    if (!data) {
      previousDataRef.current = data;
      return;
    }
    if (data.active_commit) {
      previousDataRef.current = data;
      return;
    }

    const previousData = previousDataRef.current;
    if (!previousData) {
      previousDataRef.current = data;
      return;
    }

    const completedFilePaths = fullyStagedFilePaths(previousData, data);
    const completedSelectedFilePath =
      selectedFilePath && completedFilePaths.includes(selectedFilePath) ? selectedFilePath : null;
    const completedPendingFilePath =
      pendingStageFile && completedFilePaths.includes(pendingStageFile.filePath)
        ? pendingStageFile.filePath
        : null;
    const completedFilePath =
      completedSelectedFilePath ?? completedPendingFilePath ?? completedFilePaths[0];

    if (completedFilePath) {
      const completedFileName =
        pendingStageFile?.filePath === completedFilePath
          ? pendingStageFile.fileName
          : fileNameFromPath(completedFilePath);
      toast.success(`file ${completedFileName} fully staged`);
      if (activeView === ReviewView.File) {
        const nextFilePath = nextReviewFilePath(data.hunks, completedFilePath, snoozedFileSet);
        if (nextFilePath && nextFilePath !== selectedFilePath) {
          navigateToFile(nextFilePath);
        }
      }
      if (pendingStageFile?.filePath === completedFilePath) {
        setPendingStageFile(null);
      }
    }

    previousDataRef.current = data;
  }, [activeView, data, pendingStageFile, selectedFilePath, snoozedFiles]);

  function handleAgentChange(agent: AgentKind) {
    window.localStorage.setItem(AGENT_STORAGE_KEY, agent);
    void actions.setAgent(agent);
  }

  function navigateToFile(filePath: string) {
    reviewScroll.scrollToTop();
    actions.setActiveView(ReviewView.File);
    setSelectedFilePath(filePath);
    setActiveJumpTarget({
      filePath,
      hunkId: null,
      elementId: `file-${encodeURIComponent(filePath)}`,
    });
  }

  function showAllFiles() {
    setReviewComplete(false);
    actions.setActiveView(ReviewView.All);
  }

  function jumpToComment(target: { filePath: string; hunkId: string; elementId: string }) {
    actions.setActiveView(ReviewView.File);
    setSelectedFilePath(target.filePath);
    setActiveJumpTarget(target);
  }

  function snoozeFile(filePath: string) {
    if (!data) {
      return;
    }

    const nextSnoozedFiles = new Set(snoozedFileSet);
    nextSnoozedFiles.add(filePath);
    setSnoozedFiles([...nextSnoozedFiles]);

    const nextFilePath = nextReviewFilePath(data.hunks, filePath, nextSnoozedFiles);
    if (!nextFilePath || nextFilePath === filePath) {
      return;
    }

    navigateToFile(nextFilePath);
  }

  if (!data) {
    return (
      <div className="review-pane-scroll" ref={scrollRef}>
        <section className="panel">
          <div className={loadError ? "panel-message panel-message-error" : "panel-message"}>
            {loadError || "Loading review state..."}
          </div>
        </section>
      </div>
    );
  }

  return (
    <ReviewScrollProvider value={reviewScroll}>
      <div className="review-pane">
        <LeftSidebar
          data={data}
          snoozedFiles={snoozedFileSet}
          activeFilePath={activeView === ReviewView.File ? selectedFilePath : null}
          onJumpToFile={navigateToFile}
          onJumpToComment={jumpToComment}
          onShowAll={showAllFiles}
          onStageWholeFile={(file) => {
            setPendingStageFile({ filePath: file.filePath, fileName: file.fileName });
          }}
        />

        <div className="review-pane-scroll" ref={scrollRef}>
          <section className="review-main">
            {showReviewComplete ? (
              <Success />
            ) : (
              <>
                <Hunks
                  hunks={data.hunks}
                  agents={data.available_agents}
                  selectedAgent={data.selected_agent}
                  onAgentChange={handleAgentChange}
                  onSnoozeFile={snoozeFile}
                  onJumpToHunk={jumpToComment}
                  selectedFilePath={activeView === ReviewView.File ? selectedFilePath : null}
                  targetFilePath={activeJumpTarget?.filePath ?? null}
                  targetHunkId={activeJumpTarget?.hunkId ?? null}
                  shortcutsEnabled={focused}
                />
                <Footer exportText={data.export_text} />
              </>
            )}
          </section>
        </div>
      </div>
    </ReviewScrollProvider>
  );
}
