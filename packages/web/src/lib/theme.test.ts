import { describe, expect, it, beforeEach, vi } from "vitest";
import {
  THEMES,
  applyTheme,
  isTheme,
  readStoredTheme,
  storeTheme,
  themeIsDark,
  type Theme,
} from "./theme";

beforeEach(() => {
  localStorage.clear();
  document.documentElement.className = "";
  document.documentElement.removeAttribute("data-theme");
  vi.restoreAllMocks();
});

describe("themeIsDark", () => {
  it.each([
    ["dark", false, true],
    ["solarized-dark", false, true],
    ["light", true, false],
    ["solarized-light", true, false],
  ] as [Theme, boolean, boolean][])(
    "%s ignores the OS preference (prefersDark=%s) → dark=%s",
    (theme, prefersDark, expected) => {
      // An explicit choice must win over the OS, or picking Light on a
      // dark-mode machine would do nothing.
      expect(themeIsDark(theme, prefersDark)).toBe(expected);
    },
  );

  it("system follows the OS in both directions", () => {
    expect(themeIsDark("system", true)).toBe(true);
    expect(themeIsDark("system", false)).toBe(false);
  });
});

describe("applyTheme", () => {
  it("drives the `dark` class, which every dark: utility keys off", () => {
    const root = document.documentElement;
    applyTheme("dark", false);
    expect(root.classList.contains("dark")).toBe(true);
    applyTheme("light", true);
    expect(root.classList.contains("dark")).toBe(false);
  });

  it("sets data-theme for explicit themes and clears it for system", () => {
    const root = document.documentElement;
    applyTheme("solarized-light", false);
    expect(root.getAttribute("data-theme")).toBe("solarized-light");

    // System must remove it, or the CSS `:root:not([data-theme])` rule that
    // follows prefers-color-scheme never applies.
    applyTheme("system", true);
    expect(root.hasAttribute("data-theme")).toBe(false);
  });

  it("solarized-dark sets both the class and the attribute", () => {
    // Solarized dark needs `dark` for the utilities AND `data-theme` for the
    // palette remap; either alone renders wrong.
    const root = document.documentElement;
    applyTheme("solarized-dark", false);
    expect(root.classList.contains("dark")).toBe(true);
    expect(root.getAttribute("data-theme")).toBe("solarized-dark");
  });

  it("does not clobber unrelated classes on <html>", () => {
    const root = document.documentElement;
    root.classList.add("some-app-class");
    applyTheme("dark", false);
    applyTheme("light", false);
    expect(root.classList.contains("some-app-class")).toBe(true);
  });
});

describe("persistence", () => {
  it("round-trips every theme", () => {
    for (const t of THEMES) {
      storeTheme(t);
      expect(readStoredTheme()).toBe(t);
    }
  });

  it("falls back to system for junk or absent values", () => {
    expect(readStoredTheme()).toBe("system");
    localStorage.setItem("apas_theme", "chartreuse");
    expect(readStoredTheme()).toBe("system");
  });

  it("survives storage that throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(readStoredTheme()).toBe("system");

    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    expect(() => storeTheme("dark")).not.toThrow();
  });
});

describe("isTheme", () => {
  it("accepts exactly the known themes", () => {
    for (const t of THEMES) expect(isTheme(t)).toBe(true);
    for (const bad of ["", "Dark", "solarized", null, undefined, 3]) {
      expect(isTheme(bad)).toBe(false);
    }
  });
});

describe("the pre-paint script in layout.tsx", () => {
  it("agrees with applyTheme for every theme", () => {
    // The inline script duplicates this logic because it must run before React.
    // If the two drift, the page paints one theme and then snaps to another.
    const inline = (t: string, osDark: boolean) => {
      const isDark = t === "dark" || t === "solarized-dark" || (t === "system" && osDark);
      return { isDark, attr: t === "system" ? null : t };
    };

    for (const t of THEMES) {
      for (const osDark of [true, false]) {
        applyTheme(t, osDark);
        const expected = inline(t, osDark);
        expect(document.documentElement.classList.contains("dark")).toBe(expected.isDark);
        expect(document.documentElement.getAttribute("data-theme")).toBe(expected.attr);
      }
    }
  });
});
