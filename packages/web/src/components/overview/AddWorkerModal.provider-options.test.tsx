import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AddWorkerModal } from "./AddWorkerModal";
import { useStore } from "@/lib/store";
import {
  CLAUDE_FABLE_MODEL,
  DEEPSEEK_DEFAULT_MODEL,
  PROVIDER_MODEL_OPTIONS,
} from "@/lib/providerOptions";

type StoreState = ReturnType<typeof useStore.getState>;

function renderAddWorkerModal(addPane = vi.fn(() => ({ success: true }))) {
  act(() => {
    useStore.setState({
      addPane: addPane as StoreState["addPane"],
      showToast: (() => {}) as StoreState["showToast"],
    });
  });

  render(<AddWorkerModal open={true} onClose={vi.fn()} />);

  return { addPane };
}

describe("AddWorkerModal provider options", () => {
  afterEach(() => {
    vi.clearAllMocks();
    act(() => {
      useStore.setState({
        addPane: (() => ({ success: true })) as StoreState["addPane"],
        showToast: (() => {}) as StoreState["showToast"],
      });
    });
  });

  it("renders provider choices from the shared provider/model option list", () => {
    renderAddWorkerModal();

    const select = screen.getByLabelText("Provider") as HTMLSelectElement;
    const labels = Array.from(select.options).map((option) => option.text);
    const values = Array.from(select.options).map((option) => option.value);

    expect(labels).toEqual(PROVIDER_MODEL_OPTIONS.map((option) => option.label));
    expect(values).toEqual(PROVIDER_MODEL_OPTIONS.map((option) => option.value));
    expect(labels).toContain("Claude / Fable");
    expect(values).toContain("claude/fable");
  });

  it("preserves model-backed provider selection when submitting a managed worker", () => {
    const addPane = vi.fn(() => ({ success: true }));
    renderAddWorkerModal(addPane);

    fireEvent.click(screen.getByRole("button", { name: /Developer/ }));
    fireEvent.change(screen.getByLabelText("Provider"), {
      target: { value: "claude/deepseek" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add worker" }));

    expect(addPane).toHaveBeenCalledWith(
      "claude",
      "interactive",
      "Developer",
      undefined,
      DEEPSEEK_DEFAULT_MODEL,
      true,
      expect.objectContaining({
        role: "developer",
        goal: expect.stringContaining("Implement the leaf task"),
        planReviewMode: "never",
      }),
      true,
    );
  });

  it("preserves the shared Claude Fable option when submitting a managed worker", () => {
    const addPane = vi.fn(() => ({ success: true }));
    renderAddWorkerModal(addPane);

    fireEvent.click(screen.getByRole("button", { name: /Developer/ }));
    fireEvent.change(screen.getByLabelText("Provider"), {
      target: { value: "claude/fable" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add worker" }));

    expect(addPane).toHaveBeenCalledWith(
      "claude",
      "interactive",
      "Developer",
      undefined,
      CLAUDE_FABLE_MODEL,
      true,
      expect.objectContaining({
        role: "developer",
        goal: expect.stringContaining("Implement the leaf task"),
        planReviewMode: "never",
      }),
      true,
    );
  });
});
