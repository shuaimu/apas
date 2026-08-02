"use client";

import React, { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { useStore } from "@/lib/store";
import { subscribeTerminal, type TerminalEvent } from "@/lib/terminalBus";
import "@xterm/xterm/css/xterm.css";

/**
 * Renders a `PaneKind: "terminal"` pane: the provider's real interactive
 * TUI, hosted on a pty by the CLI and streamed here as raw bytes.
 *
 * Must be loaded with `ssr: false` — xterm.js touches `document` at import
 * time and Next would fail to prerender it.
 */
export function TerminalPane({ paneId }: { paneId: number }) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [exitStatus, setExitStatus] = useState<string | null>(null);

  // Snapshot/live reconciliation state. Refs rather than closure variables
  // so the reconnect effect can reset them without tearing down the
  // terminal (which would drop the user's focus and scroll position).
  const snapshotSeenRef = useRef(false);
  const pendingRef = useRef<{ bytes: Uint8Array; seq: number }[]>([]);
  const lastSizeRef = useRef<{ cols: number; rows: number }>({ cols: 0, rows: 0 });

  const attachTerminal = useStore((s) => s.attachTerminal);
  const sendTerminalInput = useStore((s) => s.sendTerminalInput);
  const sendTerminalResize = useStore((s) => s.sendTerminalResize);
  const connected = useStore((s) => s.connected);

  /** Fit to the container and report the size when it actually changed. */
  const applyFit = useCallback(() => {
    const container = containerRef.current;
    const term = termRef.current;
    const fit = fitRef.current;
    if (!container || !term || !fit) return;
    // A hidden tab measures 0x0; fitting to that would send a degenerate
    // size to the pty and make the TUI redraw at one column.
    if (container.clientWidth === 0 || container.clientHeight === 0) return;
    try {
      fit.fit();
    } catch {
      return;
    }
    const { cols, rows } = term;
    if (cols !== lastSizeRef.current.cols || rows !== lastSizeRef.current.rows) {
      lastSizeRef.current = { cols, rows };
      sendTerminalResize(paneId, cols, rows);
    }
  }, [paneId, sendTerminalResize]);

  // Terminal lifetime: created once per pane, torn down on unmount.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontFamily:
        'ui-monospace, SFMono-Regular, Menlo, Monaco, "Cascadia Mono", "Roboto Mono", monospace',
      fontSize: 13,
      // The hosted TUI owns the screen and scrolls itself; a large xterm
      // scrollback would just fight an alt-screen app.
      scrollback: 1000,
      theme: { background: "#0a0a0a", foreground: "#e5e5e5" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(container);
    termRef.current = term;
    fitRef.current = fit;

    // WebGL is a big win for full-screen repaints but is unavailable on
    // some browsers/GPUs (and in jsdom). The DOM renderer is a correct
    // fallback, so a failure here isn't worth surfacing.
    let disposed = false;
    void (async () => {
      try {
        const { WebglAddon } = await import("@xterm/addon-webgl");
        if (disposed) return;
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => webgl.dispose());
        term.loadAddon(webgl);
      } catch {
        /* DOM renderer already active */
      }
    })();

    const unsubscribe = subscribeTerminal(paneId, (event: TerminalEvent) => {
      if (event.kind === "snapshot") {
        // A truncated replay can start partway through an escape sequence;
        // reset first so a clipped sequence can't corrupt emulator state.
        if (event.truncated) term.reset();
        if (event.bytes.length) term.write(event.bytes);
        // Drop live frames the snapshot already covers, then flush the
        // rest — otherwise replay and live tail interleave and the screen
        // paints twice.
        for (const chunk of pendingRef.current) {
          if (chunk.seq > event.seq) term.write(chunk.bytes);
        }
        pendingRef.current = [];
        snapshotSeenRef.current = true;
        return;
      }
      if (event.kind === "exited") {
        setExitStatus(event.status ?? "process ended");
        return;
      }
      if (!snapshotSeenRef.current) {
        pendingRef.current.push({ bytes: event.bytes, seq: event.seq });
        return;
      }
      term.write(event.bytes);
    });

    const onData = term.onData((data) => sendTerminalInput(paneId, data));

    const observer = new ResizeObserver(() => applyFit());
    observer.observe(container);
    applyFit();
    term.focus();

    return () => {
      disposed = true;
      observer.disconnect();
      unsubscribe();
      onData.dispose();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [paneId, sendTerminalInput, applyFit]);

  // (Re)attach whenever the socket comes up. The pty kept running on the
  // CLI across a dropped browser connection, so replaying the server's
  // scrollback is what restores the screen.
  useEffect(() => {
    if (!connected) return;
    snapshotSeenRef.current = false;
    pendingRef.current = [];
    setExitStatus(null);
    attachTerminal(paneId);
    // Re-send the size: the CLI may have restarted with a default pty.
    lastSizeRef.current = { cols: 0, rows: 0 };
    applyFit();
  }, [connected, paneId, attachTerminal, applyFit]);

  return (
    <div className="relative flex h-full w-full flex-col bg-[#0a0a0a]">
      {exitStatus && (
        <div className="border-b border-neutral-700 bg-neutral-900 px-3 py-1.5 text-xs text-amber-400">
          Process ended ({exitStatus}). Reboot the pane to start a new one.
        </div>
      )}
      <div ref={containerRef} className="min-h-0 flex-1 overflow-hidden p-1" />
    </div>
  );
}

export default TerminalPane;
