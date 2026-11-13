import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage, type AutopilotSpec, type PaneAutopilotState } from "@apas/protocol";
import { handleServerMessage, useStore } from "@/lib/store";
import {
  AUTOPILOT_STOP_NOW_TEXT,
  paneAutopilotKey,
  TAB_AUTOPILOT_CAPABILITY,
  type PaneAutopilotEntry,
} from "@/lib/autopilot";
import { noRuntimeAccessReason } from "@/lib/tabHosts";
import { AutopilotControl } from "./AutopilotControl";

/**
 * The pane toolbar's autopilot control, against the real store. The socket is
 * a stub, so every frame the control sends can be checked against the schema
 * the server parses.
 */

const SID = "20000000-0000-4000-8000-000000000001";
const PANE = 4;
const AUTOPILOT = "60000000-0000-4000-8000-000000000001";
const initialStore = useStore.getInitialState();

const SPEC: AutopilotSpec = {
  prompt: "Check the build and fix what fails.",
  repeat: { kind: "every", seconds: 900 },
  iteration_limit: { kind: "at_most", value: 20 },
  time_limit: { kind: "at_most", value: 8 * 3600 },
  end_without_tool_call: false,
};

function current(kind: string, detail?: string, spec: AutopilotSpec = SPEC): PaneAutopilotState {
  return {
    autopilot: {
      autopilot_id: AUTOPILOT,
      spec,
      started_at: "2026-11-13T14:00:00Z",
      iterations: 7,
      deadline: spec.time_limit.kind === "at_most" ? "2026-11-13T22:00:00Z" : undefined,
      next_iteration_at: detail === "next_iteration" ? "2026-11-13T14:30:00Z" : undefined,
      condition: { kind, detail },
    },
  };
}

function ended(reason: string): PaneAutopilotState {
  return {
    last_ended: {
      autopilot_id: AUTOPILOT,
      spec: SPEC,
      started_at: "2026-11-13T14:00:00Z",
      iterations: 12,
      reason,
      ended_at: "2026-11-13T22:00:00Z",
      turn_since: false,
    },
  };
}

let send: Mock;

function seed(entry: PaneAutopilotEntry | undefined, options: { connected?: boolean } = {}) {
  send = vi.fn();
  act(() => {
    useStore.setState({
      connected: options.connected ?? true,
      sessionId: SID,
      negotiatedCapabilities: new Set([TAB_AUTOPILOT_CAPABILITY]),
      ws: { readyState: WebSocket.OPEN, send } as unknown as WebSocket,
      paneAutopilot: entry ? { [paneAutopilotKey(SID, PANE)]: entry } : {},
      autopilotRequests: {},
    });
  });
}

function renderControl(props: Partial<Parameters<typeof AutopilotControl>[0]> = {}) {
  return render(
    <AutopilotControl
      sessionId={SID}
      paneId={PANE}
      tabLabel="OMP 1"
      hostname="zoo-006"
      sharedCheckout
      {...props}
    />,
  );
}

function controlButton(): HTMLButtonElement {
  return within(screen.getByTestId("autopilot-control")).getByRole("button", { name: "Autopilot" }) as HTMLButtonElement;
}

function sentActions(): Record<string, unknown>[] {
  return send.mock.calls.map(([raw]) => {
    const frame = JSON.parse(String(raw)) as Record<string, unknown>;
    const result = validateClientMessage(frame);
    expect(result.errors).toEqual([]);
    expect(frame).toMatchObject({ type: "control_pane_autopilot", session_id: SID, pane_id: PANE });
    return frame.action as Record<string, unknown>;
  });
}

function menu(): HTMLElement {
  return screen.getByRole("group", { name: "Autopilot" });
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  act(() => {
    useStore.setState(initialStore, true);
  });
  vi.clearAllMocks();
});

