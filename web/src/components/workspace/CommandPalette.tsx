import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { getRootSessionId } from "../../api";
import { useReviewStoreFor } from "../../reviewStores";
import { filterUiCommands, uiCommandsFor } from "./commands";
import { useWorkspace } from "./workspaceState";
import type { UiCommand } from "./commands";

export function CommandPalette() {
  const workspace = useWorkspace();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const reviewStore = useReviewStoreFor(workspace.focusedReviewSessionId);
  const rootReviewStore = useReviewStoreFor(getRootSessionId());
  const commands = uiCommandsFor(
    workspace.layout,
    getRootSessionId(),
    reviewStore?.state.data?.available_agents,
    rootReviewStore?.state.data?.repo_name,
  );
  const matches = filterUiCommands(commands, query);

  function closePalette() {
    setOpen(false);
    requestAnimationFrame(() => previousFocusRef.current?.focus());
  }

  function runCommand(command: UiCommand) {
    workspace.openPane(command.request);
    closePalette();
  }

  useEffect(() => {
    function openPalette(event: KeyboardEvent) {
      if (
        !event.metaKey ||
        !event.shiftKey ||
        event.ctrlKey ||
        event.altKey ||
        event.key.toLowerCase() !== "p"
      ) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
      previousFocusRef.current = document.activeElement as HTMLElement | null;
      setQuery("");
      setSelectedIndex(0);
      setOpen(true);
    }

    window.addEventListener("keydown", openPalette, true);
    return () => window.removeEventListener("keydown", openPalette, true);
  }, []);

  if (!open) {
    return null;
  }

  return createPortal(
    <div className="command-palette-backdrop" onPointerDown={closePalette}>
      <div
        className="command-palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <div className="command-palette-search">
          <span aria-hidden="true">&gt;</span>
          <input
            autoFocus
            value={query}
            placeholder="Search actions"
            aria-label="Search actions"
            aria-controls="command-palette-results"
            aria-activedescendant={
              matches[selectedIndex] ? `command-${matches[selectedIndex].id}` : undefined
            }
            onChange={(event) => {
              setQuery(event.currentTarget.value);
              setSelectedIndex(0);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                closePalette();
              } else if (event.key === "ArrowDown" && matches.length > 0) {
                event.preventDefault();
                setSelectedIndex((current) => (current + 1) % matches.length);
              } else if (event.key === "ArrowUp" && matches.length > 0) {
                event.preventDefault();
                setSelectedIndex((current) => (current - 1 + matches.length) % matches.length);
              } else if (event.key === "Enter" && matches[selectedIndex]) {
                event.preventDefault();
                runCommand(matches[selectedIndex]);
              }
            }}
          />
          <kbd>esc</kbd>
        </div>
        <div className="command-palette-results" id="command-palette-results" role="listbox">
          {matches.map((command, index) => (
            <button
              key={command.id}
              id={`command-${command.id}`}
              type="button"
              role="option"
              aria-selected={index === selectedIndex}
              className={`command-palette-item${
                index === selectedIndex ? " command-palette-item-selected" : ""
              }`}
              onPointerMove={() => setSelectedIndex(index)}
              onClick={() => runCommand(command)}
            >
              <span>{command.title}</span>
              <small>{command.description}</small>
            </button>
          ))}
          {matches.length === 0 ? (
            <div className="command-palette-empty">no matching actions</div>
          ) : null}
        </div>
        <div className="command-palette-hint">
          &uarr;&darr; select&nbsp;&nbsp; &crarr; run
        </div>
      </div>
    </div>,
    document.body,
  );
}
