import { useRef } from "react";
import { Frame } from "./Frame";
import { PaneHost } from "./PaneHost";
import { WorkspaceSurface } from "./frameRects";
import { useWorkspace } from "./workspaceState";
import type { LayoutNode, SplitDirection } from "./layout";

const mapDirectionToPointerSize = {
  row: (event: PointerEvent, rect: DOMRect) => (event.clientX - rect.left) / rect.width,
  column: (event: PointerEvent, rect: DOMRect) => (event.clientY - rect.top) / rect.height,
} satisfies Record<SplitDirection, (event: PointerEvent, rect: DOMRect) => number>;

/// The whole app body: a tree of splits whose leaves are frames of tabs. Every pane stays
/// mounted while it exists, so a background tab keeps its scroll position and its shell.
export function Workspace() {
  const { layout } = useWorkspace();

  return (
    <WorkspaceSurface>
      <LayoutBranch node={layout.root} path={[]} />
      {Object.values(layout.panes).map((pane) => (
        <PaneHost key={pane.paneId} pane={pane} />
      ))}
    </WorkspaceSurface>
  );
}

function LayoutBranch({ node, path }: { node: LayoutNode; path: number[] }) {
  if (node.kind === "frame") {
    return <Frame frameId={node.frameId} />;
  }

  return <Split node={node} path={path} />;
}

function Split({
  node,
  path,
}: {
  node: Extract<LayoutNode, { kind: "split" }>;
  path: number[];
}) {
  const { resizeSplit } = useWorkspace();
  const containerRef = useRef<HTMLDivElement | null>(null);

  /// Dragging a divider only moves the two frames it sits between; the rest keep their size.
  function startResize(dividerIndex: number) {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    const rect = container.getBoundingClientRect();
    const before = node.sizes.slice(0, dividerIndex).reduce((sum, size) => sum + size, 0);
    const pairTotal = node.sizes[dividerIndex] + node.sizes[dividerIndex + 1];

    function handlePointerMove(event: PointerEvent) {
      const position = mapDirectionToPointerSize[node.direction](event, rect);
      const firstSize = Math.min(Math.max(position - before, 0.05), pairTotal - 0.05);
      const sizes = [...node.sizes];
      sizes[dividerIndex] = firstSize;
      sizes[dividerIndex + 1] = pairTotal - firstSize;
      resizeSplit(path, sizes);
    }

    function stopResize() {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", stopResize);
      window.removeEventListener("pointercancel", stopResize);
      document.body.classList.remove("workspace-resizing");
    }

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", stopResize);
    window.addEventListener("pointercancel", stopResize);
    document.body.classList.add("workspace-resizing");
  }

  return (
    <div className={`workspace-split workspace-split-${node.direction}`} ref={containerRef}>
      {node.children.map((child, index) => (
        <div className="workspace-split-child" key={index} style={{ flexGrow: node.sizes[index] }}>
          <LayoutBranch node={child} path={[...path, index]} />
          {index < node.children.length - 1 ? (
            <div
              className={`workspace-divider workspace-divider-${node.direction}`}
              title="Drag to resize"
              onPointerDown={(event) => {
                if (event.button !== 0) {
                  return;
                }
                event.preventDefault();
                startResize(index);
              }}
            />
          ) : null}
        </div>
      ))}
    </div>
  );
}
