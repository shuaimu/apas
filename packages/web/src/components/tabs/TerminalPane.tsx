"use client";

import React, { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { ClipboardAddon } from "@xterm/addon-clipboard";
import { FitAddon } from "@xterm/addon-fit";
import { useStore } from "@/lib/store";
import {
  subscribeTerminal,
  type TerminalEvent,
  type TerminalLifecycle,
} from "@/lib/terminalBus";
import {
  applyTerminalEvent,
  createTerminalRenderState,
  terminalLifecycleBanner,
} from "@/lib/terminalReconciler";
import { SOLARIZED as SZ, readStoredTheme, themeIsDark } from "@/lib/theme";
import { useTheme } from "@/lib/useTheme";
import {
  handleTerminalClipboardKey,
  WriteOnlyTerminalClipboardProvider,
} from "@/lib/terminalClipboard";
import "@xterm/xterm/css/xterm.css";

/**
 * Light and dark palettes for the hosted TUI.
 *
 * Both define all 16 ANSI colours, not just background/foreground. A TUI
 * paints almost everything with the ANSI palette, and xterm's built-in
 * defaults are tuned for a dark background — on white, its `brightYellow`,
 * `brightWhite`, and `brightBlack` are close to invisible. Flipping only
 * bg/fg would give a light terminal with unreadable output.
 *
 * `background` / `foreground` match the app's own CSS variables in
 * `globals.css`, so the terminal sits flush with the surrounding chrome
 * instead of looking like a pasted-in rectangle.
 */
const TERMINAL_THEMES = {
  dark: {
    background: "#0a0a0a",
    foreground: "#e5e5e5",
    cursor: "#e5e5e5",
    cursorAccent: "#0a0a0a",
    selectionBackground: "#264f78",
    black: "#1e1e1e",
    red: "#f14c4c",
    green: "#23d18b",
    yellow: "#f5f543",
    blue: "#3b8eea",
    magenta: "#d670d6",
    cyan: "#29b8db",
    white: "#e5e5e5",
    brightBlack: "#7f7f7f",
    brightRed: "#f14c4c",
    brightGreen: "#23d18b",
    brightYellow: "#f5f543",
    brightBlue: "#3b8eea",
    brightMagenta: "#d670d6",
    brightCyan: "#29b8db",
    brightWhite: "#ffffff",
  },
  light: {
    background: "#ffffff",
    foreground: "#171717",
    cursor: "#171717",
    cursorAccent: "#ffffff",
    selectionBackground: "#add6ff",
    // Darkened so they hold contrast against white. The "bright" half is
    // deliberately not brighter than the normal half here — on a light
    // background "bright" has to mean *more saturated*, or bold text
    // disappears.
    black: "#000000",
    red: "#cd3131",
    green: "#00825e",
    yellow: "#8a6d00",
    blue: "#0451a5",
    magenta: "#a1258f",
    cyan: "#0598bc",
    white: "#555555",
    brightBlack: "#666666",
    brightRed: "#cd3131",
    brightGreen: "#00825e",
    brightYellow: "#8a6d00",
    brightBlue: "#0451a5",
    brightMagenta: "#a1258f",
    brightCyan: "#0598bc",
    brightWhite: "#171717",
  },
} as const;

const SOLARIZED_THEMES = {
  // Solarized began as a terminal scheme, so these are its published ANSI
  // values rather than an approximation of the CSS palette.
  "solarized-dark": {
    background: SZ.base03,
    foreground: SZ.base0,
    cursor: SZ.base1,
    cursorAccent: SZ.base03,
    selectionBackground: SZ.base02,
    black: SZ.base02,
    red: SZ.red,
    green: SZ.green,
    yellow: SZ.yellow,
    blue: SZ.blue,
    magenta: SZ.magenta,
    cyan: SZ.cyan,
    white: SZ.base2,
    brightBlack: SZ.base01,
    brightRed: SZ.orange,
    brightGreen: SZ.base01,
    brightYellow: SZ.base00,
    brightBlue: SZ.base0,
    brightMagenta: SZ.violet,
    brightCyan: SZ.base1,
    brightWhite: SZ.base3,
  },
  "solarized-light": {
    background: SZ.base3,
    foreground: SZ.base00,
    cursor: SZ.base01,
    cursorAccent: SZ.base3,
    selectionBackground: SZ.base2,
    black: SZ.base02,
    red: SZ.red,
    green: SZ.green,
    yellow: SZ.yellow,
    blue: SZ.blue,
    magenta: SZ.magenta,
    cyan: SZ.cyan,
    // On a light background ANSI "white" must stay dark enough to read —
    // base2/base3 here would make that text vanish, the same trap the built-in
    // light theme avoids.
    white: SZ.base00,
    brightBlack: SZ.base1,
    brightRed: SZ.orange,
    brightGreen: SZ.base1,
    brightYellow: SZ.base0,
    brightBlue: SZ.base01,
    brightMagenta: SZ.violet,
    brightCyan: SZ.base00,
    brightWhite: SZ.base02,
  },
} as const;

const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * OS preference, used only to resolve the "System" theme.
 *
 * Defaults to dark when `matchMedia` is unavailable (jsdom in tests, older
 * browsers): that matches the palette the terminal shipped with.
 */
function prefersDark(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return true;
  }
  return window.matchMedia(DARK_QUERY).matches;
}

