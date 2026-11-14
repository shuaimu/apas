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
