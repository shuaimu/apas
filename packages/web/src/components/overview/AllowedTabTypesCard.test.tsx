import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { AllowedTabTypesCard } from "./AllowedTabTypesCard";
import { ALL_TAB_TYPES } from "@/lib/tabTypes";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getState();

const FLAGS = {
  autoApproveTodos: false,
  autoMergePrs: false,
  teamEnabled: false,
  disallowedTabTypes: [] as string[],
};

function box(label: string): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement;
}

function seed(
  disallowedTabTypes: string[] = [],
  sessionOverrides: Record<string, unknown> = {},
  mock?: unknown,
) {
  act(() => {
    useStore.setState({
      sessionId: "session-a",
      sessions: [
        { id: "session-a", projectId: "session-a", status: "active", isShared: false, ...sessionOverrides },
      ] as StoreState["sessions"],
      projectFlags: { "session-a": { ...FLAGS, disallowedTabTypes } },
      ...(mock ? { updateProjectFlags: mock as StoreState["updateProjectFlags"] } : {}),
      showToast: vi.fn() as StoreState["showToast"],
    });
  });
}

afterEach(() => {
  vi.restoreAllMocks();
  act(() => {
    useStore.setState({
      sessionId: null,
      sessions: [],
      projectFlags: {},
      updateProjectFlags: initialStore.updateProjectFlags as StoreState["updateProjectFlags"],
    });
  });
});

describe("AllowedTabTypesCard", () => {
  it("shows every creatable tab type, all ticked when nothing is disallowed", () => {
    seed([]);
    render(<AllowedTabTypesCard />);
    for (const t of ALL_TAB_TYPES) {
      expect(box(t.label).checked, `${t.label} should be allowed`).toBe(true);
    }
  });

  it("does not offer MiniMax/GLM/DeepSeek as separate types", () => {
    // They arrive as `provider: claude` with a model, so a checkbox for them
    // would silently do nothing.
    seed([]);
    render(<AllowedTabTypesCard />);
    for (const absent of ["MiniMax", "GLM", "DeepSeek"]) {
      expect(screen.queryByLabelText(absent)).toBeNull();
    }
  });

  it("unticking a type adds exactly that key to the deny list", () => {
    const updateProjectFlags = vi.fn();
    seed([], {}, updateProjectFlags);
    render(<AllowedTabTypesCard />);

    fireEvent.click(box("Claude terminal"));

    expect(updateProjectFlags).toHaveBeenCalledWith(
      expect.objectContaining({ disallowedTabTypes: ["terminal:claude"] }),
    );
  });

  it("re-ticking removes only that key", () => {
    const updateProjectFlags = vi.fn();
    seed(["terminal:claude", "agent:opencode"], {}, updateProjectFlags);
    render(<AllowedTabTypesCard />);

    expect(box("Claude terminal").checked).toBe(false);
    // The sibling must be unaffected — that distinction is the point.
    expect(box("Claude").checked).toBe(true);

    fireEvent.click(box("Claude terminal"));
    expect(updateProjectFlags).toHaveBeenCalledWith(
      expect.objectContaining({ disallowedTabTypes: ["agent:opencode"] }),
    );
  });

  it("preserves the other project flags when changing the policy", () => {
    // The wire message carries every flag; dropping them here would reset
    // team mode as a side effect of ticking a checkbox.
    const updateProjectFlags = vi.fn();
    act(() => {
      useStore.setState({
        sessionId: "session-a",
        sessions: [
          { id: "session-a", projectId: "session-a", status: "active", isShared: false },
        ] as StoreState["sessions"],
        projectFlags: {
          "session-a": {
            autoApproveTodos: true,
            autoMergePrs: true,
            teamEnabled: true,
            disallowedTabTypes: [],
          },
        },
        updateProjectFlags: updateProjectFlags as StoreState["updateProjectFlags"],
        showToast: vi.fn() as StoreState["showToast"],
      });
    });
    render(<AllowedTabTypesCard />);

    fireEvent.click(box("Codex"));

    expect(updateProjectFlags).toHaveBeenCalledWith({
      autoApproveTodos: true,
      autoMergePrs: true,
      teamEnabled: true,
      disallowedTabTypes: ["agent:codex"],
    });
  });

  it("confirms before disallowing the very last type", () => {
    const updateProjectFlags = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    // Everything already off except one.
    const allButOne = ALL_TAB_TYPES.slice(1).map((t) => t.key);
    seed(allButOne, {}, updateProjectFlags);
    render(<AllowedTabTypesCard />);

    fireEvent.click(box(ALL_TAB_TYPES[0].label));

    expect(confirm).toHaveBeenCalled();
    expect(updateProjectFlags).not.toHaveBeenCalled();
  });

  it("warns when nothing is allowed", () => {
    seed(ALL_TAB_TYPES.map((t) => t.key));
    render(<AllowedTabTypesCard />);
    expect(screen.getByText(/cannot create any new tab/)).toBeTruthy();
  });

  describe("owner/admin gating", () => {
    it.each([
      ["owner", true],
      ["admin", true],
      ["user", false],
    ])("shared project role %s can edit: %s", (role, expected) => {
      seed([], { isShared: true, shareRole: role });
      render(<AllowedTabTypesCard />);
      expect(box("Claude").disabled).toBe(!expected);
    });

    it("a plain user's click sends nothing", () => {
      const updateProjectFlags = vi.fn();
      seed([], { isShared: true, shareRole: "user" }, updateProjectFlags);
      render(<AllowedTabTypesCard />);

      fireEvent.click(box("Claude"));

      expect(updateProjectFlags).not.toHaveBeenCalled();
      expect(screen.getByText(/only the project owner or an admin/)).toBeTruthy();
    });
  });
});
