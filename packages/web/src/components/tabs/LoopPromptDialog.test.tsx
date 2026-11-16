import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage, type SavedAutopilotPrompt } from "@apas/protocol";
import { handleServerMessage, paneKey, useStore, type PaneConfig } from "@/lib/store";
import {
  autopilotTemplateById,
  AUTOPILOT_OWN_TEXT,
  AUTOPILOT_OWN_TEXT_KEPT_TEXT,
  DEFAULT_AUTOPILOT_TEMPLATE,
} from "@/lib/autopilotTemplates";
import { PROJECT_AUTOPILOT_PROMPTS_CAPABILITY } from "@/lib/autopilotPrompts";
import { AUTOPILOT_PROMPT_MAX_BYTES } from "@/lib/autopilot";
import { DELIVERY_GRACE_MS } from "@/lib/terminalDelivery";
import {
  loopPromptMessage,
  loopPromptTemplateNote,
  LOOP_PROMPT_OWN_TEXT_NOTE,
  LOOP_PROMPT_SHOWN_INSTRUCTION,
} from "@/lib/loopPrompt";
import { LoopPromptDialog } from "./LoopPromptDialog";

/**
 * The loop prompt dialog, driven through the real store: what it says before
 * anything is sent, the prompt it offers, and the one message it sends
 * through the terminal conversation input, with what became of it.
 */

const SID = "20000000-0000-4000-8000-000000000001";
const PANE = 4;
const TODO = autopilotTemplateById("todo")!;
const NIGHTLY: SavedAutopilotPrompt = {
  name: "Nightly review",
  prompt: "Review the open pull requests and fix what the checks report.",
  updated_at: "2026-11-14T09:00:00Z",
};

const initialStore = useStore.getInitialState();
let ws: { readyState: number; send: Mock };

const claudeTab: PaneConfig = {
  pane_id: PANE,
  provider: "claude",
  mode: "interactive",
  kind: "terminal",
  session_id: `${SID}-pane-${PANE}`,
  is_paused: false,
  label: "Claude 1",
};

function seed({ savedPrompts = false }: { savedPrompts?: boolean } = {}) {
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  useStore.setState({
    ws: ws as unknown as WebSocket,
    connected: true,
    isAttached: true,
    sessionId: SID,
    paneConfigs: [claudeTab],
    paneMessages: { [paneKey(PANE)]: [] },
    pendingTerminalDeliveries: [],
    negotiatedCapabilities: new Set(savedPrompts ? [PROJECT_AUTOPILOT_PROMPTS_CAPABILITY] : []),
    projectAutopilotPrompts: savedPrompts
      ? { [SID]: { prompts: [NIGHTLY], default: NIGHTLY.name, unreadable: false, received: true } }
      : {},
  });
}

function sentInputs(): Record<string, unknown>[] {
  return ws.send.mock.calls.map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>);
}

function open(props: Partial<Parameters<typeof LoopPromptDialog>[0]> = {}) {
  const onClose = vi.fn();
  render(
    <LoopPromptDialog sessionId={SID} paneId={PANE} tabLabel="Claude 1" hostname="zoo-005" onClose={onClose} {...props} />,
  );
  const dialog = screen.getByRole("dialog", { name: "Loop prompt for Claude 1" });
  return {
    dialog,
    onClose,
    template: () => within(dialog).getByLabelText("Template") as HTMLSelectElement,
    prompt: () => within(dialog).getByLabelText("Prompt") as HTMLTextAreaElement,
    note: () => within(dialog).getByTestId("loop-prompt-template-note").textContent ?? "",
    send: () => within(dialog).queryByRole("button", { name: "Send loop prompt" }) as HTMLButtonElement | null,
    outcome: () => within(dialog).queryByTestId("loop-prompt-outcome"),
    choose(value: string) {
      fireEvent.change(within(dialog).getByLabelText("Template"), { target: { value } });
    },
    type(value: string) {
      fireEvent.change(within(dialog).getByLabelText("Prompt"), { target: { value } });
    },
  };
}

beforeEach(() => {
  localStorage.clear();
  seed();
});

afterEach(() => {
  vi.useRealTimers();
  act(() => {
    useStore.setState(initialStore, true);
  });
  localStorage.clear();
});

