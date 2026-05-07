import { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import hljs from "highlight.js/lib/common";
import "highlight.js/styles/github.css";
import { toast, Toaster } from "sonner";
import "./app.css";
import "./fullFileView.css";
import { fetchFileContent, fetchSessionState } from "./api";
import { Footer } from "./components/Footer";
import { Header } from "./components/Header";
import { LeftSidebar } from "./components/LeftSidebar";
import { Hunks } from "./components/hunks/Hunks";
import { ReviewStoreProvider, ReviewView, useReviewStore } from "./reviewStore";
import { ThemeProvider, useTheme } from "./theme";
import type { AgentKind, Hunk, SessionState } from "./types";

const AGENT_STORAGE_KEY = "moonreview:selected-agent";

function fileNameFromPath(filePath: string) {
  const segments = filePath.split("/");
  return segments[segments.length - 1] || filePath;
}

function filePathsInListOrder(hunks: Hunk[]) {
  const seen = new Set<string>();
  const ordered: string[] = [];
  for (const hunk of hunks) {
    if (seen.has(hunk.file_path)) {
      continue;
    }
    seen.add(hunk.file_path);
    ordered.push(hunk.file_path);
  }
  return ordered;
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

  const unstagedFiles = new Set(
    hunks.filter((hunk) => !hunk.staged).map((hunk) => hunk.file_path),
  );

  for (let offset = 1; offset < orderedPaths.length; offset += 1) {
    const candidate = orderedPaths[(currentIndex + offset) % orderedPaths.length];
    if (unstagedFiles.has(candidate) && !snoozedFiles.has(candidate)) {
      return candidate;
    }
  }

  return null;
}

function requestedFilePath() {
  return new URLSearchParams(window.location.search).get("file_path");
}

function requestedLineNumberFromHash(hash: string): number | null {
  const match = /^#L(\d+)(?:-L\d+)?$/.exec(hash);
  if (!match) {
    return null;
  }

  return Number.parseInt(match[1], 10);
}

function FullFileView() {
  return (
    <ThemeProvider>
      <FullFileViewContent />
    </ThemeProvider>
  );
}

function FullFileViewContent() {
  const { theme } = useTheme();
  const [session, setSession] = useState<SessionState | null>(null);
  const [content, setContent] = useState("");
  const [loadError, setLoadError] = useState("");
  const [activeHash, setActiveHash] = useState(window.location.hash);
  const filePath = requestedFilePath();

  useEffect(() => {
    let cancelled = false;

    async function load() {
      if (!filePath) {
        setLoadError("Missing file path.");
        return;
      }

      try {
        const [sessionData, fileData] = await Promise.all([
          fetchSessionState(),
          fetchFileContent(filePath),
        ]);
        if (cancelled) {
          return;
        }

        setSession(sessionData);
        setContent(fileData.content);
        setLoadError("");
      } catch (error) {
        if (cancelled) {
          return;
        }

        setLoadError(error instanceof Error ? error.message : "Failed to load file.");
      }
    }

    void load();

    return () => {
      cancelled = true;
    };
  }, [filePath]);

  const highlightedFileHtml = useMemo(() => hljs.highlightAuto(content || " ").value || "&nbsp;", [content]);
  const lineNumbers = useMemo(
    () => content.split("\n").map((_, index) => index + 1),
    [content],
  );

  useEffect(() => {
    const jumpToHashLine = () => {
      setActiveHash(window.location.hash);
      const targetLine = requestedLineNumberFromHash(window.location.hash);
      if (!targetLine) {
        return;
      }

      const element = document.getElementById(`L${targetLine}`);
      if (!element) {
        return;
      }

      element.scrollIntoView({ block: "start" });
    };

    jumpToHashLine();
    window.addEventListener("hashchange", jumpToHashLine);
    return () => {
      window.removeEventListener("hashchange", jumpToHashLine);
    };
  }, [lineNumbers]);

  return (
    <>
      <Toaster closeButton position="bottom-right" richColors theme={theme} />
      <Header repoName={session?.repo_name} branchName={session?.branch_name} />
      <main>
        <section className="panel full-file-view">
          <div className="full-file-view-head">
            <div>
              <h2>{filePath ?? "File"}</h2>
            </div>
          </div>
          {loadError ? (
            <div className="panel-message panel-message-error">{loadError}</div>
          ) : (
            <div className="full-file-code">
              <div className="full-file-gutter" aria-hidden="true">
                {lineNumbers.map((lineNumber) => (
                  <a
                    key={lineNumber}
                    id={`L${lineNumber}`}
                    className={`full-file-line-number ${
                      requestedLineNumberFromHash(activeHash) === lineNumber
                        ? "full-file-line-target"
                        : ""
                    }`.trim()}
                    href={`#L${lineNumber}`}
                  >
                    {lineNumber}
                  </a>
                ))}
              </div>
              <pre className="full-file-code-block">
                <code
                  className="hljs"
                  dangerouslySetInnerHTML={{ __html: highlightedFileHtml }}
                />
              </pre>
            </div>
          )}
        </section>
      </main>
    </>
  );
}

function AppContent() {
  return (
    <ThemeProvider>
      <AppContentInner />
    </ThemeProvider>
  );
}

function hasUnstagedHunks(data: SessionState | null) {
  return data?.hunks.some((hunk) => !hunk.staged) ?? false;
}

function Success() {
  return (
    <section className="success-panel">
      <h2>All done 🌚</h2>
    </section>
  );
}

function AppContentInner() {
  const { theme } = useTheme();
  const {
    state: { activeView, data, loadError },
    actions,
  } = useReviewStore();
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

  useEffect(() => {
    if (activeView === ReviewView.All) {
      setActiveJumpTarget(null);
    }
  }, [activeView]);

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

    const fallbackFilePath = firstReviewFilePath(data.hunks, snoozedFileSet) ?? data.hunks[0]?.file_path ?? null;
    setSelectedFilePath(fallbackFilePath);
  }, [activeView, data, selectedFilePath, snoozedFiles]);

  useEffect(() => {
    if (!data) {
      return;
    }

    const hasUnstaged = hasUnstagedHunks(data);
    if (hadUnstagedHunksRef.current && !hasUnstaged) {
      setReviewComplete(true);
    } else if (hasUnstaged) {
      setReviewComplete(false);
    }
    hadUnstagedHunksRef.current = hasUnstaged;
  }, [data]);

  useEffect(() => {
    if (!data) {
      return;
    }

    const activePaths = new Set(
      data.hunks.filter((hunk) => !hunk.staged).map((hunk) => hunk.file_path),
    );
    setSnoozedFiles((current) => current.filter((filePath) => activePaths.has(filePath)));
  }, [data]);

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

  useEffect(() => {
    if (!data || !pendingStageFile) {
      return;
    }
    if (pendingStageFile.filePath === selectedFilePath) {
      return;
    }

    const fileStillExists = data.hunks.some((hunk) => hunk.file_path === pendingStageFile.filePath);
    const hasUnstagedHunks = data.hunks.some(
      (hunk) => hunk.file_path === pendingStageFile.filePath && !hunk.staged,
    );

    if (!fileStillExists || hasUnstagedHunks) {
      return;
    }

    toast.success(`file ${pendingStageFile.fileName} fully staged`);
    const nextFilePath = nextReviewFilePath(data.hunks, pendingStageFile.filePath, snoozedFileSet);
    if (nextFilePath) {
      navigateToFile(nextFilePath);
    }
    setPendingStageFile(null);
  }, [data, pendingStageFile, snoozedFiles]);

  useEffect(() => {
    if (!data || !selectedFilePath) {
      previousDataRef.current = data;
      return;
    }

    const previousData = previousDataRef.current;
    const hadUnstagedHunks = previousData?.hunks.some(
      (hunk) => hunk.file_path === selectedFilePath && !hunk.staged,
    ) ?? false;
    const hasUnstagedHunks = data.hunks.some(
      (hunk) => hunk.file_path === selectedFilePath && !hunk.staged,
    );

    if (hadUnstagedHunks && !hasUnstagedHunks) {
      toast.success(`file ${fileNameFromPath(selectedFilePath)} fully staged`);
      const nextFilePath = nextReviewFilePath(data.hunks, selectedFilePath, snoozedFileSet);
      if (nextFilePath && nextFilePath !== selectedFilePath) {
        navigateToFile(nextFilePath);
      }
      if (pendingStageFile?.filePath === selectedFilePath) {
        setPendingStageFile(null);
      }
    }

    previousDataRef.current = data;
  }, [data, pendingStageFile, selectedFilePath, snoozedFiles]);

  function handleAgentChange(agent: AgentKind) {
    window.localStorage.setItem(AGENT_STORAGE_KEY, agent);
    void actions.setAgent(agent);
  }

  function navigateToFile(filePath: string) {
    window.scrollTo({ top: 0, left: 0, behavior: "auto" });
    actions.setActiveView(ReviewView.File);
    setSelectedFilePath(filePath);
    setActiveJumpTarget({
      filePath,
      hunkId: null,
      elementId: `file-${encodeURIComponent(filePath)}`,
    });
  }

  function jumpToFile(filePath: string) {
    navigateToFile(filePath);
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
      <>
        <Toaster closeButton position="bottom-right" richColors theme={theme} />
        <Header />
        <main>
          <section className="panel">
            <div className={loadError ? "panel-message panel-message-error" : "panel-message"}>
              {loadError || "Loading review state..."}
            </div>
          </section>
        </main>
      </>
    );
  }

  return (
    <>
      <Toaster closeButton position="bottom-right" richColors theme={theme} />
      <Header repoName={data.repo_name} branchName={data.branch_name} />
      <main>
        <div className="review-layout">
          <LeftSidebar
            data={data}
            snoozedFiles={snoozedFileSet}
            activeFilePath={activeView === ReviewView.File ? selectedFilePath : null}
            onJumpToFile={jumpToFile}
            onJumpToComment={jumpToComment}
            onStageWholeFile={(file) => {
              setPendingStageFile({ filePath: file.filePath, fileName: file.fileName });
            }}
          />

          <section className="review-main">
            {reviewComplete ? (
              <Success />
            ) : (
              <>
                <Hunks
                  hunks={data.hunks}
                  agents={data.available_agents}
                  selectedAgent={data.selected_agent}
                  onAgentChange={handleAgentChange}
                  onSnoozeFile={snoozeFile}
                  selectedFilePath={activeView === ReviewView.File ? selectedFilePath : null}
                  targetFilePath={activeJumpTarget?.filePath ?? null}
                  targetHunkId={activeJumpTarget?.hunkId ?? null}
                />
                <Footer exportText={data.export_text} />
              </>
            )}
          </section>
        </div>
      </main>
    </>
  );
}

function App() {
  if (window.location.pathname.endsWith("/file")) {
    return <FullFileView />;
  }

  return (
    <ReviewStoreProvider>
      <AppContent />
    </ReviewStoreProvider>
  );
}

createRoot(document.getElementById("app")!).render(<App />);
