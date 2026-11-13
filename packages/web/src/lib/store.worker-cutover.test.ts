import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "./store";

describe("worker cutover preserves bot evidence without restoring execution", () => {
  const send = vi.fn();
  beforeEach(() => {
    send.mockReset();
    useStore.setState({
      ws: { readyState: WebSocket.OPEN, send, close: vi.fn() } as unknown as WebSocket,
      sessionId: "project-session", isAttached: true, showToast: vi.fn(),
      paneConfigs: [{ pane_id: 8, provider: "claude", mode: "deadloop", session_id: "project-session", is_paused: true, prompt: "Original retained instructions" }],
      projectPolicies: { "project-session": { teamAvailable: true, allowedLaunchProfiles: ["agent:claude:official:default"], version: 1, projectSuspended: false, noncompliantPaneIds: [] } },
    });
  });
  it("refuses new deadloop admission and old bot resume, reboot and input without a wire command", () => {
    expect(useStore.getState().addPane("claude", "deadloop").success).toBe(false);
    useStore.getState().resumePane(8);
    useStore.getState().rebootPane(8);
    expect(useStore.getState().sendMessageToPane("continue", 8).success).toBe(false);
    expect(send).not.toHaveBeenCalled();
    expect(useStore.getState().paneConfigs[0].prompt).toBe("Original retained instructions");
    expect(useStore.getState().paneConfigs[0].mode).toBe("deadloop");
  });
});
