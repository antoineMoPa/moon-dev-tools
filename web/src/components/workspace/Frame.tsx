import { useCallback, useState } from "react";
import { HeaderActions, HeaderBrand, HeaderCenter, OpenWindowMenu } from "../Header";
import { useFrameRects } from "./frameRects";
import { frameHoldingPane, frameIdsInLayout, primaryFrameId } from "./layout";
import { paneTitle } from "./paneRegistry";
import { useReviewStoreFor } from "../../reviewStores";
import { useWorkspace } from "./workspaceState";
import type { DropSide, Pane } from "./layout";

const edgeDropSides: Exclude<DropSide, "tabs">[] = ["left", "right", "top", "bottom"];

/// The app header lives in the primary frame's strip but acts on the whole app, so pressing
/// it must not make that frame the one new panes open in.
const APP_HEADER_SELECTOR = ".header-brand, .header-center, .header-actions";

/// React events travel the component tree, so a press inside a portal this frame rendered
/// (the window menu) arrives here even though it happened elsewhere on the page.
function pressedInsideFrame(event: React.PointerEvent<HTMLDivElement>) {
  return event.target instanceof Node && event.currentTarget.contains(event.target);
}

const TRANSPARENT_GIF =
  "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

/// The browser's drag image would float the tab away from the strip, including vertically.
/// The tab itself slides between its neighbours instead, so the ghost is hidden.
function hideDragImage(event: React.DragEvent) {
  const image = new Image();
  image.src = TRANSPARENT_GIF;
  event.dataTransfer.setDragImage(image, 0, 0);
}

/// A frame is a tab strip plus the space its active tab is drawn into. The tab content
/// itself lives in `PaneHost`, positioned over the body measured here.
export function Frame({ frameId }: { frameId: string }) {
  const { layout, closePaneById, focusPaneById, movePane, draggedPaneId, setDraggedPaneId } =
    useWorkspace();
  const { registerFrameBody } = useFrameRects();
  const [hoveredDropSide, setHoveredDropSide] = useState<DropSide | null>(null);
  const frame = layout.frames[frameId];
  const isActiveFrame = layout.activeFrameId === frameId;
  // The top-left frame carries the app header, so there is only ever one of them.
  const isPrimaryFrame = primaryFrameId(layout.root) === frameId;
  // With a single frame there is nothing to tell apart, so the focus outline is just noise.
  const showFocus = isActiveFrame && frameIdsInLayout(layout.root).length > 1;

  const bodyRef = useCallback(
    (element: HTMLDivElement | null) => registerFrameBody(frameId, element),
    [frameId, registerFrameBody],
  );

  function dropPane(side: DropSide, beforePaneId?: string | null) {
    setHoveredDropSide(null);
    if (!draggedPaneId) {
      return;
    }
    movePane(draggedPaneId, frameId, side, beforePaneId);
    setDraggedPaneId(null);
  }

  /// Dragging over a neighbour swaps the two right away, so the tab travels along the strip
  /// in step with the pointer instead of waiting for the drop.
  function reorderWhileDragging(targetPaneId: string, insertAfter: boolean) {
    if (!draggedPaneId || draggedPaneId === targetPaneId) {
      return;
    }
    if (frameHoldingPane(layout, draggedPaneId)?.frameId !== frameId) {
      return;
    }

    const remaining = frame.paneIds.filter((paneId) => paneId !== draggedPaneId);
    const insertAt = remaining.indexOf(targetPaneId) + (insertAfter ? 1 : 0);
    const nextPaneIds = [...remaining];
    nextPaneIds.splice(insertAt, 0, draggedPaneId);
    if (nextPaneIds.every((paneId, index) => paneId === frame.paneIds[index])) {
      return;
    }

    movePane(draggedPaneId, frameId, "tabs", remaining[insertAt] ?? null);
  }

  return (
    <div
      className={`frame${showFocus ? " frame-active" : ""}`}
      onPointerDownCapture={(event) => {
        if (!pressedInsideFrame(event)) {
          return;
        }
        if (event.target instanceof Element && event.target.closest(APP_HEADER_SELECTOR)) {
          return;
        }
        if (!isActiveFrame && frame.activePaneId) {
          focusPaneById(frame.activePaneId);
        }
      }}
    >
      <div
        className={`frame-tabs${isPrimaryFrame ? " frame-tabs-primary" : ""}`}
        onDragOver={(event) => {
          if (draggedPaneId) {
            event.preventDefault();
          }
        }}
        onDrop={(event) => {
          event.preventDefault();
          dropPane("tabs");
        }}
      >
        {/* Three zones: the tabs, what is being reviewed, and the app controls. The tabs
            scroll within their own zone, so a long row of them never reaches the others. */}
        <div className="frame-tabs-start">
          {isPrimaryFrame ? <HeaderBrand /> : null}
          <div className="frame-tab-list">
            {frame.paneIds.map((paneId) => (
              <FrameTab
                key={paneId}
                pane={layout.panes[paneId]}
                active={paneId === frame.activePaneId}
                onFocus={() => focusPaneById(paneId)}
                onClose={() => closePaneById(paneId)}
                onDragStart={() => setDraggedPaneId(paneId)}
                onDragEnd={() => setDraggedPaneId(null)}
                onDragOverTab={(insertAfter) => reorderWhileDragging(paneId, insertAfter)}
                onDropBefore={() => dropPane("tabs", paneId)}
                dragged={draggedPaneId === paneId}
                dragging={draggedPaneId !== null}
              />
            ))}
          </div>
        </div>
        {isPrimaryFrame ? <HeaderCenter /> : null}
        <div className="frame-tabs-end">
          {isPrimaryFrame ? <HeaderActions /> : null}
          <OpenWindowMenu frameId={frameId} />
        </div>
      </div>
      <div className="frame-body" ref={bodyRef}>
        {frame.paneIds.length === 0 ? (
          <div className="frame-empty muted">empty frame - drop a tab here or open one with [+]</div>
        ) : null}
        {draggedPaneId ? (
          <div
            className="frame-drop-zones"
            onDragLeave={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
                setHoveredDropSide(null);
              }
            }}
          >
            {/* The preview shows the half of the frame the tab would take, not the
                hit area that was hovered. */}
            {hoveredDropSide ? (
              <div className={`frame-drop-preview frame-drop-preview-${hoveredDropSide}`} />
            ) : null}
            {edgeDropSides.map((side) => (
              <div
                key={side}
                className={`frame-drop-zone frame-drop-zone-${side}`}
                onDragOver={(event) => {
                  event.preventDefault();
                  setHoveredDropSide(side);
                }}
                onDrop={(event) => {
                  event.preventDefault();
                  dropPane(side);
                }}
              />
            ))}
            <div
              className="frame-drop-zone frame-drop-zone-center"
              onDragOver={(event) => {
                event.preventDefault();
                setHoveredDropSide("tabs");
              }}
              onDrop={(event) => {
                event.preventDefault();
                dropPane("tabs");
              }}
            />
          </div>
        ) : null}
      </div>
    </div>
  );
}

