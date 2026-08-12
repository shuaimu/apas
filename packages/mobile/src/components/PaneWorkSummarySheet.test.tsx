import { render } from "@testing-library/react-native";
import type { PaneWorkSummary } from "@apas/protocol";

import {
  canRetrySummary,
  PaneWorkSummarySheet,
  summaryAvailabilityMessage,
  summaryStatusLabel,
} from "./PaneWorkSummarySheet";
import { paneWorkSummaryKey, useMobileStore } from "@/state/store";

const mockSend = jest.fn(() => true);

jest.mock("@/connection/runtime", () => ({
  connectionSupervisor: () => ({ send: mockSend }),
}));
jest.mock("@/storage/cache", () => ({
  readPaneWorkSummarySnapshot: jest.fn(() => new Promise(() => undefined)),
}));

const sessionId = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";

function summary(status: PaneWorkSummary["status"], paneId = 3): PaneWorkSummary {
  return {
    session_id: sessionId,
    pane_id: paneId,
    window_start: `2026-08-08T${paneId === 3 ? "09" : "12"}:00:00Z`,
    window_end: `2026-08-08T${paneId === 3 ? "12" : "15"}:00:00Z`,
    status,
    summary: `${status} pane ${paneId}`,
    source_through: "2026-08-08T11:45:00Z",
    source_message_count: 5,
    provider: "codex",
    model: "gpt-5.6",
    error: status === "failed" ? "Worker unavailable" : null,
  };
}

describe("native pane work summary presentation", () => {
  beforeEach(() => {
    useMobileStore.getState().reset();
    mockSend.mockClear();
  });

  it("labels every durable state and only retries failed records", () => {
    const cases: [PaneWorkSummary["status"], string][] = [
      ["complete", "Complete"],
      ["partial", "In progress"],
      ["queued", "Queued"],
      ["generating", "Generating"],
      ["stale", "Updating"],
      ["failed", "Failed"],
      ["source_expired", "Source expired"],
    ];
    for (const [status, label] of cases) expect(summaryStatusLabel(summary(status))).toBe(label);
    expect(canRetrySummary(summary("failed"))).toBe(true);
    expect(canRetrySummary(summary("source_expired"))).toBe(false);
  });

  it("explains every provider availability state", () => {
    expect(summaryAvailabilityMessage("available")).toBeNull();
    expect(summaryAvailabilityMessage("cli_update_required")).toMatch(/CLI needs an update/);
    expect(summaryAvailabilityMessage("summarizer_disabled")).toMatch(/disabled/);
    expect(summaryAvailabilityMessage("summarizer_unavailable")).toMatch(/unavailable/);
    expect(summaryAvailabilityMessage("unknown")).toMatch(/not been confirmed/);
  });

  it("renders only the selected pane in an independently scrollable phone sheet", () => {
    useMobileStore.setState({
      connection: "offline",
      negotiatedCapabilities: ["pane_work_summary_v1"],
      paneWorkSummaries: {
        [paneWorkSummaryKey(sessionId, 3)]: {
          summaries: [summary("partial", 3)],
          availability: "available",
          loading: false,
          error: null,
          updatedAt: "2026-08-12T12:00:00Z",
        },
        [paneWorkSummaryKey(sessionId, 4)]: {
          summaries: [summary("complete", 4)],
          availability: "available",
          loading: false,
          error: null,
          updatedAt: "2026-08-12T12:01:00Z",
        },
      },
    });
    const view = render(<PaneWorkSummarySheet
      visible
      sessionId={sessionId}
      paneId={3}
      paneLabel="Codex 3"
      panes={[{ id: 3, label: "Codex 3" }, { id: 4, label: "Claude 4" }]}
      onSelectPane={jest.fn()}
      onClose={jest.fn()}
    />);

    expect(view.getByText("partial pane 3")).toBeTruthy();
    expect(view.queryByText("complete pane 4")).toBeNull();
    expect(view.getByTestId("summary-scroll")).toBeTruthy();
    expect(view.getByText(/Through/)).toBeTruthy();
    expect(view.getByText("5 source events")).toBeTruthy();
    expect(view.getByText("via codex · gpt-5.6")).toBeTruthy();
  });
});
