import { useEffect, useState } from "react";
import {
  clampDockSize,
  mapDockToCssVariables,
  mapDockToOtherDock,
  mapDockToPanelStyle,
  mapDockToSizeFromPointer,
  useDockWindows,
} from "./dockWindowState";
import { mapWindowIdToDefinition } from "./windowRegistry";

/// The single dock that hosts every window. Open windows stay mounted so a backgrounded
/// window (a live shell, a running agent log) keeps its state.
export function DockWindows() {
  const {
    openWindowIds,
    activeWindowId,
    dock,
    dockSize,
    closeWindow,
    focusWindow,
    setDock,
    setDockSize,
  } = useDockWindows();
  const [resizing, setResizing] = useState(false);
  const open = openWindowIds.length > 0;
  const otherDock = mapDockToOtherDock[dock];

  // Let the page layout reserve the space the dock occupies.
  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dock-window-open", open);
    const variables = open
      ? mapDockToCssVariables[dock](dockSize)
      : { "--dock-window-width": "0px", "--dock-window-height": "0px" };
    for (const [name, value] of Object.entries(variables)) {
      root.style.setProperty(name, value);
    }
    return () => {
      root.classList.remove("dock-window-open");
      root.style.removeProperty("--dock-window-width");
      root.style.removeProperty("--dock-window-height");
    };
  }, [dock, dockSize, open]);

  // Keep the dock within its limits when the browser window itself is resized.
  useEffect(() => {
    if (!open) {
      return;
    }

    function clampToWindow() {
      setDockSize(clampDockSize(dock, dockSize));
    }

    clampToWindow();
    window.addEventListener("resize", clampToWindow);
    return () => window.removeEventListener("resize", clampToWindow);
  }, [dock, dockSize, open]);

  // Track the drag on the window so the pointer can leave the fringe while resizing.
  useEffect(() => {
    if (!resizing) {
      return;
    }

    function handlePointerMove(event: PointerEvent) {
      setDockSize(clampDockSize(dock, mapDockToSizeFromPointer[dock](event)));
    }

    function stopResizing() {
      setResizing(false);
    }

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", stopResizing);
    window.addEventListener("pointercancel", stopResizing);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", stopResizing);
      window.removeEventListener("pointercancel", stopResizing);
    };
  }, [dock, resizing]);

  if (!open) {
    return null;
  }

  const fringeClasses = ["dock-window-fringe"];
  if (resizing) {
    fringeClasses.push("dock-window-fringe-resizing");
  }

  return (
    <div className={`dock-window dock-window-${dock}`} style={mapDockToPanelStyle[dock](dockSize)}>
      <div
        className={fringeClasses.join(" ")}
        title="Drag to resize"
        onPointerDown={(event) => {
          if (event.button !== 0) {
            return;
          }
          event.preventDefault();
          setResizing(true);
        }}
      />
      <div className="dock-window-body">
        <div className="dock-window-bar">
          <div className="dock-window-tabs">
            {openWindowIds.map((windowId) => (
              <div
                key={windowId}
                className={`dock-window-tab${windowId === activeWindowId ? " dock-window-tab-active" : ""}`}
              >
                <button type="button" onClick={() => focusWindow(windowId)}>
                  {mapWindowIdToDefinition[windowId].title}
                </button>
                <button
                  type="button"
                  className="dock-window-tab-close"
                  onClick={() => closeWindow(windowId)}
                  title={`Close ${mapWindowIdToDefinition[windowId].title}`}
                  aria-label={`Close ${mapWindowIdToDefinition[windowId].title}`}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
          <button
            type="button"
            className="dock-window-dock-toggle"
            onClick={() => setDock(otherDock)}
            title={`Dock to the ${otherDock}`}
          >
            [{otherDock}]
          </button>
        </div>
        {openWindowIds.map((windowId) => (
          <div key={windowId} className="dock-window-surface" hidden={windowId !== activeWindowId}>
            {mapWindowIdToDefinition[windowId].render()}
          </div>
        ))}
      </div>
    </div>
  );
}
