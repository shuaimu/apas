import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AddWorkerModal } from "./AddWorkerModal";
import { useStore } from "@/lib/store";

type StoreState = ReturnType<typeof useStore.getState>;

function renderAddWorkerModal(overrides: {
  addPane?: ReturnType<typeof vi.fn>;
  showToast?: ReturnType<typeof vi.fn>;
} = {}) {
  const addPane = overrides.addPane ?? vi.fn(() => ({ success: true }));
  const showToast = overrides.showToast ?? vi.fn();
  const onClose = vi.fn();

  act(() => {
    useStore.setState({
      addPane: addPane as StoreState["addPane"],
      showToast: showToast as StoreState["showToast"],
    });
  });

  render(<AddWorkerModal open={true} onClose={onClose} />);

  return { addPane, onClose, showToast };
}

function selectDeveloperTemplate() {
  fireEvent.click(screen.getByRole("button", { name: /Developer/ }));
}

function labelInput(): HTMLInputElement {
  return screen.getByLabelText("Label") as HTMLInputElement;
}

function isolatedCheckbox(): HTMLInputElement {
  return screen.getByLabelText(/Isolated git worktree/) as HTMLInputElement;
}

function roleInput(): HTMLInputElement {
  return screen.getByLabelText(/Role/) as HTMLInputElement;
}

function goalInput(): HTMLTextAreaElement {
  return screen.getByLabelText(/Goal/) as HTMLTextAreaElement;
}

function backstoryInput(): HTMLTextAreaElement {
  return screen.getByLabelText(/Backstory/) as HTMLTextAreaElement;
}

function planReviewSelect(): HTMLSelectElement {
  return screen.getByLabelText(/Plan review/) as HTMLSelectElement;
}

describe("AddWorkerModal", () => {
  afterEach(() => {
    vi.clearAllMocks();
    act(() => {
      useStore.setState({
        addPane: (() => ({ success: true })) as StoreState["addPane"],
        showToast: (() => {}) as StoreState["showToast"],
      });
    });
  });

  it("fills role metadata, default label, and isolated worktree from the Developer template", () => {
    renderAddWorkerModal();

    selectDeveloperTemplate();

    expect(labelInput().value).toBe("Developer");
    expect(isolatedCheckbox().checked).toBe(true);
    expect(roleInput().value).toBe("developer");
    expect(goalInput().value).toContain("Implement the leaf task assigned to you");
    expect(backstoryInput().value).toContain("hands-on implementer");
    expect(planReviewSelect().value).toBe("never");
  });

  it("submits template-backed workers as managed interactive panes with selected provider and metadata", () => {
    const addPane = vi.fn(() => ({ success: true }));
    renderAddWorkerModal({ addPane });

    selectDeveloperTemplate();
    fireEvent.change(screen.getByLabelText("Provider"), {
      target: { value: "codex" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add worker" }));

    expect(addPane).toHaveBeenCalledWith(
      "codex",
      "interactive",
      "Developer",
      undefined,
      undefined,
      true,
      expect.objectContaining({
        role: "developer",
        goal: expect.stringContaining("Implement the leaf task"),
        backstory: expect.stringContaining("hands-on implementer"),
        planReviewMode: "never",
      }),
      true,
    );
  });

  it("clears template-derived fields and worktree settings with No template", () => {
    renderAddWorkerModal();

    selectDeveloperTemplate();
    expect(roleInput().value).toBe("developer");
    expect(isolatedCheckbox().checked).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: /No template/ }));

    expect(labelInput().value).toBe("");
    expect(roleInput().value).toBe("");
    expect(goalInput().value).toBe("");
    expect(backstoryInput().value).toBe("");
    expect(isolatedCheckbox().checked).toBe(false);
    expect(planReviewSelect().value).toBe("never");
  });

  it("keeps the modal open and shows the returned error when addPane fails", () => {
    const addPane = vi.fn(() => ({
      success: false,
      error: "No agent capacity",
    }));
    const { onClose } = renderAddWorkerModal({ addPane });

    selectDeveloperTemplate();
    fireEvent.click(screen.getByRole("button", { name: "Add worker" }));

    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByText("No agent capacity")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add worker" })).toBeTruthy();
  });

  it("resets fields and closes the modal after a successful submit", () => {
    const addPane = vi.fn(() => ({ success: true }));
    const showToast = vi.fn();
    const { onClose } = renderAddWorkerModal({ addPane, showToast });

    selectDeveloperTemplate();
    fireEvent.click(screen.getByRole("button", { name: "Add worker" }));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(showToast).toHaveBeenCalledWith(
      expect.stringContaining("Developer (isolated)"),
      "success",
    );
    expect(labelInput().value).toBe("");
    expect(roleInput().value).toBe("");
    expect(isolatedCheckbox().checked).toBe(false);
  });
});
