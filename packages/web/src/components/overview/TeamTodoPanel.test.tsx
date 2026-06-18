import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import { parsePrLine } from "./TeamTodoPanel";
import { TeamTodoPanel } from "./TeamTodoPanel";
import { paneKey, useStore, type PaneConfig, type TeamTodoState } from "@/lib/store";

describe("parsePrLine", () => {
  it("parses a well-formed line", () => {
    const r = parsePrLine("pr: 218 https://github.com/shuaimu/apas/pull/1");
    expect(r).toEqual({
      pane: 218,
      url: "https://github.com/shuaimu/apas/pull/1",
      owner: "shuaimu",
      repo: "apas",
      num: 1,
    });
  });

  it("tolerates trailing whitespace", () => {
    const r = parsePrLine("pr: 7 https://github.com/o/r/pull/42   ");
    expect(r?.num).toBe(42);
    expect(r?.owner).toBe("o");
  });

  it("parses an annotated merged line with a clean pull URL", () => {
    const r = parsePrLine(
      "pr: 568 https://github.com/shuaimu/apas/pull/12 (MERGED 2026-06-16T03:59:12Z 7d78b3e...)",
    );
    expect(r).toEqual({
      pane: 568,
      url: "https://github.com/shuaimu/apas/pull/12",
      owner: "shuaimu",
      repo: "apas",
      num: 12,
    });
  });

  it("returns null for a missing pr: prefix", () => {
    expect(parsePrLine("218 https://github.com/o/r/pull/1")).toBeNull();
  });

  it("returns null for a non-GitHub URL", () => {
    expect(
      parsePrLine("pr: 218 https://gitlab.com/o/r/-/merge_requests/1"),
    ).toBeNull();
  });

  it("returns null for a GitHub URL that isn't a /pull/", () => {
    expect(
      parsePrLine("pr: 218 https://github.com/o/r/issues/1"),
    ).toBeNull();
  });

  it("returns null for empty input", () => {
    expect(parsePrLine("")).toBeNull();
  });

  it("returns null when pane id is non-numeric", () => {
    expect(
      parsePrLine("pr: abc https://github.com/o/r/pull/1"),
    ).toBeNull();
  });

  it("returns null when PR number is non-numeric", () => {
    expect(
      parsePrLine("pr: 1 https://github.com/o/r/pull/notanum"),
    ).toBeNull();
  });

  it("parses multiple distinct lines (multi-pr per Global)", () => {
    const lines = [
      "pr: 218 https://github.com/shuaimu/apas/pull/1",
      "pr: 676 https://github.com/shuaimu/apas/pull/2",
    ];
    const parsed = lines.map(parsePrLine);
    expect(parsed[0]?.num).toBe(1);
    expect(parsed[0]?.pane).toBe(218);
    expect(parsed[1]?.num).toBe(2);
    expect(parsed[1]?.pane).toBe(676);
  });
});

// --- UI test: PrStateBadge color via stubbed fetch ----------------------

function seedTeamTodo(state: TeamTodoState) {
  act(() => {
    useStore.setState({
      sessionId: "test-session",
      teamTodoState: state,
      teamTodoStates: new Map([["test-session", state]]),
      fetchTeamTodo: vi.fn(),
    });
  });
}

function mkGlobal(status: string, prUrl: string) {
  return {
    globals: [
      {
        id: "TODO-001",
        title: "test",
        status,
        origin: "tech-lead",
        prs: [{ pane_id: 218, url: prUrl }],
        body: "",
      },
    ],
    workers: [],
    tech_lead_cursor: null,
    reviewer_cursor: null,
  } as TeamTodoState;
}

function emptyTeamTodo(): TeamTodoState {
  return {
    globals: [],
    workers: [],
    tech_lead_cursor: null,
    reviewer_cursor: null,
  };
}

function mkTodo(
  id: string,
  title: string,
  status: string,
  body = "",
) {
  return {
    id,
    title,
    status,
    origin: "tech-lead",
    prs: [],
    body,
  };
}

