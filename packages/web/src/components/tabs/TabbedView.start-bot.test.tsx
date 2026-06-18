import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneKey, useStore, type PaneConfig } from "@/lib/store";
import { CLASSIC_TODO_BOT_LOOP_PROMPT, TabbedView } from "./TabbedView";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();
const SESSION_ID = "session-start-bot";
const CLI_CLIENT_ID = "cli-start-bot";
const PANE_ID = 42;

function activeTabKey(): string {
  return `apas_layout_${CLI_CLIENT_ID}_active_tab`;
}

function pane(overrides: Partial<PaneConfig> = {}): PaneConfig {
  return {
    pane_id: PANE_ID,
    provider: overrides.provider ?? "claude",
    mode: "interactive",
    session_id: `${SESSION_ID}-pane-${PANE_ID}`,
    is_paused: false,
    label: overrides.label ?? "Worker",
    prompt: overrides.prompt,
    min_iteration_interval_minutes: overrides.min_iteration_interval_minutes,
    effort: overrides.effort,
    managed: overrides.managed,
    role: overrides.role,
    goal: overrides.goal,
    backstory: overrides.backstory,
  };
}

function seedTabbedView({
  paneConfig = pane(),
  startBot = vi.fn(),
}: {
  paneConfig?: PaneConfig;
  startBot?: StoreState["startBot"];
} = {}) {
  localStorage.setItem(activeTabKey(), String(paneConfig.pane_id));

  act(() => {
    useStore.setState({
      connected: true,
      isAttached: true,
      isDualPane: true,
      sessionId: SESSION_ID,
      cliClientId: CLI_CLIENT_ID,
      messages: [],
      paneConfigs: [paneConfig],
      paneMessages: { [paneKey(paneConfig.pane_id)]: [] },
      paneHasMore: {},
      paneStatuses: {},
      paneModes: {},
      pausedPanes: [],
      loadingMorePane: null,
      hasMoreMessages: false,
      isLoadingMore: false,
      teamRecords: [],
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      startBot,
      updatePaneEffort: vi.fn(),
    });
  });

  return { startBot };
}

async function openStartBotModal() {
  render(<TabbedView />);
  const button = await screen.findByTitle("Start autonomous bot execution in this tab");
  fireEvent.click(button);
  return {
    interval: screen.getByRole("spinbutton") as HTMLInputElement,
    prompt: screen.getByPlaceholderText("Enter bot loop prompt...") as HTMLTextAreaElement,
  };
}

function startBotBackdrop(): HTMLElement {
  const heading = screen.getByText("Start Bot on Worker");
  const header = heading.closest("div");
  const panel = header?.parentElement;
  const backdrop = panel?.parentElement;
  expect(backdrop).toBeTruthy();
  return backdrop as HTMLElement;
}

describe("TabbedView Start Bot prompt modal", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("hydrates saved prompt and interval, then starts with edited sanitized values", async () => {
    const { startBot } = seedTabbedView({
      paneConfig: pane({
        prompt: "Saved worker loop",
        min_iteration_interval_minutes: 9,
        effort: "high",
      }),
    });

    const { interval, prompt } = await openStartBotModal();

    expect(screen.getByText("Start Bot on Worker")).toBeTruthy();
    expect(interval.value).toBe("9");
    expect(prompt.value).toBe("Saved worker loop");

    fireEvent.change(prompt, { target: { value: "Run the next delegated task" } });
    fireEvent.change(interval, { target: { value: "7.8" } });
    fireEvent.click(screen.getByRole("button", { name: "Start Bot" }));

    expect(startBot).toHaveBeenCalledWith(
      PANE_ID,
      "Run the next delegated task",
      7,
      "high",
    );
    await waitFor(() => {
      expect(screen.queryByText("Start Bot on Worker")).toBeNull();
    });
  });

  it("falls back to the default prompt and interval for blank or invalid inputs", async () => {
    const { startBot } = seedTabbedView({
      paneConfig: pane({
        prompt: "Saved prompt ignored by default fallback",
        min_iteration_interval_minutes: 3,
      }),
    });

    const { interval, prompt } = await openStartBotModal();

    fireEvent.change(prompt, { target: { value: "   " } });
    fireEvent.change(interval, { target: { value: "not-a-number" } });
    fireEvent.click(screen.getByRole("button", { name: "Start Bot" }));

    expect(startBot).toHaveBeenCalledWith(
      PANE_ID,
      CLASSIC_TODO_BOT_LOOP_PROMPT,
      15,
      "default",
    );
  });

  it("clamps negative decimal intervals to a nonnegative integer", async () => {
    const { startBot } = seedTabbedView();
    const { interval, prompt } = await openStartBotModal();

    fireEvent.change(prompt, { target: { value: "Keep going" } });
    fireEvent.change(interval, { target: { value: "-2.7" } });
    fireEvent.click(screen.getByRole("button", { name: "Start Bot" }));

    expect(startBot).toHaveBeenCalledWith(PANE_ID, "Keep going", 0, "default");
  });

  it("closes from Cancel or backdrop without starting the bot", async () => {
    const { startBot } = seedTabbedView({ paneConfig: pane({ prompt: "Saved worker loop" }) });

    await openStartBotModal();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(startBot).not.toHaveBeenCalled();
    expect(screen.queryByText("Start Bot on Worker")).toBeNull();

    fireEvent.click(await screen.findByTitle("Start autonomous bot execution in this tab"));
    fireEvent.click(startBotBackdrop());

    expect(startBot).not.toHaveBeenCalled();
    expect(screen.queryByText("Start Bot on Worker")).toBeNull();
  });
});