export function terminalThemeFor(dark: boolean, theme?: string) {
  if (theme === "solarized-dark" || theme === "solarized-light") {
    return SOLARIZED_THEMES[theme];
  }
  return dark ? TERMINAL_THEMES.dark : TERMINAL_THEMES.light;
}

/** The palette matching whatever theme the app is currently showing. */
function currentTerminalTheme() {
  const t = readStoredTheme();
  return terminalThemeFor(themeIsDark(t, prefersDark()), t);
}

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
  const renderStateRef = useRef(createTerminalRenderState());
  const [lifecycleView, setLifecycleView] = useState<{
    lifecycle?: TerminalLifecycle;
    status?: string;
  }>({ lifecycle: undefined });

  // Snapshot/live reconciliation state lives in a ref so reconnects do not
  // tear down the terminal (which would drop focus and scroll position).
  const lastSizeRef = useRef<{ cols: number; rows: number }>({ cols: 0, rows: 0 });

  const attachTerminal = useStore((s) => s.attachTerminal);
  const sendTerminalInput = useStore((s) => s.sendTerminalInput);
  const sendTerminalResize = useStore((s) => s.sendTerminalResize);
  const connected = useStore((s) => s.connected);
  // Subscribing to the theme is what makes the *picker* reach the terminal.
  // Watching matchMedia alone only caught OS changes, so choosing Solarized
  // left the terminal on the old palette until the OS happened to flip.
  const { theme, isDark } = useTheme();

  // Repaint whenever the resolved theme changes, for as long as the pane is
  // mounted.
  // Repainting in place beats recreating the terminal: the hosted TUI owns the
  // screen and would not know to redraw, so a rebuild would leave a blank pane
  // until the user pressed a key.
  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.theme = terminalThemeFor(isDark, theme);
    // The WebGL renderer caches glyphs with their colours baked in, so a theme
    // swap alone can leave the old palette on screen until something else
    // invalidates those cells.
    term.refresh(0, term.rows - 1);
    // Repainting in place beats recreating the terminal: the hosted TUI owns
    // the screen and would not know to redraw, so a rebuild would leave a blank
    // pane until the user pressed a key.
  }, [theme, isDark]);

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
      theme: currentTerminalTheme(),
    });
    const fit = new FitAddon();
    const clipboard = new ClipboardAddon(
      undefined,
      new WriteOnlyTerminalClipboardProvider(),
    );
    term.loadAddon(fit);
    term.loadAddon(clipboard);
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
      const accepted = applyTerminalEvent(renderStateRef.current, event, {
        write: (bytes) => term.write(bytes),
        reset: () => term.reset(),
      });
      // Output remains entirely outside React. Only low-frequency lifecycle
      // changes update component state to render or clear the status banner.
      if (accepted && event.kind !== "output") {
        setLifecycleView({
          lifecycle: renderStateRef.current.lifecycle,
          status: renderStateRef.current.status,
        });
      }
    });

    const onData = term.onData((data) => sendTerminalInput(paneId, data));
    term.attachCustomKeyEventHandler((event) =>
      handleTerminalClipboardKey(event, term.hasSelection()),
    );

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
    renderStateRef.current.snapshotSeen = false;
    renderStateRef.current.pending = [];
    attachTerminal(paneId);
    // Re-send the size: the CLI may have restarted with a default pty.
    lastSizeRef.current = { cols: 0, rows: 0 };
    applyFit();
  }, [connected, paneId, attachTerminal, applyFit]);

  const lifecycleBanner = terminalLifecycleBanner(
    lifecycleView.lifecycle,
    lifecycleView.status,
  );

  return (
    // The wrapper must track the xterm theme, or a light terminal sits inside
    // a black frame wherever the padding shows through. Tailwind's `dark:`
    // runs in `media` mode here, so it follows the same signal the palette
    // above does and the two cannot disagree.
    <div className="relative flex h-full w-full flex-col bg-white dark:bg-[#0a0a0a]">
      {lifecycleBanner && (
        <div className="border-b border-neutral-300 bg-neutral-100 px-3 py-1.5 text-xs text-amber-700 dark:border-neutral-700 dark:bg-neutral-900 dark:text-amber-400">
          {lifecycleBanner}
        </div>
      )}
      <div ref={containerRef} className="min-h-0 flex-1 overflow-hidden p-1" />
    </div>
  );
}

export default TerminalPane;
