import { fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import { PaneDiffModal } from "./TabbedView";

const DIFF = {
  branch: "feature/example",
  base: "master",
  diff: "diff --git a/file.txt b/file.txt\n+hello\n",
  fetchedAt: 1,
};

function renderModal(overrides: Partial<ComponentProps<typeof PaneDiffModal>> = {}) {
  return render(
    <PaneDiffModal
      open
      diff={DIFF}
      onClose={vi.fn()}
      onRefresh={vi.fn()}
      onMerge={vi.fn()}
      onDiscard={vi.fn()}
      onCreatePr={vi.fn()}
      {...overrides}
    />,
  );
}

describe("PaneDiffModal manual PR action", () => {
  it("keeps Create PR available for unmanaged panes", () => {
    const onCreatePr = vi.fn();
    renderModal({ onCreatePr });

    fireEvent.click(screen.getByRole("button", { name: "Create PR" }));

    expect(onCreatePr).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByText(/Managed team panes open PRs after Reviewer approval/),
    ).toBeNull();
  });

  it("replaces Create PR with team workflow guidance for managed panes", () => {
    const onCreatePr = vi.fn();
    renderModal({ manualPrCreationDisabled: true, onCreatePr });

    expect(screen.queryByRole("button", { name: "Create PR" })).toBeNull();
    expect(
      screen.getByText(/Managed team panes open PRs after Reviewer approval/),
    ).toBeTruthy();
    expect(onCreatePr).not.toHaveBeenCalled();
  });
});