describe("before anything is sent", () => {
  it("says first that this is a shortcut APAS does not watch, and how to stop it", () => {
    const view = open();
    const warning = within(view.dialog).getByTestId("loop-prompt-warning");
    expect(warning.textContent).toContain("This is a shortcut, not autopilot. It sends one prompt that asks the agent to keep working in a loop by itself.");
    expect(warning.textContent).toContain("APAS does not run or watch that loop. It cannot count rounds, enforce a limit, pause, resume or stop it, or tell whether the agent is still looping.");
    expect(warning.textContent).toContain("The agent keeps this tab's permissions on zoo-005");
    expect(warning.textContent).toContain("Until it is stopped, it spends the agent's login on zoo-005, whoever sent the prompt. APAS sets no spending limit.");
    expect(warning.textContent).toContain("To stop it, interrupt it in the terminal (Esc).");
    // Read before the prompt and the button.
    expect(warning.compareDocumentPosition(view.template()) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    expect(warning.compareDocumentPosition(view.send()!) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    expect(ws.send).not.toHaveBeenCalled();
  });

  it("shows the loop instruction it puts before the prompt, read-only", () => {
    const view = open();
    expect(within(view.dialog).getByText("APAS puts this before your prompt:")).toBeTruthy();
    const instruction = within(view.dialog).getByTestId("loop-prompt-instruction");
    expect(instruction.textContent).toBe(LOOP_PROMPT_SHOWN_INSTRUCTION);
    expect(instruction.tagName).not.toBe("TEXTAREA");
    expect(instruction.querySelector("input, textarea")).toBeNull();
  });

  it("offers no limits, timing or tool-call option", () => {
    const view = open();
    expect(within(view.dialog).queryByRole("checkbox")).toBeNull();
    expect(within(view.dialog).queryByRole("radio")).toBeNull();
    expect(within(view.dialog).queryByText(/limit/i, { selector: "legend, label" })).toBeNull();
  });
});

describe("the prompt", () => {
  it("opens on the default template, and offers the other templates in rounds", () => {
    const view = open();
    expect(view.template().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.id);
    expect(view.prompt().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
    expect(view.note()).toBe(loopPromptTemplateNote(DEFAULT_AUTOPILOT_TEMPLATE));

    view.choose(TODO.id);
    expect(view.prompt().value).toBe(TODO.prompt);
    expect(view.note()).toBe(loopPromptTemplateNote(TODO));
    expect(view.note()).not.toMatch(/iteration/i);

    // A template with no ending of its own says its rounds follow at once.
    view.choose("tests_green");
    expect(view.note()).toContain("It has no end of its own: each round starts again at once, also when the last one found nothing to fix, until you interrupt it.");
  });

  it("opens on the project's default saved prompt, and lists the project's prompts", () => {
    seed({ savedPrompts: true });
    const view = open();
    expect(view.prompt().value).toBe(NIGHTLY.prompt);
    expect(Array.from(view.template().options).map((option) => option.textContent)).toContain("Nightly review (default)");
    expect(view.note()).toBe(`Saved to this project. ${LOOP_PROMPT_OWN_TEXT_NOTE}`);
    // Saving and deleting are the autopilot dialog's.
    expect(within(view.dialog).queryByRole("button", { name: /Save to project/ })).toBeNull();
  });

  it("keeps the person's own text when a template replaces it", () => {
    const view = open();
    view.type("Fix one lint warning.");
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    expect(view.note()).toBe(LOOP_PROMPT_OWN_TEXT_NOTE);
    view.choose(TODO.id);
    expect(view.note()).toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe("Fix one lint warning.");
  });

  it("refuses an empty prompt and one too long with the instruction, sending nothing", () => {
    const view = open();
    view.type("   ");
    fireEvent.click(view.send()!);
    expect(within(view.dialog).getByRole("alert").textContent).toBe("The loop prompt needs a prompt.");
    view.type("x".repeat(AUTOPILOT_PROMPT_MAX_BYTES));
    fireEvent.click(view.send()!);
    expect(within(view.dialog).getByRole("alert").textContent).toMatch(/longer than 16000 bytes/);
    expect(ws.send).not.toHaveBeenCalled();
  });
});

describe("sending", () => {
  it("sends one terminal conversation message, the instruction and then the prompt, once per click", () => {
    const view = open();
    view.choose(TODO.id);
    const button = view.send()!;
    // Two clicks before React renders again: the button is still there for
    // the second one.
    act(() => {
      button.click();
      button.click();
    });
    fireEvent.click(button);
    const frames = sentInputs();
    expect(frames).toHaveLength(1);
    expect(validateClientMessage(frames[0]).errors).toEqual([]);
    expect(frames[0]).toMatchObject({
      type: "terminal_conversation_input",
      session_id: SID,
      pane_id: PANE,
      text: loopPromptMessage(TODO.prompt),
    });
    // Never offered again; the prompt is shown as it was sent.
    expect(view.send()).toBeNull();
    expect(within(view.dialog).getByRole("button", { name: "Close" })).toBeTruthy();
    expect(view.prompt().readOnly).toBe(true);
    expect(view.outcome()?.getAttribute("data-outcome")).toBe("waiting");
    expect(view.outcome()?.textContent).toBe("Sent to terminal. Waiting for the agent to record it.");
  });

  it("says Sent once the transcript records it", () => {
    const view = open();
    fireEvent.click(view.send()!);
    const [frame] = sentInputs();
    act(() => handleServerMessage({
      type: "terminal_conversation_recorded",
      session_id: SID,
      pane_id: PANE,
      client_msg_id: frame.client_msg_id,
    }, useStore.setState, useStore.getState));
    expect(view.outcome()?.getAttribute("data-outcome")).toBe("sent");
    expect(view.outcome()?.textContent).toMatch(/^Sent\./);
  });

  it("says Not confirmed when nothing records it, and never sends it again", () => {
    vi.useFakeTimers();
    const view = open();
    fireEvent.click(view.send()!);
    act(() => {
      vi.advanceTimersByTime(DELIVERY_GRACE_MS + 1_000);
    });
    expect(view.outcome()?.getAttribute("data-outcome")).toBe("not_confirmed");
    expect(view.outcome()?.textContent).toMatch(/^Not confirmed\..*It was not sent again\./);
    // Kept for the existing recovery of unconfirmed sends.
    expect(useStore.getState().pendingTerminalDeliveries).toHaveLength(1);
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(sentInputs()).toHaveLength(1);
    expect(view.send()).toBeNull();
  });

  it("shows the server's reason for a refused send", () => {
    const view = open();
    fireEvent.click(view.send()!);
    const [frame] = sentInputs();
    act(() => handleServerMessage({
      type: "mutation_ack",
      mutation: "input",
      session_id: SID,
      pane_id: PANE,
      request_id: frame.client_msg_id,
      accepted: false,
      error: "zoo-005 is not running this project. Open the project again to start it there.",
    }, useStore.setState, useStore.getState));
    expect(view.outcome()?.getAttribute("data-outcome")).toBe("refused");
    expect(view.outcome()?.textContent).toBe("Not delivered: zoo-005 is not running this project. Open the project again to start it there.");
    expect(sentInputs()).toHaveLength(1);
  });

  it("keeps the dialog usable when the send is refused before leaving the page", () => {
    useStore.setState({ connected: false, ws: { readyState: WebSocket.CLOSED, send: ws.send } as unknown as WebSocket });
    const view = open();
    fireEvent.click(view.send()!);
    expect(within(view.dialog).getByRole("alert").textContent).toBe("Not connected to server");
    expect(ws.send).not.toHaveBeenCalled();
    // Nothing was sent, so it may be tried again.
    expect(view.send()).not.toBeNull();
  });

  it("sends nothing while the tab cannot take a message", () => {
    const view = open({ disabledReason: "This project is not running. Open it first." });
    expect(view.send()!.disabled).toBe(true);
    expect(within(view.dialog).getByRole("note").textContent).toBe("This project is not running. Open it first.");
    fireEvent.click(view.send()!);
    expect(ws.send).not.toHaveBeenCalled();
  });

  it("changes nothing about autopilot on the tab", () => {
    const before = useStore.getState();
    const view = open();
    fireEvent.click(view.send()!);
    const after = useStore.getState();
    expect(after.paneAutopilot).toBe(before.paneAutopilot);
    expect(after.autopilotRequests).toBe(before.autopilotRequests);
    expect(sentInputs().map((frame) => frame.type)).toEqual(["terminal_conversation_input"]);
  });

  it("closes with Cancel without sending", () => {
    const view = open();
    fireEvent.click(within(view.dialog).getByRole("button", { name: "Cancel" }));
    expect(view.onClose).toHaveBeenCalledTimes(1);
    expect(ws.send).not.toHaveBeenCalled();
  });
});
