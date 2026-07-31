import { createContext, useContext } from "react";
import { useFrameRects } from "./frameRects";
import { renderPane } from "./paneRegistry";
import { useWorkspace } from "./workspaceState";
import { frameHoldingPane } from "./layout";
import type { Pane } from "./layout";

const offscreenRect = { left: 0, top: 0, width: 0, height: 0 };

/// Whether the surrounding pane is the one its frame is currently showing.
const PaneVisibleContext = createContext(true);

export function usePaneVisible() {
  return useContext(PaneVisibleContext);
}

/// Draws one pane over its frame's body. Panes are never unmounted while they exist, so
/// moving a tab between frames only changes these coordinates.
export function PaneHost({ pane }: { pane: Pane }) {
  const { layout, focusPaneById } = useWorkspace();
  const { frameRects } = useFrameRects();
  const frame = frameHoldingPane(layout, pane.paneId);
  const rect = frame ? frameRects[frame.frameId] : null;
  const visible = Boolean(rect) && frame?.activePaneId === pane.paneId;

  const { left, top, width, height } = rect ?? offscreenRect;

  return (
    <div
      className="pane-host"
      style={{ left, top, width, height }}
      aria-hidden={!visible}
      data-visible={visible ? "true" : "false"}
      // Working inside a pane makes its frame the one new tabs open in.
      onPointerDownCapture={() => {
        if (visible && layout.activeFrameId !== frame?.frameId) {
          focusPaneById(pane.paneId);
        }
      }}
    >
      <PaneVisibleContext.Provider value={visible}>{renderPane(pane)}</PaneVisibleContext.Provider>
    </div>
  );
}