function seedAgentStatus({
  lastActivity,
  cursor = null,
  present = true,
}: {
  lastActivity: Date | null;
  cursor?: string | null;
  present?: boolean;
}) {
  const paneId = 178;
  const messages =
    lastActivity == null
      ? {}
      : { [paneKey(paneId)]: [{ timestamp: lastActivity }] };
  const techLeadPane: PaneConfig = {
    pane_id: paneId,
    provider: "claude",
    mode: "deadloop",
    session_id: "tech-lead-session",
    is_paused: false,
    role: "tech lead",
  };
  act(() => {
    useStore.setState({
      sessionId: "test-session",
      teamTodoState: emptyTeamTodo(),
      teamTodoStates: new Map([
        [
          "test-session",
          {
            ...emptyTeamTodo(),
            tech_lead_cursor: cursor,
          },
        ],
      ]),
      fetchTeamTodo: vi.fn(),
      paneConfigs: present ? [techLeadPane] : [],
      paneMessages: messages as never,
    });
  });
}

describe("PrStateBadge fetch-driven color", () => {
  const PR_URL = "https://github.com/shuaimu/apas/pull/1";
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    seedTeamTodo(mkGlobal("pr_open", PR_URL));
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    act(() => {
      useStore.setState({
        sessionId: null,
        teamTodoState: null,
        teamTodoStates: new Map(),
      });
    });
  });

  it("renders MERGED (green) when GitHub reports merged", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ state: "closed", merged: true }),
    }) as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    const badge = await screen.findByTestId("pr-state-badge");
    await waitFor(() =>
      expect(badge.getAttribute("data-pr-state")).toBe("merged"),
    );
    expect(badge.textContent).toBe("MERGED");
  });

  it("renders OPEN (amber) when GitHub reports open", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ state: "open", merged: false }),
    }) as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    const badge = await screen.findByTestId("pr-state-badge");
    await waitFor(() =>
      expect(badge.getAttribute("data-pr-state")).toBe("open"),
    );
    expect(badge.textContent).toBe("OPEN");
  });

  it("renders CLOSED (gray) when GitHub reports closed without merge", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ state: "closed", merged: false }),
    }) as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    const badge = await screen.findByTestId("pr-state-badge");
    await waitFor(() =>
      expect(badge.getAttribute("data-pr-state")).toBe("closed"),
    );
    expect(badge.textContent).toBe("CLOSED");
  });

  it("renders error (red) on a 403 rate-limit response", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 403,
      json: async () => ({}),
    }) as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    const badge = await screen.findByTestId("pr-state-badge");
    await waitFor(() =>
      expect(badge.getAttribute("data-pr-state")).toBe("error"),
    );
  });

  it("renders error (red) when fetch rejects", async () => {
    globalThis.fetch = vi
      .fn()
      .mockRejectedValue(new Error("network")) as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    const badge = await screen.findByTestId("pr-state-badge");
    await waitFor(() =>
      expect(badge.getAttribute("data-pr-state")).toBe("error"),
    );
  });

  it("skips fetch entirely and renders only the PR link when global is status: done", () => {
    seedTeamTodo(mkGlobal("done", PR_URL));
    const fetchSpy = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({}),
    });
    globalThis.fetch = fetchSpy as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    expect(screen.queryByTestId("pr-state-badge")).toBeNull();
    const link = screen.getByRole("link", {
      name: /PR \(pane 218\)/,
    }) as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe(PR_URL);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("dedupes by URL when a Global lists the same PR twice", async () => {
    const dup: TeamTodoState = {
      globals: [
        {
          id: "TODO-001",
          title: "test",
          status: "pr_open",
          origin: "tech-lead",
          prs: [
            { pane_id: 218, url: PR_URL },
            { pane_id: 676, url: PR_URL },
          ],
          body: "",
        },
      ],
      workers: [],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    };
    seedTeamTodo(dup);
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ state: "open", merged: false }),
    }) as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    const badges = await screen.findAllByTestId("pr-state-badge");
    expect(badges).toHaveLength(1);
  });

  it("renders no PR link when the URL is malformed", () => {
    seedTeamTodo(
      mkGlobal("pr_open", "https://example.com/not-a-github-pull"),
    );
    globalThis.fetch = vi.fn() as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    expect(screen.queryByTestId("pr-state-badge")).toBeNull();
  });
});

