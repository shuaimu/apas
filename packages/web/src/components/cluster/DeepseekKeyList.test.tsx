import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MachineInfo, MachineWithProjects } from "@/lib/store";
import { DeepseekKeyList } from "./DeepseekKeyList";

afterEach(cleanup);

function machine(machineId: string, hostname: string, configured: boolean | undefined): MachineWithProjects {
  return {
    machine: {
      machineId,
      hostname,
      os: "linux",
      arch: "x64",
      // A stray key, as an older store would have held: never shown.
      deepseekBackend: configured === undefined ? undefined : {
        apiBaseUrl: "https://api.deepseek.com/anthropic",
        apiKeyConfigured: configured,
        apiKey: "sk-stray-secret",
      } as NonNullable<MachineInfo["deepseekBackend"]>,
    },
    projects: [],
  };
}

function renderList(connected = true, sends = true) {
  const setMachineDeepseekConfig = vi.fn().mockReturnValue(sends);
  const requestConfirm = vi.fn();
  render(
    <DeepseekKeyList
      machines={[machine("m1", "alpha", true), machine("m2", "beta", false), machine("m3", "gamma", undefined)]}
      connected={connected}
      setMachineDeepseekConfig={setMachineDeepseekConfig}
      requestConfirm={requestConfirm}
    />,
  );
  return { setMachineDeepseekConfig, requestConfirm };
}

describe("DeepseekKeyList", () => {
  it("says whether each machine has a key and offers Replace or Set", () => {
    renderList();
    expect(screen.getByRole("heading", { name: "DeepSeek keys" })).toBeTruthy();
    expect(screen.getAllByText("API key configured")).toHaveLength(1);
    expect(screen.getAllByText("API key not configured")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Replace DeepSeek API key for alpha" }).textContent).toBe("Replace key");
    expect(screen.getByRole("button", { name: "Set DeepSeek API key for beta" }).textContent).toBe("Set key");
    expect(screen.getByRole("button", { name: "Clear DeepSeek API key for alpha" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Clear DeepSeek API key for beta" })).toBeNull();
    // The URL is printed once, not per machine.
    expect(screen.getAllByText("https://api.deepseek.com/anthropic")).toHaveLength(1);
    expect(document.body.textContent).not.toContain("sk-stray-secret");
  });

  it("never prefills the key and sends a typed one once", () => {
    const { setMachineDeepseekConfig } = renderList();
    fireEvent.click(screen.getByRole("button", { name: "Replace DeepSeek API key for alpha" }));
    const input = screen.getByLabelText("DeepSeek API key for alpha") as HTMLInputElement;
    expect(input.type).toBe("password");
    // A password manager should not offer to save it as a sign-in.
    expect(input.autocomplete).toBe("new-password");
    expect(input.value).toBe("");
    expect(screen.queryByDisplayValue("sk-stray-secret")).toBeNull();
    const save = screen.getByRole("button", { name: "Save DeepSeek API key for alpha" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "   " } });
    expect(save.disabled).toBe(true);
    fireEvent.change(input, { target: { value: " new-key " } });
    fireEvent.click(save);
    expect(setMachineDeepseekConfig).toHaveBeenCalledWith("m1", "new-key", false);
    expect(screen.queryByLabelText("DeepSeek API key for alpha")).toBeNull();
    expect(screen.getByRole("status").textContent).toBe("Key sent to alpha.");
  });

  it("focuses the field when the editor opens and the opener when it closes", () => {
    renderList();
    fireEvent.click(screen.getByRole("button", { name: "Set DeepSeek API key for beta" }));
    const input = screen.getByLabelText("DeepSeek API key for beta");
    expect(document.activeElement).toBe(input);
    fireEvent.keyDown(input, { key: "Escape" });
    expect(screen.queryByLabelText("DeepSeek API key for beta")).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Set DeepSeek API key for beta" }));
  });

  it("keeps the editor and says nothing was sent when the socket is not open", () => {
    const { setMachineDeepseekConfig } = renderList(true, false);
    fireEvent.click(screen.getByRole("button", { name: "Set DeepSeek API key for beta" }));
    fireEvent.change(screen.getByLabelText("DeepSeek API key for beta"), { target: { value: "typed" } });
    fireEvent.click(screen.getByRole("button", { name: "Save DeepSeek API key for beta" }));
    expect(setMachineDeepseekConfig).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("alert").textContent).toBe("Not connected to the server. Nothing was sent.");
    expect(screen.queryByText(/Key sent/)).toBeNull();
    expect(screen.getByLabelText("DeepSeek API key for beta")).toBeTruthy();
  });

  it("discards the draft on Cancel", () => {
    const { setMachineDeepseekConfig } = renderList();
    fireEvent.click(screen.getByRole("button", { name: "Set DeepSeek API key for beta" }));
    fireEvent.change(screen.getByLabelText("DeepSeek API key for beta"), { target: { value: "typed" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByLabelText("DeepSeek API key for beta")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Set DeepSeek API key for beta" }));
    expect((screen.getByLabelText("DeepSeek API key for beta") as HTMLInputElement).value).toBe("");
    expect(setMachineDeepseekConfig).not.toHaveBeenCalled();
  });

  it("asks before clearing a key", () => {
    const { setMachineDeepseekConfig, requestConfirm } = renderList();
    fireEvent.click(screen.getByRole("button", { name: "Clear DeepSeek API key for alpha" }));
    expect(setMachineDeepseekConfig).not.toHaveBeenCalled();
    const request = requestConfirm.mock.calls[0][0];
    expect(request).toMatchObject({ title: "Clear the DeepSeek API key on alpha?", confirmLabel: "Clear key", tone: "danger" });
    request.onConfirm();
    expect(setMachineDeepseekConfig).toHaveBeenCalledWith("m1", undefined, true);
  });

  it("disables every key control while not connected", () => {
    renderList(false);
    for (const name of [
      "Replace DeepSeek API key for alpha",
      "Clear DeepSeek API key for alpha",
      "Set DeepSeek API key for beta",
    ]) {
      const button = screen.getByRole("button", { name }) as HTMLButtonElement;
      expect(button.disabled).toBe(true);
      expect(button.title).toBe("Not connected to the server");
    }
    expect(within(document.body).queryByRole("textbox")).toBeNull();
  });
});
