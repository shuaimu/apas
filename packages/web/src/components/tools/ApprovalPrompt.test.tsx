import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { ApprovalPrompt } from "./ApprovalPrompt";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();

function renderApprovalPrompt() {
  const approve = vi.fn();
  const reject = vi.fn();

  act(() => {
    useStore.setState({
      approve: approve as StoreState["approve"],
      reject: reject as StoreState["reject"],
    });
  });

  render(
    <ApprovalPrompt
      toolCallId="toolu_permission_123"
      tool="Bash"
      description="Run `npm test` in the web package"
    />,
  );

  return { approve, reject };
}

describe("ApprovalPrompt", () => {
  afterEach(() => {
    vi.clearAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders the permission request details", () => {
    renderApprovalPrompt();

    expect(screen.getByText("Permission Required")).toBeTruthy();
    expect(screen.getByText("Bash")).toBeTruthy();
    expect(
      screen.getByText("Run `npm test` in the web package"),
    ).toBeTruthy();
  });

  it("approves the exact tool call id once", () => {
    const { approve, reject } = renderApprovalPrompt();

    fireEvent.click(screen.getByRole("button", { name: /Approve/ }));

    expect(approve).toHaveBeenCalledTimes(1);
    expect(approve).toHaveBeenCalledWith("toolu_permission_123");
    expect(reject).not.toHaveBeenCalled();
  });

  it("rejects the exact tool call id once", () => {
    const { approve, reject } = renderApprovalPrompt();

    fireEvent.click(screen.getByRole("button", { name: /Reject/ }));

    expect(reject).toHaveBeenCalledTimes(1);
    expect(reject).toHaveBeenCalledWith("toolu_permission_123");
    expect(approve).not.toHaveBeenCalled();
  });
});