describe("active TODO status groups", () => {
  afterEach(() => {
    act(() => {
      useStore.setState({
        sessionId: null,
        teamTodoState: null,
        teamTodoStates: new Map(),
      });
    });
  });

  it("groups active globals by status with active work above proposed backlog", () => {
    seedTeamTodo({
      globals: [
        mkTodo("TODO-005", "proposed backlog", "proposed"),
        mkTodo("TODO-003", "implementation underway", "in_progress"),
        mkTodo("TODO-001", "open PR waiting", "pr_open"),
        mkTodo("TODO-004", "approved not started", "approved"),
        mkTodo("TODO-002", "reviewing diff", "under_review"),
        mkTodo("TODO-006", "finished work", "done"),
        mkTodo("TODO-007", "declined work", "rejected"),
      ],
      workers: [],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });

    render(<TeamTodoPanel />);

    const prOpen = screen.getByText("PR open (1)");
    const underReview = screen.getByText("Under review (1)");
    const inProgress = screen.getByText("In progress (1)");
    const approved = screen.getByText("Approved (1)");
    const proposed = screen.getByText("Proposed (1)");

    expect(
      prOpen.compareDocumentPosition(underReview) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      underReview.compareDocumentPosition(inProgress) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      inProgress.compareDocumentPosition(approved) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      approved.compareDocumentPosition(proposed) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getByText("Done (1)")).toBeTruthy();
    expect(screen.getByText("Rejected (1)")).toBeTruthy();
  });

  it("keeps proposed Approve and Reject actions in the Proposed group", () => {
    seedTeamTodo({
      globals: [
        mkTodo("TODO-010", "active work", "in_progress"),
        mkTodo("TODO-011", "needs approval", "proposed"),
      ],
      workers: [],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });

    render(<TeamTodoPanel />);

    const proposedGroup = screen.getByText("Proposed (1)").closest("section");
    expect(proposedGroup).toBeTruthy();
    const proposedWithin = within(proposedGroup as HTMLElement);
    expect(proposedWithin.getByText("needs approval")).toBeTruthy();
    expect(proposedWithin.getByTitle(/Approve/)).toBeTruthy();
    expect(proposedWithin.getByTitle("Reject")).toBeTruthy();
  });
});

describe("worker subtask lifecycle rows", () => {
  afterEach(() => {
    act(() => {
      useStore.setState({
        sessionId: null,
        teamTodoState: null,
        teamTodoStates: new Map(),
      });
    });
  });

  it("renders each subtask status, including revising under a PR-open Global", () => {
    seedTeamTodo({
      globals: [
        {
          id: "TODO-024",
          title: "conflicting PR",
          status: "pr_open",
          origin: "tech-lead",
          prs: [],
          body: "",
        },
      ],
      workers: [
        {
          pane_id: 568,
          role_hint: "Developer",
          subtasks: [
            {
              id: "TODO-024 · pending",
              title: "Queued implementation",
              status: "pending",
              parent: "TODO-024",
              body: "",
            },
            {
              id: "TODO-024 · active",
              title: "Active implementation",
              status: "in_progress",
              parent: "TODO-024",
              body: "",
            },
            {
              id: "TODO-024 · review",
              title: "Review submitted diff",
              status: "reviewing",
              parent: "TODO-024",
              body: "",
            },
            {
              id: "TODO-024 · conflict",
              title: "Resolve PR conflict",
              status: "revising",
              parent: "TODO-024",
              body: "",
            },
            {
              id: "TODO-024 · approved",
              title: "Open approved PR",
              status: "approved",
              parent: "TODO-024",
              body: "",
            },
            {
              id: "TODO-024 · done",
              title: "Merged cleanup",
              status: "done",
              parent: "TODO-024",
              body: "",
            },
          ],
        },
      ],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });

    render(<TeamTodoPanel />);

    expect(screen.getByText("PR open (1)")).toBeTruthy();
    expect(screen.getByText("conflicting PR")).toBeTruthy();
    for (const [title, status] of [
      ["Queued implementation", "pending"],
      ["Active implementation", "in_progress"],
      ["Review submitted diff", "reviewing"],
      ["Resolve PR conflict", "revising"],
      ["Open approved PR", "approved"],
      ["Merged cleanup", "done"],
    ]) {
      expect(
        screen.getByText((_, element) =>
          element?.tagName.toLowerCase() === "p" &&
          element.textContent?.replace(/\s+/g, " ") ===
            `${title} (${status} · TODO-024)`,
        ),
      ).toBeTruthy();
    }
  });
});

