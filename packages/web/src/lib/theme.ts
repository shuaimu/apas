/**
 * UI themes.
 *
 * Tailwind is in **class** dark mode (see `globals.css`), so a theme is applied
 * by putting two things on `<html>`:
 *
 *   - `class="dark"` when the theme is a dark one, which is what all 721
 *     existing `dark:` utilities key off,
 *   - `data-theme="<id>"`, which the solarized themes use to remap the neutral
 *     palette.
 *
 * The solarized variants deliberately do **not** touch component code. Tailwind
 * v4 exposes palette colours as CSS variables (`--color-gray-700`, …), so
 * overriding those under a `[data-theme]` selector recolours the whole UI at
 * once. The app uses ~1300 `gray`/`zinc` utilities; restyling those by hand
 * would have been the entire feature, and every new component would have had to
 * remember to opt in.
 */

export const THEMES = ["system", "light", "dark", "solarized-light", "solarized-dark"] as const;
export type Theme = (typeof THEMES)[number];

export const THEME_LABELS: Record<Theme, string> = {
  system: "System",
  light: "Light",
  dark: "Dark",
  "solarized-light": "Solarized Light",
  "solarized-dark": "Solarized Dark",
};

const STORAGE_KEY = "apas_theme";

export function isTheme(value: unknown): value is Theme {
  return typeof value === "string" && (THEMES as readonly string[]).includes(value);
}

/** Whether a theme renders dark. `system` defers to the OS. */
export function themeIsDark(theme: Theme, prefersDark: boolean): boolean {
  switch (theme) {
    case "dark":
    case "solarized-dark":
      return true;
    case "light":
    case "solarized-light":
      return false;
    case "system":
      return prefersDark;
  }
}

/**
 * Apply a theme to the document.
 *
 * Exported (rather than living inside a hook) because it also runs from the
 * pre-paint inline script in `layout.tsx` — the theme has to be on `<html>`
 * before first paint or every load flashes the default theme first.
 */
export function applyTheme(theme: Theme, prefersDark: boolean): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.classList.toggle("dark", themeIsDark(theme, prefersDark));
  if (theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", theme);
  }
}

export function readStoredTheme(): Theme {
  if (typeof window === "undefined") return "system";
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    return isTheme(raw) ? raw : "system";
  } catch {
    // Private mode / storage disabled must not break rendering.
    return "system";
  }
}

export function storeTheme(theme: Theme): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // The choice just won't survive a reload.
  }
}

export function systemPrefersDark(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return true;
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/**
 * Solarized palettes for xterm.js, so a terminal pane matches the app.
 *
 * Solarized began life as a terminal scheme, so these are its published ANSI
 * values rather than an approximation. As with the built-in light theme, ANSI
 * "white" on a light background must stay dark enough to read — on solarized
 * light that is base00/base01, not base3.
 */
export const SOLARIZED = {
  base03: "#002b36",
  base02: "#073642",
  base01: "#586e75",
  base00: "#657b83",
  base0: "#839496",
  base1: "#93a1a1",
  base2: "#eee8d5",
  base3: "#fdf6e3",
  yellow: "#b58900",
  orange: "#cb4b16",
  red: "#dc322f",
  magenta: "#d33682",
  violet: "#6c71c4",
  blue: "#268bd2",
  cyan: "#2aa198",
  green: "#859900",
} as const;
