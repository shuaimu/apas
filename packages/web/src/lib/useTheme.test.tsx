import { act, render, screen, fireEvent } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useTheme, setTheme, __resetThemeStore } from "./useTheme";
import { THEMES } from "./theme";

beforeEach(() => {
  localStorage.clear();
  document.documentElement.className = "";
  document.documentElement.removeAttribute("data-theme");
  __resetThemeStore();
});

/** Stands in for the picker. */
function Picker() {
  const { theme, setTheme } = useTheme();
  return (
    <select
      data-testid="picker"
      value={theme}
      onChange={(e) => setTheme(e.target.value as (typeof THEMES)[number])}
    >
      {THEMES.map((t) => (
        <option key={t} value={t}>
          {t}
        </option>
      ))}
    </select>
  );
}

/** Stands in for TerminalPane — a *different* component reading the theme. */
function Observer() {
  const { theme, isDark } = useTheme();
  return <span data-testid="observer">{`${theme}:${isDark}`}</span>;
}

describe("useTheme is shared across components", () => {
  it("propagates a change from one component to another", () => {
    // The bug this replaces: per-component useState meant the picker updated
    // only its own copy. Anything styled by CSS still themed (the attributes on
    // <html> were set), so it looked fine — but the terminal pane, which
    // repaints its xterm palette from JS, never heard about it.
    render(
      <>
        <Picker />
        <Observer />
      </>,
    );
    expect(screen.getByTestId("observer").textContent).toBe("system:true");

    fireEvent.change(screen.getByTestId("picker"), {
      target: { value: "solarized-light" },
    });

    expect(screen.getByTestId("observer").textContent).toBe("solarized-light:false");
  });

  it("reaches a component that never rendered a picker", () => {
    render(<Observer />);
    act(() => setTheme("solarized-dark"));
    expect(screen.getByTestId("observer").textContent).toBe("solarized-dark:true");
  });

  it("applies to <html> as well as to subscribers", () => {
    render(<Observer />);
    act(() => setTheme("solarized-dark"));
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.getAttribute("data-theme")).toBe("solarized-dark");
  });

  it("persists the choice", () => {
    render(<Picker />);
    fireEvent.change(screen.getByTestId("picker"), { target: { value: "light" } });
    expect(localStorage.getItem("apas_theme")).toBe("light");
  });

  it("picks up a stored theme on first subscribe", () => {
    localStorage.setItem("apas_theme", "solarized-light");
    render(<Observer />);
    expect(screen.getByTestId("observer").textContent).toBe("solarized-light:false");
  });

  it("re-selecting the same theme does not churn subscribers", () => {
    // Identity is the change signal, so emitting on a no-op would re-render
    // every consumer — including repainting every terminal pane.
    let renders = 0;
    function Counting() {
      useTheme();
      renders++;
      return null;
    }
    render(<Counting />);
    const before = renders;
    act(() => setTheme("system"));
    expect(renders).toBe(before);
  });
});
