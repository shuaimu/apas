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
  // A pane an older `.apas` marked managed used to get "Managed team panes
  // open PRs after Reviewer approval" here instead of the button. Team mode is
  // removed, so the modal takes no such flag and always offers the action.
  it("offers Create PR and no team workflow guidance", () => {
    const onCreatePr = vi.fn();
    renderModal({ onCreatePr });

    fireEvent.click(screen.getByRole("button", { name: "Create PR" }));

    expect(onCreatePr).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(/Managed team panes/)).toBeNull();
  });
});
