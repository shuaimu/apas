import { describe, expect, it } from "vitest";
import { terminalThemeFor } from "./TerminalPane";

/**
 * The palettes matter more than they look. A TUI paints with the 16 ANSI
 * colours, so a "light theme" that only flips background/foreground leaves
 * dark-tuned ANSI colours on white and the output becomes unreadable. These
 * tests pin the contrast property rather than the exact hex values.
 */

/** WCAG relative luminance, 0 (black) .. 1 (white). */
function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  const channels = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const ANSI_KEYS = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;

describe("terminalThemeFor", () => {
  it("returns a light palette on a light background and a dark one on dark", () => {
    expect(terminalThemeFor(false).background).toBe("#ffffff");
    expect(terminalThemeFor(true).background).toBe("#0a0a0a");
  });

  it("matches the app's own background variables from globals.css", () => {
    // The terminal has to sit flush with the surrounding chrome; if these
    // drift apart the pane reads as a pasted-in rectangle.
    expect(terminalThemeFor(true).background).toBe("#0a0a0a");
    expect(terminalThemeFor(false).background).toBe("#ffffff");
    expect(terminalThemeFor(false).foreground).toBe("#171717");
  });

  it.each([
    ["light", false],
    ["dark", true],
  ])("defines all 16 ANSI colours in the %s palette", (_name, dark) => {
    const theme = terminalThemeFor(dark) as Record<string, string>;
    for (const key of ANSI_KEYS) {
      expect(theme[key], `${key} missing`).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  it.each([
    ["light", false, []],
    // ANSI `black` on a dark background is conventionally near-invisible —
    // TUIs use it for shadows and dim chrome, and every terminal ships it that
    // way (VS Code Dark puts #000000 on #1e1e1e). Raising it would change how
    // existing dark terminals look, which is not something adding a light
    // theme should do. Every colour a TUI actually draws *text* with is held
    // to the floor.
    ["dark", true, ["black"]],
  ] as const)(
    "keeps every ANSI text colour legible on the %s background",
    (_name, dark, exempt) => {
      const theme = terminalThemeFor(dark) as Record<string, string>;
      const bg = theme.background;
      for (const key of ANSI_KEYS) {
        if ((exempt as readonly string[]).includes(key)) continue;
        // 3:1 is the WCAG floor for large/bold text, the closest analogue to
        // terminal glyphs.
        const ratio = contrastRatio(theme[key], bg);
        expect(ratio, `${key} (${theme[key]}) on ${bg} is only ${ratio.toFixed(2)}:1`)
          .toBeGreaterThanOrEqual(3);
      }
    },
  );

  it("makes the light palette's white/brightWhite dark enough to read", () => {
    // The trap this whole feature exists to avoid: on a light theme, ANSI
    // "white" must not be literally white or text using it vanishes. Asserted
    // separately because it is the single most likely thing to regress if
    // someone later "fixes" the palette to look more literal.
    const light = terminalThemeFor(false) as Record<string, string>;
    expect(contrastRatio(light.white, light.background)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(light.brightWhite, light.background)).toBeGreaterThanOrEqual(3);
  });

  it("keeps the cursor visible against its own background", () => {
    for (const dark of [true, false]) {
      const theme = terminalThemeFor(dark) as Record<string, string>;
      expect(contrastRatio(theme.cursor, theme.background)).toBeGreaterThanOrEqual(3);
    }
  });
});

describe("solarized terminal palettes", () => {
  it("uses solarized ANSI values, not the default light/dark ones", () => {
    const sd = terminalThemeFor(true, "solarized-dark") as Record<string, string>;
    const sl = terminalThemeFor(false, "solarized-light") as Record<string, string>;
    expect(sd.background).toBe("#002b36"); // base03
    expect(sl.background).toBe("#fdf6e3"); // base3
    expect(sd.blue).toBe("#268bd2");
    expect(sl.blue).toBe("#268bd2");
  });

  it("keeps solarized's *accent* colours legible on their own background", () => {
    // Solarized's ANSI mapping repurposes the bright slots for its base tones
    // (brightGreen=base01, brightYellow=base00, brightBlue=base0,
    // brightCyan=base1), and black/brightBlack are the dim-chrome slots. Those
    // are meant to be low contrast — they are the scheme's de-emphasis colours,
    // what it uses for comments. Only the actual accents are held to a floor.
    //
    // That floor is 2.9, not the 3:1 used for the built-in palettes. Solarized
    // is designed around uniform CIELAB lightness rather than WCAG ratios, so
    // its accents land at 2.93–2.98 on base3. Raising them would mean shipping
    // something that is not Solarized, which is the one thing a theme by that
    // name must not do. Measured, not guessed: min accent is cyan at 2.93.
    const BASE_TONE_SLOTS = [
      "black",
      "brightBlack",
      "brightGreen",
      "brightYellow",
      "brightBlue",
      "brightCyan",
    ];
    const ACCENT_FLOOR = 2.9;

    for (const [name, dark] of [
      ["solarized-dark", true],
      ["solarized-light", false],
    ] as [string, boolean][]) {
      const theme = terminalThemeFor(dark, name) as Record<string, string>;
      for (const key of ANSI_KEYS) {
        if (BASE_TONE_SLOTS.includes(key)) continue;
        const ratio = contrastRatio(theme[key], theme.background);
        expect(ratio, `${name} ${key} (${theme[key]}) is only ${ratio.toFixed(2)}:1`)
          .toBeGreaterThanOrEqual(ACCENT_FLOOR);
      }
    }
  });

  it("keeps even the dim base-tone slots distinguishable from the background", () => {
    // They are allowed to be low contrast, but not invisible: an actually
    // invisible colour comes out near 1:1. `black` on solarized-dark is the
    // conventional exception every terminal ships (it is base02 on base03).
    for (const [name, dark] of [
      ["solarized-dark", true],
      ["solarized-light", false],
    ] as [string, boolean][]) {
      const theme = terminalThemeFor(dark, name) as Record<string, string>;
      for (const key of ["brightBlack", "brightGreen", "brightYellow", "brightCyan"]) {
        const ratio = contrastRatio(theme[key], theme.background);
        expect(ratio, `${name} ${key} is ${ratio.toFixed(2)}:1 — effectively invisible`)
          .toBeGreaterThanOrEqual(2);
      }
    }
  });

  it("solarized-light's ANSI white stays dark enough to read", () => {
    // The same trap as the built-in light theme: base2/base3 here would make
    // any text using the white slot vanish.
    const sl = terminalThemeFor(false, "solarized-light") as Record<string, string>;
    expect(contrastRatio(sl.white, sl.background)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(sl.brightWhite, sl.background)).toBeGreaterThanOrEqual(3);
  });

  it("falls back to the built-in palettes for non-solarized themes", () => {
    expect(terminalThemeFor(true, "dark").background).toBe("#0a0a0a");
    expect(terminalThemeFor(false, "light").background).toBe("#ffffff");
    expect(terminalThemeFor(true, "system").background).toBe("#0a0a0a");
    expect(terminalThemeFor(false).background).toBe("#ffffff");
  });
});
