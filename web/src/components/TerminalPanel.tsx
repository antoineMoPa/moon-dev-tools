import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { terminalSocketUrl } from "../api";

type TerminalDock = "right" | "bottom";

type TerminalPanelProps = {
  open: boolean;
  onClose: () => void;
};

// The terminal stays dark regardless of the app theme.
const terminalTheme = {
  background: "#0d121a",
  foreground: "#edf3fb",
  cursor: "#ee8d68",
  selectionBackground: "rgba(238, 141, 104, 0.3)",
};

const NARROW_SCREEN_WIDTH = 900;

const mapDockToOtherDock = {
  right: "bottom",
  bottom: "right",
} satisfies Record<TerminalDock, TerminalDock>;

const mapDockToDefaultSize = {
  right: 520,
  bottom: 352,
};

const mapDockToSizeLimits = {
  right: () => ({ min: 260, max: window.innerWidth - 320 }),
  bottom: () => ({ min: 120, max: window.innerHeight - 120 }),
};

const mapDockToSizeFromPointer = {
  right: (event: PointerEvent) => window.innerWidth - event.clientX,
  bottom: (event: PointerEvent) => window.innerHeight - event.clientY,
};

const mapDockToCssVariables = {
  right: (size: number) => ({ "--terminal-width": `${size}px`, "--terminal-height": "0px" }),
  bottom: (size: number) => ({ "--terminal-width": "0px", "--terminal-height": `${size}px` }),
};

const mapDockToPanelStyle = {
  right: (size: number) => ({ width: `${size}px` }),
  bottom: (size: number) => ({ height: `${size}px` }),
};

function clampDockSize(dock: TerminalDock, size: number) {
  const { min, max } = mapDockToSizeLimits[dock]();
  return Math.min(Math.max(size, min), Math.max(min, max));
}

function initialDock(): TerminalDock {
  return window.innerWidth < NARROW_SCREEN_WIDTH ? "bottom" : "right";
}

export function TerminalPanel({ open, onClose }: TerminalPanelProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const [dock, setDock] = useState<TerminalDock>(initialDock);
  const [dockSizes, setDockSizes] = useState(mapDockToDefaultSize);
  const [resizing, setResizing] = useState(false);
  const dockSize = dockSizes[dock];
  const otherDock = mapDockToOtherDock[dock];

  // Create the terminal and its shell connection once the panel is first opened.
  useEffect(() => {
    if (!open || terminalRef.current || !containerRef.current) {
      return;
    }

    const terminal = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      fontSize: 12,
      theme: terminalTheme,
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(containerRef.current);
    fitAddon.fit();

    const socket = new WebSocket(terminalSocketUrl());
    socket.binaryType = "arraybuffer";
    const decoder = new TextDecoder();

    socket.addEventListener("open", () => {
      socket.send(JSON.stringify({ type: "resize", cols: terminal.cols, rows: terminal.rows }));
      terminal.focus();
    });
    socket.addEventListener("message", (event) => {
      terminal.write(decoder.decode(event.data as ArrayBuffer, { stream: true }));
    });
    socket.addEventListener("close", () => {
      terminal.write("\r\n\x1b[2m[shell exited]\x1b[0m\r\n");
    });

    terminal.onData((data) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: "input", data }));
      }
    });
    terminal.onResize(({ cols, rows }) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: "resize", cols, rows }));
      }
    });

    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;
    socketRef.current = socket;
  }, [open]);

  // Tear the shell down when the whole panel unmounts.
  useEffect(() => {
    return () => {
      socketRef.current?.close();
      terminalRef.current?.dispose();
      socketRef.current = null;
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, []);

  // Let the page layout reserve the space the terminal occupies.
  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("terminal-open", open);
    const variables = open
      ? mapDockToCssVariables[dock](dockSize)
      : { "--terminal-width": "0px", "--terminal-height": "0px" };
    for (const [name, value] of Object.entries(variables)) {
      root.style.setProperty(name, value);
    }
    return () => {
      root.classList.remove("terminal-open");
      root.style.removeProperty("--terminal-width");
      root.style.removeProperty("--terminal-height");
    };
  }, [dock, dockSize, open]);

  // Refit after window resizes, dock changes, drags, and when the panel becomes visible again.
  useEffect(() => {
    if (!open) {
      return;
    }

    function refit() {
      setDockSizes((current) => ({ ...current, [dock]: clampDockSize(dock, current[dock]) }));
      fitAddonRef.current?.fit();
    }

    refit();
    terminalRef.current?.focus();
    window.addEventListener("resize", refit);
    return () => window.removeEventListener("resize", refit);
  }, [dock, dockSize, open]);

  // Track the drag on the window so the pointer can leave the fringe while resizing.
  useEffect(() => {
    if (!resizing) {
      return;
    }

    function handlePointerMove(event: PointerEvent) {
      const size = clampDockSize(dock, mapDockToSizeFromPointer[dock](event));
      setDockSizes((current) => ({ ...current, [dock]: size }));
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

  const panelClasses = ["terminal-panel", `terminal-panel-${dock}`];
  const fringeClasses = ["terminal-panel-fringe"];
  if (resizing) {
    fringeClasses.push("terminal-panel-fringe-resizing");
  }

  return (
    <div className={panelClasses.join(" ")} hidden={!open} style={mapDockToPanelStyle[dock](dockSize)}>
      <div
        className={fringeClasses.join(" ")}
        title="Drag to resize terminal"
        onPointerDown={(event) => {
          if (event.button !== 0) {
            return;
          }
          event.preventDefault();
          setResizing(true);
        }}
      />
      <div className="terminal-panel-body">
        <div className="terminal-panel-bar">
          <span className="terminal-panel-title">terminal</span>
          <div className="terminal-panel-bar-actions">
            <button
              type="button"
              className="terminal-panel-dock-toggle"
              onClick={() => setDock(otherDock)}
              title={`Dock terminal to the ${otherDock}`}
            >
              [{otherDock}]
            </button>
            <button type="button" className="terminal-panel-close" onClick={onClose} title="Close terminal">
              [close]
            </button>
          </div>
        </div>
        <div className="terminal-panel-surface" ref={containerRef} />
      </div>
    </div>
  );
}
