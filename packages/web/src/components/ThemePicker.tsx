"use client";

import { THEMES, THEME_LABELS, type Theme } from "@/lib/theme";
import { useTheme } from "@/lib/useTheme";

/**
 * Theme selector.
 *
 * A plain `<select>` rather than a dropdown of swatches: it is five options in
 * a corner of the sidebar, it gets keyboard and screen-reader behaviour for
 * free, and on a phone it opens the native picker — which is the one control
 * surface where a custom menu would be actively worse.
 */
export function ThemePicker({ className = "" }: { className?: string }) {
  const { theme, setTheme } = useTheme();

  return (
    <label className={`flex items-center gap-2 text-xs ${className}`}>
      <span className="text-gray-500 dark:text-gray-400">Theme</span>
      <select
        aria-label="Theme"
        value={theme}
        onChange={(e) => setTheme(e.target.value as Theme)}
        className="flex-1 rounded border border-gray-300 bg-white px-2 py-1 text-xs text-gray-800 outline-none focus:border-gray-500 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200"
      >
        {THEMES.map((t) => (
          <option key={t} value={t}>
            {THEME_LABELS[t]}
          </option>
        ))}
      </select>
    </label>
  );
}
