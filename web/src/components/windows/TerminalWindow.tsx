import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { terminalSocketUrl } from "../../api";

// The terminal stays dark regardless of the app theme.
const terminalTheme = {
  background: "#0d121a",
  foreground: "#edf3fb",
  cursor: "#ee8d68",
  selectionBackground: "rgba(238, 141, 104, 0.3)",
};

export function TerminalWindow() {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  // Each connection spawns its own shell, so restarting means remounting on a new generation.
  const [shellGeneration, setShellGeneration] = useState(0);
  const [shellExited, setShellExited] = useState(false);

  function restartShell() {
    setShellExited(false);
    setShellGeneration((current) => current + 1);
  }

  // Create the terminal and its shell connection, and tear both down when the window
  // closes or the shell is restarted.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
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
    terminal.open(container);
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
      terminal.write("\r\n\x1b[2m[shell exited — press enter to start a new one]\x1b[0m\r\n");
      setShellExited(true);
    });

    terminal.onData((data) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: "input", data }));
        return;
      }
      // The shell is gone: enter and space restart it instead of typing into nothing.
      if (data === "\r" || data === " ") {
        restartShell();
      }
    });
    terminal.onResize(({ cols, rows }) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: "resize", cols, rows }));
      }
    });

    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;

    return () => {
      socket.close();
      terminal.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, [shellGeneration]);

  // Refit whenever the dock resizes us or brings this tab back to the front.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    const observer = new ResizeObserver(() => {
      if (container.clientWidth === 0 || container.clientHeight === 0) {
        return;
      }
      fitAddonRef.current?.fit();
      terminalRef.current?.focus();
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  return (
    <div className="terminal-window">
      {shellExited ? (
        <div className="terminal-window-restart">
          <span className="muted">shell exited</span>
          <button type="button" onClick={restartShell}>
            [new shell]
          </button>
        </div>
      ) : null}
      <div className="terminal-window-surface" ref={containerRef} />
    </div>
  );
}