describe("AgentStatusRow accessible indicators", () => {
  const NOW = new Date("2026-06-16T12:00:00Z");

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
    act(() => {
      useStore.setState({
        sessionId: null,
        teamTodoState: null,
        teamTodoStates: new Map(),
        paneConfigs: [],
        paneMessages: {},
      });
    });
  });

  it.each([
    ["active", 60_000, /1m ago/],
    ["recent", 10 * 60_000, /10m ago/],
    ["stale", 45 * 60_000, /45m ago/],
  ])("renders %s activity with an accessible status", (status, ageMs, relativeText) => {
    seedAgentStatus({
      lastActivity: new Date(NOW.getTime() - ageMs),
      cursor: "2026-06-16T11:58:00Z",
    });

    render(<TeamTodoPanel />);

    const badge = screen.getByLabelText(`Agent status: ${status}`);
    expect(badge.getAttribute("data-agent-status")).toBe(status);
    expect(screen.getByText(relativeText)).toBeTruthy();
    expect(screen.getByText(/cursor 2m ago/)).toBeTruthy();
  });

  it("renders unknown activity when a running pane has no messages", () => {
    seedAgentStatus({ lastActivity: null });

    render(<TeamTodoPanel />);

    const badge = screen.getByLabelText("Agent status: unknown");
    expect(badge.getAttribute("data-agent-status")).toBe("unknown");
    expect(screen.getByText("—")).toBeTruthy();
  });

  it("preserves missing-pane text without an activity indicator", () => {
    seedAgentStatus({ lastActivity: null, present: false });

    render(<TeamTodoPanel />);

    expect(screen.getAllByText("not running")).toHaveLength(2);
    expect(screen.queryByLabelText(/Agent status:/)).toBeNull();
  });
});

describe("waiting-for-PR hint", () => {
  const originalFetch = globalThis.fetch;

  afterEach(() => {
    globalThis.fetch = originalFetch;
    act(() => {
      useStore.setState({
        sessionId: null,
        teamTodoState: null,
        teamTodoStates: new Map(),
        paneConfigs: [],
        paneMessages: {},
      });
    });
  });

  it("shows approved-worker handoff when an under-review Global has no PR", () => {
    seedTeamTodo({
      globals: [
        {
          id: "TODO-001",
          title: "waiting for worker PR",
          status: "under_review",
          origin: "tech-lead",
          prs: [],
          body: "",
        },
      ],
      workers: [
        {
          pane_id: 568,
          role_hint: "Developer",
          subtasks: [
            {
              id: "TODO-001 · worker",
              title: "Do the work",
              status: "approved",
              parent: "TODO-001",
              body: "",
            },
          ],
        },
      ],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });

    render(<TeamTodoPanel />);

    expect(
      screen.getByText("Reviewer approved; waiting for pane 568 to open PR"),
    ).toBeTruthy();
  });

  it("hides approved-worker handoff once a PR exists", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ state: "open", merged: false }),
    }) as unknown as typeof fetch;
    seedTeamTodo({
      globals: [
        {
          id: "TODO-001",
          title: "already has PR",
          status: "pr_open",
          origin: "tech-lead",
          prs: [{ pane_id: 568, url: "https://github.com/shuaimu/apas/pull/15" }],
          body: "",
        },
      ],
      workers: [
        {
          pane_id: 568,
          role_hint: "Developer",
          subtasks: [
            {
              id: "TODO-001 · worker",
              title: "Do the work",
              status: "approved",
              parent: "TODO-001",
              body: "",
            },
          ],
        },
      ],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });

    render(<TeamTodoPanel />);

    expect(screen.queryByTestId("waiting-for-pr-hint")).toBeNull();
    const badge = await screen.findByTestId("pr-state-badge");
    await waitFor(() =>
      expect(badge.getAttribute("data-pr-state")).toBe("open"),
    );
  });
});
