"use client";

import { useCallback, useEffect, useState } from "react";
import {
  applyTheme,
  readStoredTheme,
  storeTheme,
  systemPrefersDark,
  themeIsDark,
  type Theme,
} from "@/lib/theme";

/**
 * The active theme, plus a setter that persists and applies it.
 *
 * Starts at `system` and syncs from storage after mount rather than reading
 * localStorage during render — the latter mismatches the server-rendered HTML
 * and trips a hydration error. The inline script in `layout.tsx` has already
 * put the correct theme on `<html>` by then, so this catches React up rather
 * than causing a visible change.
 */
export function useTheme(): {
  theme: Theme;
  setTheme: (t: Theme) => void;
  isDark: boolean;
} {
  const [theme, setThemeState] = useState<Theme>("system");
  const [prefersDark, setPrefersDark] = useState(true);

  useEffect(() => {
    setThemeState(readStoredTheme());
    setPrefersDark(systemPrefersDark());
  }, []);

  // Keep "System" live: the OS switching between light and dark should be
  // picked up without a reload.
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setPrefersDark(e.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  useEffect(() => {
    applyTheme(theme, prefersDark);
  }, [theme, prefersDark]);

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    storeTheme(next);
  }, []);

  return { theme, setTheme, isDark: themeIsDark(theme, prefersDark) };
}
