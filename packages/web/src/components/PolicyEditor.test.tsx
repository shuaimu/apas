import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PolicyEditor } from "./PolicyEditor";

const PROFILES = [
  { key: "terminal:claude:official:default", label: "Claude terminal" },
  { key: "terminal:codex:official:default", label: "Codex terminal" },
];

afterEach(cleanup);

// Team mode is removed. `team_available` stays in stored policy and on the
// wire only so older builds keep parsing; it must not be a setting.
describe("PolicyEditor and the retained team field", () => {
  it.each([true, false])("offers no team control when the stored value is %s", (teamAvailable) => {
    render(
      <PolicyEditor
        title="Project policy"
        policy={{ team_available: teamAvailable, allowed_launch_profiles: [PROFILES[0].key] }}
        profiles={PROFILES}
        onSave={vi.fn()}
      />,
    );

    expect(screen.queryByText(/team/i)).toBeNull();
    // One checkbox per launch profile and nothing else.
    expect(screen.getAllByRole("checkbox")).toHaveLength(PROFILES.length);
  });

  it.each([true, false])("saves the launch profiles and sends the stored value %s back unchanged", (teamAvailable) => {
    const onSave = vi.fn();
    render(
      <PolicyEditor
        title="Project policy"
        policy={{ team_available: teamAvailable, allowed_launch_profiles: [PROFILES[0].key] }}
        profiles={PROFILES}
        onSave={onSave}
      />,
    );

    fireEvent.click(screen.getByRole("checkbox", { name: /Codex terminal/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save policy" }));

    expect(onSave).toHaveBeenCalledWith({
      team_available: teamAvailable,
      allowed_launch_profiles: [PROFILES[0].key, PROFILES[1].key],
    });
  });
});

describe("PolicyEditor on the cluster page", () => {
  it("draws a part of a section with each profile by name and the key in its title", () => {
    render(
      <PolicyEditor
        appearance="overview"
        title="Cluster default policy"
        policy={{ team_available: true, allowed_launch_profiles: [PROFILES[0].key] }}
        profiles={PROFILES}
        bound={{ team_available: true, allowed_launch_profiles: [PROFILES[0].key], version: 1, project_suspended: false }}
        onSave={vi.fn()}
        saveLabel="Save cluster policy"
      />,
    );

    expect(screen.getByRole("heading", { level: 3, name: "Cluster default policy" })).toBeTruthy();
    expect(screen.queryByText(PROFILES[0].key)).toBeNull();
    expect(screen.getByText("Claude terminal").closest("label")?.getAttribute("title")).toBe(PROFILES[0].key);
    expect(screen.getByText("Codex terminal").closest("label")?.getAttribute("title")).toBe("Not permitted by the policy above this one");
    expect(screen.getAllByRole("checkbox")).toHaveLength(PROFILES.length);
    expect(screen.queryByText(/team/i)).toBeNull();
    expect(screen.getByRole("button", { name: "Save cluster policy" })).toBeTruthy();
  });
});

describe("PolicyEditor's unsaved choices", () => {
  const stored = { team_available: false, allowed_launch_profiles: [PROFILES[0].key], version: 2 };

  it("keeps them when the same policy arrives again as a new object", () => {
    const { rerender } = render(
      <PolicyEditor appearance="overview" title="Cluster default policy" policy={{ ...stored }} profiles={PROFILES} onSave={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("checkbox", { name: /Codex terminal/ }));
    rerender(
      <PolicyEditor appearance="overview" title="Cluster default policy" policy={{ ...stored, allowed_launch_profiles: [...stored.allowed_launch_profiles] }} profiles={PROFILES} onSave={vi.fn()} />,
    );
    expect((screen.getByRole("checkbox", { name: /Codex terminal/ }) as HTMLInputElement).checked).toBe(true);
  });

  it("drops them when the stored policy changes", () => {
    const { rerender } = render(
      <PolicyEditor appearance="panel" title="Project policy" policy={stored} profiles={PROFILES} onSave={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("checkbox", { name: /Codex terminal/ }));
    rerender(
      <PolicyEditor appearance="panel" title="Project policy" policy={{ ...stored, version: 3 }} profiles={PROFILES} onSave={vi.fn()} />,
    );
    expect((screen.getByRole("checkbox", { name: /Codex terminal/ }) as HTMLInputElement).checked).toBe(false);
  });
});
