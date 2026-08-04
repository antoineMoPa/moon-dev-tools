import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { fetchTerminalIds, terminalSocketUrl } from "../../api";
import { usePaneVisible } from "./PaneHost";
import { useWorkspace } from "./workspaceState";
import type { TerminalCommand } from "./layout";

// The terminal stays dark regardless of the app theme.
const terminalTheme = {
  background: "#0d121a",
  foreground: "#edf3fb",
  cursor: "#ee8d68",
  selectionBackground: "rgba(238, 141, 104, 0.3)",
};

/// A view onto a shell that runs in the moonreview server. Closing the tab detaches this
/// socket but leaves the shell running, so reopening it resumes where it left off.
export function TerminalPane({
  paneId,
  terminalId,
  command,
}: {
  paneId: string;
  terminalId: string;
  command?: TerminalCommand;
}) {
  const { restartTerminalPane, closePaneById } = useWorkspace();
  const paneVisible = usePaneVisible();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const [shellExited, setShellExited] = useState(false);

  // Attach to the shell, and detach when the tab closes or moves to another shell.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    setShellExited(false);
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
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown" || event.altKey || event.shiftKey) {
        return true;
      }

      const shortcuts: Record<string, string> = event.metaKey && !event.ctrlKey
        ? { ArrowLeft: "\x01", ArrowRight: "\x05", Backspace: "\x15" }
        : event.ctrlKey && !event.metaKey
          ? { ArrowLeft: "\x1bb", ArrowRight: "\x1bf", Backspace: "\x17" }
          : {};
      const input = shortcuts[event.key];
      if (input === undefined) {
        return true;
      }

      event.preventDefault();
      terminal.input(input);
      return false;
    });
    if (container.clientWidth > 0 && container.clientHeight > 0) {
      fitAddon.fit();
    }

    const socket = new WebSocket(terminalSocketUrl(terminalId));
    socket.binaryType = "arraybuffer";
    const decoder = new TextDecoder();

    socket.addEventListener("open", () => {
      socket.send(JSON.stringify({ type: "resize", cols: terminal.cols, rows: terminal.rows }));
    });
    socket.addEventListener("message", (event) => {
      terminal.write(decoder.decode(event.data as ArrayBuffer, { stream: true }));
    });
    socket.addEventListener("close", () => {
      // The socket drops both when the shell exits and when the server goes away. Only the
      // first means the tab has nothing left to show, so ask which one it was.
      void fetchTerminalIds()
        .then(({ terminal_ids }) => {
          if (terminal_ids.includes(terminalId)) {
            terminal.write(
              `\r\n\x1b[2m[disconnected — press enter for a new ${command ?? "shell"}]\x1b[0m\r\n`,
            );
            setShellExited(true);
            return;
          }
          closePaneById(paneId);
        })
        .catch(() => {
          terminal.write(
            `\r\n\x1b[2m[disconnected — press enter for a new ${command ?? "shell"}]\x1b[0m\r\n`,
          );
          setShellExited(true);
        });
    });

    terminal.onData((data) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: "input", data }));
        return;
      }
      // The shell is gone: enter and space start a new one instead of typing into nothing.
      if (data === "\r" || data === " ") {
        restartTerminalPane(paneId);
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
  }, [terminalId]);

  // Bringing the tab to the front puts the cursor back in the shell.
  useEffect(() => {
    if (paneVisible) {
      terminalRef.current?.focus();
    }
  }, [paneVisible, terminalId]);

  // Refit whenever the frame resizes us or brings this tab back to the front.
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
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  return (
    <div className="terminal-pane" onPointerDown={() => terminalRef.current?.focus()}>
      {shellExited ? (
        <div className="terminal-pane-restart">
          <span className="muted">disconnected</span>
          <button type="button" onClick={() => restartTerminalPane(paneId)}>
            [new {command ?? "shell"}]
          </button>
        </div>
      ) : null}
      <div className="terminal-pane-surface" ref={containerRef} />
    </div>
  );
}
