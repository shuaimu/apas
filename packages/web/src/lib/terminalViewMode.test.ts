import { describe, expect, it, beforeEach, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useTerminalViewMode, viewModeKey } from "./terminalViewMode";

const STORAGE_KEY = "apas_terminal_view_mode";

beforeEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("viewModeKey", () => {
  it("scopes a pane to its session", () => {
    // The same pane id exists in every project; a bare pane id would make one
    // project's choice leak into another's.
    expect(viewModeKey("s1", 5)).not.toBe(viewModeKey("s2", 5));
    expect(viewModeKey(null, 5)).toBe("none:5");
  });
});

describe("useTerminalViewMode", () => {
  it("defaults to the terminal, which is what the pane actually is", () => {
    // Defaulting to the lagging read-only view would look like the terminal
    // had broken.
    const { result } = renderHook(() => useTerminalViewMode("s1", 5));
    expect(result.current[0]).toBe("terminal");
  });

  it("persists a choice and restores it", () => {
    const { result } = renderHook(() => useTerminalViewMode("s1", 5));
    act(() => result.current[1]("conversation"));
    expect(result.current[0]).toBe("conversation");

    const again = renderHook(() => useTerminalViewMode("s1", 5));
    expect(again.result.current[0]).toBe("conversation");
  });

  it("keeps panes independent", () => {
    // Watching one agent work while reading another's transcript is the
    // normal case, so a single global flag would fight the user constantly.
    const a = renderHook(() => useTerminalViewMode("s1", 5));
    act(() => a.result.current[1]("conversation"));

    const b = renderHook(() => useTerminalViewMode("s1", 6));
    expect(b.result.current[0]).toBe("terminal");

    const other = renderHook(() => useTerminalViewMode("s2", 5));
    expect(other.result.current[0]).toBe("terminal");
  });

  it("survives corrupt storage instead of taking the pane down", () => {
    localStorage.setItem(STORAGE_KEY, "{not json");
    const { result } = renderHook(() => useTerminalViewMode("s1", 5));
    expect(result.current[0]).toBe("terminal");
  });

  it("ignores a stored value that is not a known mode", () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ "s1:5": "wat" }));
    const { result } = renderHook(() => useTerminalViewMode("s1", 5));
    expect(result.current[0]).toBe("terminal");
  });

  it("still switches when storage refuses writes", () => {
    // Private mode / quota. The choice must apply now even if it cannot be
    // remembered.
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota");
    });
    const { result } = renderHook(() => useTerminalViewMode("s1", 5));
    act(() => result.current[1]("conversation"));
    expect(result.current[0]).toBe("conversation");
  });
});
