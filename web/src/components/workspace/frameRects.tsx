import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";

/// Where a frame's content area sits inside the workspace, in pixels.
export type FrameRect = { left: number; top: number; width: number; height: number };

type FrameRectsValue = {
  frameRects: Record<string, FrameRect>;
  registerFrameBody: (frameId: string, element: HTMLElement | null) => void;
};

const FrameRectsContext = createContext<FrameRectsValue | null>(null);

/// Panes are laid out over the frames rather than inside them: a tab dragged to another
/// frame only changes coordinates, so its DOM — a live terminal, a scrolled diff — survives
/// the move untouched.
export function WorkspaceSurface({ children }: { children: ReactNode }) {
  const surfaceRef = useRef<HTMLDivElement | null>(null);
  const bodyElementsRef = useRef(new Map<string, HTMLElement>());
  const [frameRects, setFrameRects] = useState<Record<string, FrameRect>>({});

  const measureFrames = useCallback(() => {
    const surface = surfaceRef.current;
    if (!surface) {
      return;
    }

    const surfaceRect = surface.getBoundingClientRect();
    const measured: Record<string, FrameRect> = {};
    for (const [frameId, element] of bodyElementsRef.current) {
      const rect = element.getBoundingClientRect();
      measured[frameId] = {
        left: rect.left - surfaceRect.left,
        top: rect.top - surfaceRect.top,
        width: rect.width,
        height: rect.height,
      };
    }

    setFrameRects((current) => (sameRects(current, measured) ? current : measured));
  }, []);

  const observerRef = useRef<ResizeObserver | null>(null);
  if (!observerRef.current && typeof ResizeObserver !== "undefined") {
    observerRef.current = new ResizeObserver(() => measureFrames());
  }

  const registerFrameBody = useCallback(
    (frameId: string, element: HTMLElement | null) => {
      const known = bodyElementsRef.current.get(frameId);
      if (known) {
        observerRef.current?.unobserve(known);
        bodyElementsRef.current.delete(frameId);
      }
      if (element) {
        bodyElementsRef.current.set(frameId, element);
        observerRef.current?.observe(element);
      }
      measureFrames();
    },
    [measureFrames],
  );

  useEffect(() => {
    window.addEventListener("resize", measureFrames);
    return () => window.removeEventListener("resize", measureFrames);
  }, [measureFrames]);

  const value = useMemo<FrameRectsValue>(
    () => ({ frameRects, registerFrameBody }),
    [frameRects, registerFrameBody],
  );

  return (
    <FrameRectsContext.Provider value={value}>
      <div className="workspace" ref={surfaceRef}>
        {children}
      </div>
    </FrameRectsContext.Provider>
  );
}

function sameRects(left: Record<string, FrameRect>, right: Record<string, FrameRect>) {
  const leftKeys = Object.keys(left);
  if (leftKeys.length !== Object.keys(right).length) {
    return false;
  }

  return leftKeys.every((frameId) => {
    const a = left[frameId];
    const b = right[frameId];
    return (
      b && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height
    );
  });
}

export function useFrameRects() {
  const value = useContext(FrameRectsContext);
  if (!value) {
    throw new Error("useFrameRects must be used within WorkspaceSurface");
  }
  return value;
}
