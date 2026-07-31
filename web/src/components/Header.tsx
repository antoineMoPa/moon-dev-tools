import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTheme } from "../theme";
import { getRootSessionId } from "../api";
import { useReviewStoreFor } from "../reviewStores";
import { useWorkspace } from "./workspace/workspaceState";
import { findPaneOfKind, findReviewPane } from "./workspace/layout";
import type { OpenPaneRequest } from "./workspace/layout";
import type { SessionState } from "../types";

/// What the review is currently pointed at: the working tree, or one commit.
function reviewLabelForSession(session?: SessionState | null): string | null {
  if (!session) {
    return null;
  }

  if (!session.active_commit) {
    return "local changes";
  }

  const commit = session.commits.find((candidate) => candidate.sha === session.active_commit);
  return commit ? `${commit.short_sha} ${commit.subject}` : session.active_commit.slice(0, 7);
}

/// The app name, shown at the head of the top-left frame's tab strip — the app has a single
/// header, and that strip is it.
export function HeaderBrand() {
  return (
    <h1 className="header-brand">
      <a
        className="header-title-link"
        href="https://github.com/antoineMoPa/moonreview"
        target="_blank"
        rel="noreferrer"
      >
        🌚 moonreview
      </a>
    </h1>
  );
}

type OpenableWindow = {
  key: string;
  title: string;
  request: OpenPaneRequest;
};

/// What the menu offers: the review of the repo this page was opened on, the agent monitor,
/// and always another shell. Submodule reviews are not listed — they open as tabs on their
/// own when the submodule has changes.
function openableWindowsFor(workspace: ReturnType<typeof useWorkspace>): OpenableWindow[] {
  const rootSessionId = getRootSessionId();

  return [
    ...(findReviewPane(workspace.layout, rootSessionId)
      ? []
      : [
          {
            key: rootSessionId,
            title: "review",
            request: { kind: "review" as const, sessionId: rootSessionId, title: "review" },
          },
        ]),
    ...(findPaneOfKind(workspace.layout, "agents")
      ? []
      : [{ key: "agents", title: "agents", request: { kind: "agents" as const } }]),
    { key: "terminal", title: "terminal", request: { kind: "terminal" as const } },
  ];
}

/// Each frame's button for opening a pane into it, with the kinds behind a popover. The
/// popover is rendered into the body so the tab strip it lives in cannot clip it.
export function OpenWindowMenu({ frameId }: { frameId: string }) {
  const workspace = useWorkspace();
  const [menuRect, setMenuRect] = useState<DOMRect | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const openableWindows = openableWindowsFor(workspace);

  useEffect(() => {
    if (!menuRect) {
      return;
    }

    function closeOnOutsidePress(event: PointerEvent) {
      const target = event.target instanceof Node ? event.target : null;
      // A press on the button or inside the popover itself is not "outside".
      if (target && (buttonRef.current?.contains(target) || menuRef.current?.contains(target))) {
        return;
      }
      setMenuRect(null);
    }

    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setMenuRect(null);
      }
    }

    window.addEventListener("pointerdown", closeOnOutsidePress, true);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", closeOnOutsidePress, true);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [menuRect]);

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className="header-pane-toggle"
        aria-haspopup="menu"
        aria-expanded={menuRect !== null}
        aria-label="Open a window"
        title="Open a window"
        onClick={() =>
          setMenuRect((current) =>
            current ? null : (buttonRef.current?.getBoundingClientRect() ?? null),
          )
        }
      >
        [+]
      </button>
      {menuRect
        ? createPortal(
            <div
              ref={menuRef}
              className="pane-menu"
              role="menu"
              style={{ top: menuRect.bottom + 4, right: window.innerWidth - menuRect.right }}
            >
              {openableWindows.map((window) => (
                <button
                  key={window.key}
                  type="button"
                  role="menuitem"
                  className="pane-menu-item"
                  onClick={() => {
                    workspace.openPane(window.request, frameId);
                    setMenuRect(null);
                  }}
                >
                  {window.title}
                </button>
              ))}
            </div>,
            document.body,
          )
        : null}
    </>
  );
}

/// What is being reviewed, centred in the header strip between the tabs and the actions.
export function HeaderCenter() {
  const { focusedReviewSessionId } = useWorkspace();
  const reviewStore = useReviewStoreFor(focusedReviewSessionId);
  const data = reviewStore?.state.data;
  const reviewLabel = reviewLabelForSession(data);
  const movedDiffLayout = reviewStore?.state.movedDiffLayout ?? "unified";
  const nextMovedDiffLayout = movedDiffLayout === "side-by-side" ? "unified" : "side-by-side";

  return (
    <div className="header-center">
      {reviewLabel ? <div className="header-review-label">{reviewLabel}</div> : null}
      {reviewStore ? (
        <button
          type="button"
          className="header-move-layout-toggle"
          onClick={() => reviewStore.actions.setMovedDiffLayout(nextMovedDiffLayout)}
          title="Toggle moved-code diff layout"
        >
          [{movedDiffLayout === "side-by-side" ? "unified" : "side by side"}]
        </button>
      ) : null}
    </div>
  );
}

/// The app-wide controls that belong to no single tab.
export function HeaderActions() {
  const { theme, toggleTheme } = useTheme();
  const nextTheme = theme === "dark" ? "light" : "dark";

  return (
    <div className="header-actions">
      <button
        type="button"
        className="theme-toggle"
        onClick={toggleTheme}
        aria-label={`Switch to ${nextTheme} mode`}
        title={`Switch to ${nextTheme} mode`}
      >
        <span className="theme-toggle-icon" aria-hidden="true">
          {theme === "dark" ? "☀" : "☾"}
        </span>
        <span>{theme === "dark" ? "Light" : "Dark"}</span>
      </button>
    </div>
  );
}