type FrameTabProps = {
  pane: Pane;
  active: boolean;
  dragged: boolean;
  dragging: boolean;
  onFocus: () => void;
  onClose: () => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  onDragOverTab: (insertAfter: boolean) => void;
  onDropBefore: () => void;
};

function FrameTab({
  pane,
  active,
  dragged,
  dragging,
  onFocus,
  onClose,
  onDragStart,
  onDragEnd,
  onDragOverTab,
  onDropBefore,
}: FrameTabProps) {
  const reviewStore = useReviewStoreFor(pane.kind === "review" ? pane.sessionId : null);
  const title = paneTitle(pane, reviewStore?.state.data ?? null);
  const classes = ["frame-tab"];
  if (active) {
    classes.push("frame-tab-active");
  }
  if (dragged) {
    classes.push("frame-tab-dragged");
  }

  return (
    <div
      className={classes.join(" ")}
      draggable
      onDragStart={(event) => {
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("text/plain", pane.paneId);
        hideDragImage(event);
        onDragStart();
      }}
      onDragEnd={onDragEnd}
      onDragOver={(event) => {
        if (!dragging) {
          return;
        }
        event.preventDefault();
        const rect = event.currentTarget.getBoundingClientRect();
        onDragOverTab(event.clientX > rect.left + rect.width / 2);
      }}
      onDrop={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onDropBefore();
      }}
    >
      <button type="button" onClick={onFocus}>
        {title}
      </button>
      <button
        type="button"
        className="frame-tab-close"
        onClick={onClose}
        title={`Close ${title}`}
        aria-label={`Close ${title}`}
      >
        ×
      </button>
    </div>
  );
}
