import { createContext, useContext, useMemo, useState } from "react";
import type { ReactNode } from "react";

export type DockSide = "right" | "bottom";

/// Every window the dock can host. Add an entry here and in `mapWindowIdToDefinition`.
export type DockWindowId = "terminal" | "agents";

const NARROW_SCREEN_WIDTH = 900;

const mapDockToDefaultSize = {
  right: 520,
  bottom: 352,
} satisfies Record<DockSide, number>;

const mapDockToSizeLimits = {
  right: () => ({ min: 260, max: window.innerWidth - 320 }),
  bottom: () => ({ min: 120, max: window.innerHeight - 120 }),
} satisfies Record<DockSide, () => { min: number; max: number }>;

export const mapDockToOtherDock = {
  right: "bottom",
  bottom: "right",
} satisfies Record<DockSide, DockSide>;

export const mapDockToSizeFromPointer = {
  right: (event: PointerEvent) => window.innerWidth - event.clientX,
  bottom: (event: PointerEvent) => window.innerHeight - event.clientY,
} satisfies Record<DockSide, (event: PointerEvent) => number>;

export const mapDockToCssVariables = {
  right: (size: number) => ({ "--dock-window-width": `${size}px`, "--dock-window-height": "0px" }),
  bottom: (size: number) => ({ "--dock-window-width": "0px", "--dock-window-height": `${size}px` }),
} satisfies Record<DockSide, (size: number) => Record<string, string>>;

export const mapDockToPanelStyle = {
  right: (size: number) => ({ width: `${size}px` }),
  bottom: (size: number) => ({ height: `${size}px` }),
} satisfies Record<DockSide, (size: number) => Record<string, string>>;

export function clampDockSize(dock: DockSide, size: number) {
  const { min, max } = mapDockToSizeLimits[dock]();
  return Math.min(Math.max(size, min), Math.max(min, max));
}

function initialDock(): DockSide {
  return window.innerWidth < NARROW_SCREEN_WIDTH ? "bottom" : "right";
}

type DockWindowsValue = {
  openWindowIds: DockWindowId[];
  activeWindowId: DockWindowId | null;
  dock: DockSide;
  dockSize: number;
  openWindow: (windowId: DockWindowId) => void;
  closeWindow: (windowId: DockWindowId) => void;
  focusWindow: (windowId: DockWindowId) => void;
  setDock: (dock: DockSide) => void;
  setDockSize: (size: number) => void;
};

const DockWindowsContext = createContext<DockWindowsValue | null>(null);

export function DockWindowsProvider({ children }: { children: ReactNode }) {
  const [openWindowIds, setOpenWindowIds] = useState<DockWindowId[]>([]);
  const [activeWindowId, setActiveWindowId] = useState<DockWindowId | null>(null);
  const [dock, setDock] = useState<DockSide>(initialDock);
  const [dockSizes, setDockSizes] = useState<Record<DockSide, number>>(mapDockToDefaultSize);

  function openWindow(windowId: DockWindowId) {
    setOpenWindowIds((current) => (current.includes(windowId) ? current : [...current, windowId]));
    setActiveWindowId(windowId);
  }

  function closeWindow(windowId: DockWindowId) {
    setOpenWindowIds((current) => {
      const remaining = current.filter((id) => id !== windowId);
      setActiveWindowId((active) =>
        active === windowId ? (remaining[remaining.length - 1] ?? null) : active,
      );
      return remaining;
    });
  }

  const value = useMemo<DockWindowsValue>(
    () => ({
      openWindowIds,
      activeWindowId,
      dock,
      dockSize: dockSizes[dock],
      openWindow,
      closeWindow,
      focusWindow: setActiveWindowId,
      setDock,
      setDockSize: (size) => setDockSizes((current) => ({ ...current, [dock]: size })),
    }),
    [activeWindowId, dock, dockSizes, openWindowIds],
  );

  return <DockWindowsContext.Provider value={value}>{children}</DockWindowsContext.Provider>;
}

export function useDockWindows() {
  const value = useContext(DockWindowsContext);
  if (!value) {
    throw new Error("useDockWindows must be used within DockWindowsProvider");
  }
  return value;
}

export function useOptionalDockWindows() {
  return useContext(DockWindowsContext);
}
