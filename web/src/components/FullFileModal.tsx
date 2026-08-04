import { useEffect, useMemo, useState } from "react";
import hljs from "highlight.js/lib/common";
import { fetchFileContent } from "../api";
import { useReviewStore } from "../reviewStore";

type FullFileModalProps = {
  filePath: string;
  lineNumber?: number | null;
  onClose: () => void;
};

type FullFileContentProps = {
  filePath: string;
  lineNumber?: number | null;
  idPrefix: string;
};

function FullFileContent({ filePath, lineNumber, idPrefix }: FullFileContentProps) {
  const {
    state: { sessionId },
  } = useReviewStore();
  const [content, setContent] = useState("");
  const [loadError, setLoadError] = useState("");

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        const file = await fetchFileContent(sessionId, filePath);
        if (cancelled) {
          return;
        }
        setContent(file.content);
        setLoadError("");
      } catch (error) {
        if (cancelled) {
          return;
        }
        setLoadError(error instanceof Error ? error.message : "Failed to load file.");
      }
    }

    setContent("");
    setLoadError("");
    void load();

    return () => {
      cancelled = true;
    };
  }, [filePath, sessionId]);

  useEffect(() => {
    if (!lineNumber || loadError) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      document.getElementById(`${idPrefix}-L${lineNumber}`)?.scrollIntoView({ block: "center" });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [lineNumber, loadError, content, idPrefix]);

  const highlightedFileHtml = useMemo(
    () => hljs.highlightAuto(content || " ").value || "&nbsp;",
    [content],
  );
  const lineNumbers = useMemo(
    () => content.split("\n").map((_, index) => index + 1),
    [content],
  );

  if (loadError) {
    return <div className="panel-message panel-message-error">{loadError}</div>;
  }

  return (
    <div className="full-file-code">
      <div className="full-file-gutter" aria-hidden="true">
        {lineNumbers.map((currentLineNumber) => (
          <a
            key={currentLineNumber}
            id={`${idPrefix}-L${currentLineNumber}`}
            className={`full-file-line-number ${
              lineNumber === currentLineNumber ? "full-file-line-target" : ""
            }`.trim()}
            href={`#${idPrefix}-L${currentLineNumber}`}
          >
            {currentLineNumber}
          </a>
        ))}
      </div>
      <pre className="full-file-code-block">
        <code className="hljs" dangerouslySetInnerHTML={{ __html: highlightedFileHtml }} />
      </pre>
    </div>
  );
}

export function FullFileView({ filePath }: { filePath: string }) {
  return (
    <section className="panel full-file-view">
      <div className="full-file-view-head">
        <h2>{filePath}</h2>
      </div>
      <FullFileContent filePath={filePath} idPrefix="file" />
    </section>
  );
}

export function FullFileModal({ filePath, lineNumber, onClose }: FullFileModalProps) {
  useEffect(() => {
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }

    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  return (
    <div className="full-file-modal-backdrop" role="presentation" onClick={onClose}>
      <section
        className="panel full-file-modal"
        role="dialog"
        aria-modal="true"
        aria-label={`View ${filePath}`}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="full-file-view-head">
          <h2>{filePath}</h2>
          <button type="button" onClick={onClose}>Close</button>
        </div>
        <FullFileContent filePath={filePath} lineNumber={lineNumber} idPrefix="modal" />
      </section>
    </div>
  );
}
