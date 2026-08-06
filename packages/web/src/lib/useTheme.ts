"use client";

import { useCallback, useSyncExternalStore } from "react";
import {
  applyTheme,
  readStoredTheme,
  storeTheme,
  systemPrefersDark,
  themeIsDark,
  type Theme,
} from "@/lib/theme";

/**
 * One shared theme value for the whole app.
 *
 * This was originally per-component `useState`, which looked fine and was
 * subtly broken: the picker updated *its own* copy, wrote localStorage, and set
 * the attributes on `<html>` — so anything styled by CSS themed correctly,
 * while every other component's hook still held the old value. The terminal
 * pane, which repaints its xterm palette from JS rather than CSS, therefore
 * never changed. A module-level store plus `useSyncExternalStore` means all
 * consumers observe the same value and re-render together.
 */

type Snapshot = { theme: Theme; prefersDark: boolean };

let snapshot: Snapshot = { theme: "system", prefersDark: true };
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

function setSnapshot(next: Snapshot) {
  // Object identity is the change signal for useSyncExternalStore, so bail when
  // nothing actually differs or every emit re-renders every consumer.
  if (next.theme === snapshot.theme && next.prefersDark === snapshot.prefersDark) {
    return;
  }
  snapshot = next;
  applyTheme(snapshot.theme, snapshot.prefersDark);
  emit();
}

let initialised = false;

/**
 * Read the persisted theme and start watching the OS preference. Runs once,
 * lazily, from the first `subscribe` — not at module scope, which would touch
 * `window` during SSR.
 */
function initOnce() {
  if (initialised || typeof window === "undefined") return;
  initialised = true;
  snapshot = { theme: readStoredTheme(), prefersDark: systemPrefersDark() };
  if (typeof window.matchMedia === "function") {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    // Never removed: this lives for the page's lifetime, and there is exactly
    // one regardless of how many components use the hook.
    media.addEventListener("change", (e) =>
      setSnapshot({ ...snapshot, prefersDark: e.matches }),
    );
  }
}

function subscribe(listener: () => void): () => void {
  initOnce();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

const getSnapshot = () => snapshot;
// The server has no localStorage and no OS preference; "system" matches what
// the pre-paint script assumes before it runs.
const getServerSnapshot = (): Snapshot => ({ theme: "system", prefersDark: true });

export function setTheme(next: Theme): void {
  storeTheme(next);
  setSnapshot({ ...snapshot, theme: next });
}

export function useTheme(): {
  theme: Theme;
  setTheme: (t: Theme) => void;
  isDark: boolean;
} {
  const { theme, prefersDark } = useSyncExternalStore(
    subscribe,
    getSnapshot,
    getServerSnapshot,
  );
  return {
    theme,
    setTheme: useCallback(setTheme, []),
    isDark: themeIsDark(theme, prefersDark),
  };
}

/** Reset for tests. */
export function __resetThemeStore(): void {
  snapshot = { theme: "system", prefersDark: true };
  listeners.clear();
  initialised = false;
}
