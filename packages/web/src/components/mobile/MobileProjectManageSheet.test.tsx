import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { ALL_TAB_TYPES } from "@/lib/tabTypes";
import { MobileProjectManageSheet } from "./MobileProjectManageSheet";

const initialStore = useStore.getState();

const EVERY_PROFILE = ALL_TAB_TYPES.map((option) => `${option.key}:official:default`);

function seed({
  role = "owner" as "owner" | "user",
  disallowedTabTypes = [] as string[],
  teamEnabled = true,
  allowedLaunchProfiles = EVERY_PROFILE,
} = {}) {
  const updateProjectFlags = vi.fn();
  act(() => {
    useStore.setState({
      sessionId: "session-a",
      sessions: [{
        id: "session-a",
        projectId: "project-a",
        workingDir: "/workspace/alpha",
        status: "active",
        isActive: true,
        isShared: role !== "owner",
        shareRole: role,
      }] as never,
      projectPolicies: {
        "session-a": {
          teamAvailable: teamEnabled,
          allowedLaunchProfiles,
          version: 4,
          projectSuspended: false,
          noncompliantPaneIds: [],
        },
      },
      projectFlags: {
        "session-a": {
          autoApproveTodos: true,
          autoMergePrs: false,
          teamEnabled,
          disallowedTabTypes,
        },
      },
      updateProjectFlags,
    });
  });
  return { updateProjectFlags };
}

afterEach(() => {
  cleanup();
  act(() => useStore.setState(initialStore, true));
});

describe("MobileProjectManageSheet", () => {
  it("shows every tab type permitted when the project has never been restricted", () => {
    // Stored as a deny list precisely so this is the default: an allow list on
    // the wire would make an absent field mean "nothing permitted".
    seed({ disallowedTabTypes: [] });
    render(<MobileProjectManageSheet onClose={vi.fn()} />);

    for (const option of ALL_TAB_TYPES) {
      expect(
        screen.getByRole("switch", { name: option.label }).getAttribute("aria-checked"),
        option.key,
      ).toBe("true");
    }
  });

  it("reflects an existing restriction as that one type being off", () => {
    const target = ALL_TAB_TYPES[0];
    seed({ disallowedTabTypes: [target.key] });
    render(<MobileProjectManageSheet onClose={vi.fn()} />);

    expect(screen.getByRole("switch", { name: target.label }).getAttribute("aria-checked")).toBe("false");
    expect(
      screen.getByRole("switch", { name: ALL_TAB_TYPES[1].label }).getAttribute("aria-checked"),
    ).toBe("true");
  });

  it("writes the complementary deny list and preserves the other project flags", () => {
    // This path replaces every flag, so sending only tab types would silently
    // reset team mode and the autonomy switches.
    const target = ALL_TAB_TYPES[0];
    const { updateProjectFlags } = seed({ disallowedTabTypes: [], teamEnabled: true });
    render(<MobileProjectManageSheet onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("switch", { name: target.label }));

    expect(updateProjectFlags).toHaveBeenCalledWith({
      autoApproveTodos: true,
      autoMergePrs: false,
      teamEnabled: true,
      disallowedTabTypes: [target.key],
    });
  });

  it("turning a type back on removes it from the deny list", () => {
    const target = ALL_TAB_TYPES[0];
    const { updateProjectFlags } = seed({ disallowedTabTypes: [target.key, ALL_TAB_TYPES[1].key] });
    render(<MobileProjectManageSheet onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("switch", { name: target.label }));

    expect(updateProjectFlags).toHaveBeenCalledWith(
      expect.objectContaining({ disallowedTabTypes: [ALL_TAB_TYPES[1].key] }),
    );
  });

  it("is read-only for someone who cannot manage the project", () => {
    const target = ALL_TAB_TYPES[0];
    const { updateProjectFlags } = seed({ role: "user" });
    render(<MobileProjectManageSheet onClose={vi.fn()} />);

    const toggle = screen.getByRole("switch", { name: target.label });
    expect(toggle.hasAttribute("disabled")).toBe(true);
    fireEvent.click(toggle);
    expect(updateProjectFlags).not.toHaveBeenCalled();
    expect(screen.getByText(/Only the project owner can change these/)).toBeTruthy();
  });

  it("does not present a type the cluster policy forbids as permitted", () => {
    // The bug this fixes: an empty project deny list read as "everything
    // permitted" while the create menu offered one option, because the
    // deployment default narrows to a single launch profile above it.
    const only = ALL_TAB_TYPES[1];
    const { updateProjectFlags } = seed({
      disallowedTabTypes: [],
      allowedLaunchProfiles: [`${only.key}:official:default`],
    });
    render(<MobileProjectManageSheet onClose={vi.fn()} />);

    for (const option of ALL_TAB_TYPES) {
      const toggle = screen.getByRole("switch", { name: option.label });
      const permitted = option.key === only.key;
      expect(toggle.getAttribute("aria-checked"), option.key).toBe(String(permitted));
      expect(toggle.hasAttribute("disabled"), option.key).toBe(!permitted);
    }

    // Not even an owner can permit it here — the restriction is above them.
    fireEvent.click(screen.getByRole("switch", { name: ALL_TAB_TYPES[0].label }));
    expect(updateProjectFlags).not.toHaveBeenCalled();
    expect(screen.getAllByText(/Not allowed by cluster policy/).length).toBe(ALL_TAB_TYPES.length - 1);
  });

  it("a type inside the cluster policy stays the owner's to change", () => {
    const only = ALL_TAB_TYPES[1];
    const { updateProjectFlags } = seed({
      disallowedTabTypes: [],
      allowedLaunchProfiles: [`${only.key}:official:default`],
    });
    render(<MobileProjectManageSheet onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("switch", { name: only.label }));
    expect(updateProjectFlags).toHaveBeenCalledWith(
      expect.objectContaining({ disallowedTabTypes: [only.key] }),
    );
  });

  it("waits rather than showing an unrestricted project before settings arrive", () => {
    // Rendering every type as permitted before the flags land would show a
    // restricted project as open until the snapshot corrected it.
    act(() => {
      useStore.setState({ sessionId: "session-a", projectFlags: {} });
    });
    render(<MobileProjectManageSheet onClose={vi.fn()} />);

    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.getByText(/Waiting for this project/)).toBeTruthy();
  });
});