describe("a tab that is not on autopilot", () => {
  it("opens the start dialog without sending anything", () => {
    seed({ support: "supported" });
    renderControl();
    expect(screen.queryByTestId("autopilot-condition")).toBeNull();

    fireEvent.click(controlButton());

    expect(screen.getByRole("dialog", { name: "Start autopilot — OMP 1" })).toBeTruthy();
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("keeps the prompt field's height when the dialog has to scroll", () => {
    // The dialog is a scrolling column and the textarea is the one child
    // that can shrink: in a browser it collapsed to a sliver on a phone and
    // on a short window, hiding what was typed.
    seed({ support: "supported" });
    renderControl();
    fireEvent.click(controlButton());

    const dialog = screen.getByRole("dialog", { name: "Start autopilot — OMP 1" });
    expect(dialog.className.split(" ")).toEqual(expect.arrayContaining(["flex-col", "overflow-y-auto"]));
    const prompt = screen.getByLabelText("Prompt") as HTMLTextAreaElement;
    expect(prompt.rows).toBe(5);
    expect(prompt.className.split(" ")).toContain("shrink-0");
    expect(prompt.className.split(" ")).toContain("text-sm");
  });

  it("states what the mode is and is not before a start is sent", () => {
    seed({ support: "supported" });
    renderControl();
    fireEvent.click(controlButton());

    const statement = screen.getByRole("list", { name: "What autopilot is and is not" }).textContent ?? "";
    expect(statement).toContain("This tab keeps running on zoo-006");
    expect(statement).toContain("the tab's own permissions, including tools that run without asking");
    expect(statement).toContain("spends the OMP login configured there, whoever starts it");
    expect(statement).toContain("APAS adds no sandbox, no independent review and no spending limit.");
    expect(statement).toContain("Limits are checked between turns, so a turn in progress can outlast them.");
    expect(statement).toContain("not a security boundary");
    expect(statement).toContain("Everyone who can open this session can read the prompt.");
    expect(statement).toContain("Slash commands and skills in the prompt are not run.");
    expect(statement).toContain("may have to be answered in the terminal");
    expect(statement).toContain("This tab works in the project's shared checkout.");
  });

  it("proposes no limit, says what that means, and starts with both limits said explicitly", () => {
    seed({ support: "supported" });
    renderControl();
    fireEvent.click(controlButton());
    const dialog = screen.getByRole("dialog");

    // No prompt of APAS's own.
    expect((within(dialog).getByLabelText("Prompt") as HTMLTextAreaElement).value).toBe("");
    const noLimits = within(dialog).getAllByRole("radio", { name: "No limit" }) as HTMLInputElement[];
    expect(noLimits).toHaveLength(2);
    expect(noLimits.every((radio) => radio.checked)).toBe(true);
    expect(within(dialog).getByTestId("autopilot-limits-statement").textContent)
      .toBe("No limit is set. Autopilot then runs until someone stops it.");
    // Proposed on for "as soon as each turn ends".
    expect((within(dialog).getByRole("radio", { name: "As soon as each turn ends" }) as HTMLInputElement).checked).toBe(true);
    expect((within(dialog).getByRole("checkbox", { name: /End when an iteration makes no tool call/ }) as HTMLInputElement).checked).toBe(true);

    fireEvent.click(within(dialog).getByRole("button", { name: "Start autopilot" }));
    expect(within(dialog).getByRole("alert").textContent).toBe("Autopilot needs a prompt.");
    expect(send).not.toHaveBeenCalled();

    fireEvent.change(within(dialog).getByLabelText("Prompt"), { target: { value: "Fix the next failing test." } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Start autopilot" }));

    expect(sentActions()).toEqual([{
      kind: "start",
      spec: {
        prompt: "Fix the next failing test.",
        repeat: { kind: "after_each_turn" },
        iteration_limit: { kind: "none" },
        time_limit: { kind: "none" },
        end_without_tool_call: true,
      },
    }]);
    expect(screen.queryByRole("dialog")).toBeNull();
    // The start is on its way: a second one would only be refused. Sending
    // it shows no condition: the tab has reported none.
    expect(controlButton().disabled).toBe(true);
    expect(controlButton().title).toBe("A start is on its way to this tab.");
    expect(screen.getByRole("note").textContent).toBe("A start is on its way to this tab.");
    expect(screen.queryByTestId("autopilot-condition")).toBeNull();
  });

  it("proposes the no-tool-call ending only for an immediate repeat, until the person decides", () => {
    seed({ support: "supported" });
    renderControl();
    fireEvent.click(controlButton());
    const dialog = screen.getByRole("dialog");
    const toolCall = () => within(dialog).getByRole("checkbox", { name: /End when an iteration makes no tool call/ }) as HTMLInputElement;

    fireEvent.click(within(dialog).getByRole("radio", { name: "Every" }));
    expect(toolCall().checked).toBe(false);
    expect(within(dialog).getByTestId("autopilot-limits-statement").textContent)
      .toContain("A timed repeat has no end of its own: it runs until a limit is reached or a person stops it.");
    fireEvent.click(within(dialog).getByRole("radio", { name: "As soon as each turn ends" }));
    expect(toolCall().checked).toBe(true);

    fireEvent.click(toolCall());
    expect(toolCall().checked).toBe(false);
    fireEvent.click(within(dialog).getByRole("radio", { name: "Every" }));
    fireEvent.click(within(dialog).getByRole("radio", { name: "As soon as each turn ends" }));
    expect(toolCall().checked).toBe(false);
  });

  it("refuses a time limit shorter than the repeat interval before sending", () => {
    seed({ support: "supported" });
    renderControl();
    fireEvent.click(controlButton());
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Prompt"), { target: { value: "Check the build." } });
    fireEvent.click(within(dialog).getByRole("radio", { name: "Every" }));
    fireEvent.change(within(dialog).getByLabelText("Repeat interval"), { target: { value: "2" } });
    fireEvent.change(within(dialog).getByLabelText("Repeat interval unit"), { target: { value: "hours" } });
    fireEvent.click(within(dialog).getAllByRole("radio", { name: "At most" })[1]);
    fireEvent.change(within(dialog).getByLabelText("Time limit"), { target: { value: "1" } });
    fireEvent.change(within(dialog).getByLabelText("Time limit unit"), { target: { value: "hours" } });

    fireEvent.click(within(dialog).getByRole("button", { name: "Start autopilot" }));
    expect(within(dialog).getByRole("alert").textContent).toBe("The time limit is shorter than the repeat interval.");
    expect(send).not.toHaveBeenCalled();

    fireEvent.change(within(dialog).getByLabelText("Time limit"), { target: { value: "8" } });
    expect(within(dialog).queryByRole("alert")).toBeNull();
    expect(within(dialog).getByTestId("autopilot-limits-statement").textContent).toContain("Autopilot ends after 8 hours.");
    fireEvent.click(within(dialog).getByRole("button", { name: "Start autopilot" }));
    expect(sentActions()).toEqual([{
      kind: "start",
      spec: {
        prompt: "Check the build.",
        repeat: { kind: "every", seconds: 7200 },
        iteration_limit: { kind: "none" },
        time_limit: { kind: "at_most", value: 8 * 3600 },
        end_without_tool_call: false,
      },
    }]);
  });

  it("keeps the dialog open with the reason when the start cannot be sent", () => {
    seed({ support: "supported" });
    renderControl();
    fireEvent.click(controlButton());
    act(() => useStore.setState({ ws: null }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Prompt"), { target: { value: "go" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Start autopilot" }));
    expect(within(dialog).getByRole("alert").textContent).toBe("Not connected to the server.");
    expect(useStore.getState().autopilotRequests).toEqual({});
  });
});

describe("a tab with an ended autopilot", () => {
  it("shows why it ended and offers its values for a new start", () => {
    seed({ support: "supported", state: ended("time_limit") });
    renderControl();
    expect(screen.getByTestId("autopilot-condition").textContent).toContain("Ended: time limit reached");

    fireEvent.click(controlButton());
    const dialog = screen.getByRole("dialog");
    expect((within(dialog).getByLabelText("Prompt") as HTMLTextAreaElement).value).toBe(SPEC.prompt);
    expect((within(dialog).getByRole("radio", { name: "Every" }) as HTMLInputElement).checked).toBe(true);
    expect((within(dialog).getByLabelText("Repeat interval") as HTMLInputElement).value).toBe("15");
    expect((within(dialog).getByLabelText("Iteration limit") as HTMLInputElement).value).toBe("20");
    expect((within(dialog).getByLabelText("Time limit") as HTMLInputElement).value).toBe("8");
    expect((within(dialog).getByLabelText("Time limit unit") as HTMLSelectElement).value).toBe("hours");
    expect(within(dialog).getByTestId("autopilot-limits-statement").textContent)
      .toContain("Autopilot ends after 20 iterations or 8 hours, whichever comes first.");
    // Nothing is sent until the person confirms the start.
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Start autopilot" }));
    expect(sentActions()).toEqual([{ kind: "start", spec: SPEC }]);
  });

  it("keeps showing what the tab reported through the start and its receipt, until the tab reports", () => {
    seed({ support: "supported", state: ended("time_limit") });
    renderControl();
    const reported = screen.getByTestId("autopilot-condition").textContent;
    expect(reported).toMatch(/^Ended: time limit reached · /);

    fireEvent.click(controlButton());
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Start autopilot" }));
    const [requestId] = Object.keys(useStore.getState().autopilotRequests);

    // Sent: the condition is still the tab's last report, and another start
    // is not offered meanwhile.
    expect(screen.getByTestId("autopilot-condition").textContent).toBe(reported);
    expect(controlButton().disabled).toBe(true);
    expect(screen.getByRole("note").textContent).toBe("A start is on its way to this tab.");

    // Accepted means only that the tab took the request.
    act(() => handleServerMessage(
      { type: "pane_autopilot_receipt", session_id: SID, pane_id: PANE, request_id: requestId, outcome: "accepted" },
      useStore.setState,
      useStore.getState,
    ));
    expect(useStore.getState().autopilotRequests[requestId]?.status).toBe("accepted");
    expect(screen.getByTestId("autopilot-condition").textContent).toBe(reported);
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();

    // The tab reports: only now does the control show an autopilot.
    act(() => handleServerMessage(
      { type: "pane_autopilot_state", session_id: SID, pane_id: PANE, support: "supported", state: current("starting") },
      useStore.setState,
      useStore.getState,
    ));
    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Starting");
    expect(controlButton().disabled).toBe(false);
    expect(screen.queryByRole("note")).toBeNull();
    expect(send).toHaveBeenCalledTimes(1);
  });

  it("lets the person start again deliberately after a start that was not confirmed", () => {
    seed({ support: "supported" });
    renderControl();
    fireEvent.click(controlButton());
    fireEvent.change(within(screen.getByRole("dialog")).getByLabelText("Prompt"), { target: { value: "go" } });
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Start autopilot" }));
    const [requestId] = Object.keys(useStore.getState().autopilotRequests);
    expect(controlButton().disabled).toBe(true);

    act(() => handleServerMessage(
      { type: "pane_autopilot_receipt", session_id: SID, pane_id: PANE, request_id: requestId, outcome: "uncertain" },
      useStore.setState,
      useStore.getState,
    ));
    // Nothing was sent again, and nothing is shown as started.
    expect(send).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId("autopilot-condition")).toBeNull();
    // The tab's reported state shows whether it started; here it did not,
    // and starting is offered again.
    expect(controlButton().disabled).toBe(false);
    fireEvent.click(controlButton());
    expect(screen.getByRole("dialog", { name: "Start autopilot — OMP 1" })).toBeTruthy();
    expect(send).toHaveBeenCalledTimes(1);
  });
});

describe("a tab on autopilot", () => {
  it("shows the condition and offers Pause, Stop, Stop now and the details", () => {
    seed({ support: "supported", state: current("waiting", "next_iteration") });
    renderControl();
    expect(screen.getByTestId("autopilot-condition").textContent).toMatch(/^Waiting for the next iteration at /);

    fireEvent.click(controlButton());
    // Pause, Stop, Stop now, and Details.
    expect(within(menu()).getAllByRole("button")).toHaveLength(4);
    expect(within(menu()).getByRole("button", { name: /^Pause/ })).toBeTruthy();
    expect(within(menu()).getByRole("button", { name: /^Stop(?! now)/ })).toBeTruthy();
    expect(within(menu()).getByRole("button", { name: /^Stop now/ }).textContent).toContain(AUTOPILOT_STOP_NOW_TEXT);
    expect(within(menu()).queryByRole("button", { name: /^Resume/ })).toBeNull();
    expect(menu().textContent).toContain("7 of 20 iterations");
    expect(screen.queryByRole("dialog")).toBeNull();

    fireEvent.click(within(menu()).getByRole("button", { name: "Details" }));
    const details = screen.getByTestId("autopilot-details").textContent ?? "";
    expect(details).toContain("Check the build and fix what fails.");
    expect(details).toContain("Every 15 minutes");
    expect(details).toContain("20 iterations");
    expect(details).toContain("8 hours");
    expect(details).toContain("Started");
    expect(details).toContain("7 of 20 iterations");
    expect(details).toContain("Time limit ends");
  });

  it("says No limit in the details of an autopilot that has none", () => {
    const unlimited: AutopilotSpec = {
      ...SPEC,
      repeat: { kind: "after_each_turn" },
      iteration_limit: { kind: "none" },
      time_limit: { kind: "none" },
    };
    seed({ support: "supported", state: current("running", undefined, unlimited) });
    renderControl();
    fireEvent.click(controlButton());
    expect(menu().textContent).toContain("7 iterations");
    fireEvent.click(within(menu()).getByRole("button", { name: "Details" }));
    const details = screen.getByTestId("autopilot-details");
    expect(within(details).getAllByText("No limit")).toHaveLength(2);
    expect(details.textContent).toContain("As soon as each turn ends");
    expect(details.textContent).not.toContain("Time limit ends");
  });

  it("lets someone who may use the tab pause, stop and stop now", () => {
    seed({ support: "supported", state: current("running") });
    renderControl();

    fireEvent.click(controlButton());
    fireEvent.click(within(menu()).getByRole("button", { name: /^Pause/ }));
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    fireEvent.click(controlButton());
    fireEvent.click(within(menu()).getByRole("button", { name: /^Stop(?! now)/ }));
    fireEvent.click(controlButton());
    fireEvent.click(within(menu()).getByRole("button", { name: /^Stop now/ }));

    expect(sentActions()).toEqual([
      { kind: "pause", autopilot_id: AUTOPILOT },
      { kind: "stop", autopilot_id: AUTOPILOT, now: false },
      { kind: "stop", autopilot_id: AUTOPILOT, now: true },
    ]);
    // The displayed state is still what the tab reported.
    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Running an iteration");
  });

  it("offers Resume for a paused autopilot, naming the autopilot it resumes", () => {
    seed({ support: "supported", state: current("paused", "failures") });
    renderControl();
    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Paused after three failed iterations");

    fireEvent.click(controlButton());
    expect(within(menu()).queryByRole("button", { name: /^Pause/ })).toBeNull();
    expect(within(menu()).getByRole("button", { name: /^Stop now/ })).toBeTruthy();
    fireEvent.click(within(menu()).getByRole("button", { name: /^Resume/ }));
    expect(sentActions()).toEqual([{ kind: "resume", autopilot_id: AUTOPILOT }]);
  });
});

describe("who may use the tab", () => {
  it("lets a collaborator with runtime access start, as the hosting account can", () => {
    // Nothing about the viewer disables the control: no refusal is passed
    // for someone who may type into the tab, whoever hosts it.
    seed({ support: "supported" });
    renderControl({ tabRefusal: undefined });
    expect(controlButton().disabled).toBe(false);
    fireEvent.click(controlButton());
    fireEvent.change(screen.getByLabelText("Prompt"), { target: { value: "go" } });
    fireEvent.click(screen.getByRole("button", { name: "Start autopilot" }));
    expect(sentActions()).toHaveLength(1);
  });

  it("shows the state to a viewer without runtime access and disables every action with the reason", () => {
    const reason = noRuntimeAccessReason("zoo-006");
    seed({ support: "supported", state: current("paused", "interrupted") });
    renderControl({ tabRefusal: reason });

    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Paused: the turn was interrupted");
    fireEvent.click(controlButton());
    for (const name of [/^Resume/, /^Stop(?! now)/, /^Stop now/]) {
      const action = within(menu()).getByRole("button", { name }) as HTMLButtonElement;
      expect(action.disabled).toBe(true);
      expect(action.title).toBe(reason);
      fireEvent.click(action);
    }
    expect(within(menu()).getAllByRole("note").map((note) => note.textContent)).toContain(reason);
    expect(send).not.toHaveBeenCalled();

    // The prompt and the rest of the details stay readable.
    fireEvent.click(within(menu()).getByRole("button", { name: "Details" }));
    expect(screen.getByTestId("autopilot-details").textContent).toContain(SPEC.prompt);
  });

  it("disables starting, with the reason, for a viewer without runtime access", () => {
    const reason = noRuntimeAccessReason("zoo-006");
    seed({ support: "supported", state: ended("iteration_limit") });
    renderControl({ tabRefusal: reason });
    expect(controlButton().disabled).toBe(true);
    expect(screen.getByRole("note").textContent).toBe(reason);
    // The ended reason is still shown.
    expect(screen.getByTestId("autopilot-condition").textContent).toContain("Ended: iteration limit reached");
  });
});

describe("a state that is not reported", () => {
  it("says so and offers Stop and Stop now only", () => {
    for (const entry of [undefined, { support: "not_reported" as const }]) {
      seed(entry);
      const view = renderControl();
      expect(screen.getByTestId("autopilot-condition").textContent).toBe("Not reported");

      fireEvent.click(controlButton());
      expect(menu().textContent).toContain("This tab has not reported its autopilot state.");
      const offered = within(menu()).getAllByRole("button").map((button) => button.textContent ?? "");
      expect(offered).toHaveLength(2);
      expect(offered[0]).toMatch(/^Stop(?! now)/);
      expect(offered[1]).toMatch(/^Stop now/);
      expect(screen.queryByRole("dialog")).toBeNull();

      fireEvent.click(within(menu()).getByRole("button", { name: /^Stop now/ }));
      // No autopilot id is known, and none is needed.
      expect(sentActions()).toEqual([{ kind: "stop", now: true }]);
      view.unmount();
    }
  });

  it("says when the tab last reported", () => {
    seed({ support: "not_reported", reportedAt: "2026-11-13T14:16:05Z" });
    renderControl();
    fireEvent.click(controlButton());
    expect(menu().textContent).toMatch(/This tab has not reported its autopilot state\. Last reported at .+\./);
  });

  it("disables Stop while the tab's machine is not running the project", () => {
    seed(undefined);
    const reason = "zoo-005 is not running this project. Start it on zoo-005 first.";
    renderControl({ tabRefusal: reason, hostname: "zoo-005" });
    fireEvent.click(controlButton());
    const stop = within(menu()).getByRole("button", { name: /^Stop(?! now)/ }) as HTMLButtonElement;
    expect(stop.disabled).toBe(true);
    expect(stop.title).toBe(reason);
  });
});

describe("an autopilot paused because the agent process exited", () => {
  it("offers no Resume, Stop or Stop now and says what to do", () => {
    seed({ support: "supported", state: { ...current("paused", "agent_exited"), reported_by_cli: true } });
    renderControl();
    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Paused: the agent process exited");

    fireEvent.click(controlButton());
    expect(menu().textContent)
      .toContain("This tab's agent is not running. Reboot it first; its autopilot comes back paused.");
    expect(within(menu()).queryByRole("button", { name: /^Resume/ })).toBeNull();
    expect(within(menu()).queryByRole("button", { name: /^Stop/ })).toBeNull();
    // Its details can still be read.
    expect(within(menu()).getByRole("button", { name: "Details" })).toBeTruthy();
  });
});

describe("where autopilot cannot be used", () => {
  it.each([
    ["needs_agent_reboot", "Reboot this tab's agent to use autopilot. Rebooting ends its turn in progress."],
    ["instance_update_required", "zoo-006 runs an older APAS that cannot run autopilot. Reboot zoo-006 to update it."],
    ["unsupported_omp", "This tab's OMP version cannot run autopilot."],
    ["unavailable_on_platform", "Autopilot is not available for tabs on this machine."],
  ] as const)("shows the control disabled with the reason for %s", (support, reason) => {
    seed({ support });
    renderControl();
    // Disabled, not hidden.
    expect(controlButton().disabled).toBe(true);
    expect(controlButton().title).toBe(reason);
    expect(screen.getByRole("note").textContent).toBe(reason);
    fireEvent.click(controlButton());
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("disables the control while the page is offline and queues nothing", () => {
    seed({ support: "supported", state: current("running") }, { connected: false });
    renderControl();
    fireEvent.click(controlButton());
    const stop = within(menu()).getByRole("button", { name: /^Stop(?! now)/ }) as HTMLButtonElement;
    expect(stop.disabled).toBe(true);
    expect(stop.title).toBe("Not connected to the server.");
    expect(useStore.getState().autopilotRequests).toEqual({});
  });
});
