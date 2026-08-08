import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { parsePrLine } from "./TeamTodoPanel";
import { TeamTodoPanel } from "./TeamTodoPanel";
import { paneKey, useStore, type Message, type PaneConfig, type TeamTodoState } from "@/lib/store";

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
  reviewerLastActivity = null,
  reviewerCursor = null,
  reviewerPresent = false,
}: {
  lastActivity: Date | null;
  cursor?: string | null;
  present?: boolean;
  reviewerLastActivity?: Date | null;
  reviewerCursor?: string | null;
  reviewerPresent?: boolean;
}) {
  const paneId = 178;
  const reviewerPaneId = 4;
  const messages: Record<string, Message[]> = {};
  if (lastActivity != null) {
    messages[paneKey(paneId)] = [{
      id: "tech-lead-activity",
      role: "assistant",
      content: "activity",
      timestamp: lastActivity,
    }];
  }
  if (reviewerLastActivity != null) {
    messages[paneKey(reviewerPaneId)] = [{
      id: "reviewer-activity",
      role: "assistant",
      content: "activity",
      timestamp: reviewerLastActivity,
    }];
  }
  const techLeadPane: PaneConfig = {
    pane_id: paneId,
    provider: "claude",
    mode: "deadloop",
    session_id: "tech-lead-session",
    is_paused: false,
    role: "tech lead",
  };
  const reviewerPane: PaneConfig = {
    pane_id: reviewerPaneId,
    provider: "claude",
    mode: "deadloop",
    session_id: "reviewer-session",
    is_paused: false,
    role: "reviewer",
  };
  const paneConfigs = [
    ...(present ? [techLeadPane] : []),
    ...(reviewerPresent ? [reviewerPane] : []),
  ];
  act(() => {
    useStore.setState({
      sessionId: "test-session",
      teamTodoStates: new Map([
        [
          "test-session",
          {
            ...emptyTeamTodo(),
            tech_lead_cursor: cursor,
            reviewer_cursor: reviewerCursor,
          },
        ],
      ]),
      fetchTeamTodo: vi.fn(),
      paneConfigs,
      paneMessages: messages,
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

  it("renders annotated merged metadata without changing the PR link href", async () => {
    const annotation = "(MERGED 2026-06-17T05:33:24Z a696d8e...)";
    seedTeamTodo({
      globals: [
        {
          id: "TODO-001",
          title: "test",
          status: "pr_open",
          origin: "tech-lead",
          prs: [{ pane_id: 568, url: PR_URL, annotation }],
          body: "",
        },
      ],
      workers: [],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });
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
    const link = screen.getByRole("link", {
      name: /PR #1 \(pane 568\)/,
    }) as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe(PR_URL);
    expect(
      screen.getByLabelText(
        "PR annotation: merged 2026-06-17T05:33:24Z a696d8e",
      ),
    ).toBeTruthy();
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
    expect(screen.queryByTestId("pr-readiness-badge")).toBeNull();
  });

  it("renders ready readiness when review, checks, and mergeability are clear", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({
          state: "open",
          merged: false,
          mergeable: true,
          mergeable_state: "clean",
          statuses_url: "https://api.github.com/repos/shuaimu/apas/statuses/abc",
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => [{ context: "ci", state: "success" }],
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => [
          { state: "COMMENTED", submitted_at: "2026-06-18T09:00:00Z" },
          { state: "APPROVED", submitted_at: "2026-06-18T09:01:00Z" },
        ],
      });
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    render(<TeamTodoPanel />);

    const readiness = await screen.findByTestId("pr-readiness-badge");
    await waitFor(() =>
      expect(readiness.getAttribute("data-pr-readiness")).toBe("ready"),
    );
    expect(readiness.textContent).toBe("ready");
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it("renders changes-requested readiness from PR reviews data", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({
          state: "open",
          merged: false,
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => [
          { state: "APPROVED", submitted_at: "2026-06-18T09:00:00Z" },
          {
            state: "CHANGES_REQUESTED",
            submitted_at: "2026-06-18T09:01:00Z",
          },
        ],
      });
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    render(<TeamTodoPanel />);

    const readiness = await screen.findByTestId("pr-readiness-badge");
    await waitFor(() =>
      expect(readiness.getAttribute("data-pr-readiness")).toBe("changes_requested"),
    );
    expect(readiness.textContent).toBe("changes requested");
  });

  it("renders pending-checks readiness from commit status data", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({
          state: "open",
          merged: false,
          mergeable: true,
          mergeable_state: "clean",
          statuses_url: "https://api.github.com/repos/shuaimu/apas/statuses/abc",
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => [{ context: "ci", state: "pending" }],
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => [
          { state: "APPROVED", submitted_at: "2026-06-18T09:00:00Z" },
        ],
      });
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    render(<TeamTodoPanel />);

    const readiness = await screen.findByTestId("pr-readiness-badge");
    await waitFor(() =>
      expect(readiness.getAttribute("data-pr-readiness")).toBe("checks_pending"),
    );
    expect(readiness.textContent).toBe("checks pending");
  });

  it("renders failing-checks readiness from commit status data", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({
          state: "open",
          merged: false,
          mergeable: true,
          mergeable_state: "clean",
          statuses_url: "https://api.github.com/repos/shuaimu/apas/statuses/abc",
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => [{ context: "ci", state: "failure" }],
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => [
          { state: "APPROVED", submitted_at: "2026-06-18T09:00:00Z" },
        ],
      });
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    render(<TeamTodoPanel />);

    const readiness = await screen.findByTestId("pr-readiness-badge");
    await waitFor(() =>
      expect(readiness.getAttribute("data-pr-readiness")).toBe("checks_failing"),
    );
    expect(readiness.textContent).toBe("checks failing");
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

  it("renders done Global PR annotations accessibly without fetching", () => {
    const annotation = "(MERGED 2026-06-17T05:33:24Z a696d8e...)";
    seedTeamTodo({
      globals: [
        {
          id: "TODO-001",
          title: "test",
          status: "done",
          origin: "tech-lead",
          prs: [{ pane_id: 568, url: PR_URL, annotation }],
          body: "",
        },
      ],
      workers: [],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });
    const fetchSpy = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({}),
    });
    globalThis.fetch = fetchSpy as unknown as typeof fetch;

    render(<TeamTodoPanel />);

    const link = screen.getByRole("link", {
      name: /PR \(pane 568\)/,
    }) as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe(PR_URL);
    expect(
      screen.getByLabelText(
        "PR annotation: merged 2026-06-17T05:33:24Z a696d8e",
      ),
    ).toBeTruthy();
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

describe("team TODO search filter", () => {
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ state: "open", merged: false }),
    }) as unknown as typeof fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    act(() => {
      useStore.setState({
        sessionId: null,
        teamTodoStates: new Map(),
      });
    });
  });

  it("filters globals by TODO id, title, status, and PR number", () => {
    seedTeamTodo({
      globals: [
        mkTodo(
          "TODO-151",
          "Team TODO search filter",
          "in_progress",
          "Find backlog entries quickly",
        ),
        mkTodo(
          "TODO-152",
          "Legacy project registry migration",
          "proposed",
        ),
        {
          id: "TODO-153",
          title: "Open review branch",
          status: "pr_open",
          origin: "tech-lead",
          prs: [
            {
              pane_id: 568,
              url: "https://github.com/shuaimu/apas/pull/137",
            },
          ],
          body: "",
        },
      ],
      workers: [],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });

    render(<TeamTodoPanel />);

    const input = screen.getByLabelText("Search Team TODOs");

    fireEvent.change(input, { target: { value: "TODO-151" } });
    expect(screen.getByText("Team TODO search filter")).toBeTruthy();
    expect(screen.queryByText("Legacy project registry migration")).toBeNull();
    expect(screen.queryByText("Open review branch")).toBeNull();
    expect(screen.getByText("In progress (1)")).toBeTruthy();

    fireEvent.change(input, { target: { value: "legacy project" } });
    expect(screen.queryByText("Team TODO search filter")).toBeNull();
    expect(screen.getByText("Legacy project registry migration")).toBeTruthy();
    expect(screen.getByText("Proposed (1)")).toBeTruthy();

    fireEvent.change(input, { target: { value: "pr open" } });
    expect(screen.queryByText("Team TODO search filter")).toBeNull();
    expect(screen.queryByText("Legacy project registry migration")).toBeNull();
    expect(screen.getByText("Open review branch")).toBeTruthy();
    expect(screen.getByText("PR open (1)")).toBeTruthy();

    fireEvent.change(input, { target: { value: "137" } });
    expect(screen.getByText("Open review branch")).toBeTruthy();
    expect(screen.getByRole("link", { name: /PR #137/ })).toBeTruthy();
  });

  it("clears the query without changing proposed approval controls", () => {
    seedTeamTodo({
      globals: [
        mkTodo("TODO-160", "Active implementation", "in_progress"),
        mkTodo("TODO-161", "Needs approval", "proposed"),
      ],
      workers: [],
      tech_lead_cursor: null,
      reviewer_cursor: null,
    });

    render(<TeamTodoPanel />);

    const input = screen.getByLabelText("Search Team TODOs");
    fireEvent.change(input, { target: { value: "needs approval" } });
    expect(screen.queryByText("Active implementation")).toBeNull();

    const proposedGroup = screen.getByText("Proposed (1)").closest("section");
    expect(proposedGroup).toBeTruthy();
    expect(within(proposedGroup as HTMLElement).getByTitle(/Approve/)).toBeTruthy();
    expect(within(proposedGroup as HTMLElement).getByTitle("Reject")).toBeTruthy();

    fireEvent.click(screen.getByLabelText("Clear Team TODO search"));

    expect(screen.getByText("Active implementation")).toBeTruthy();
    const restoredProposedGroup = screen
      .getByText("Proposed (1)")
      .closest("section");
    expect(restoredProposedGroup).toBeTruthy();
    expect(
      within(restoredProposedGroup as HTMLElement).getByTitle(/Approve/),
    ).toBeTruthy();
    expect(
      within(restoredProposedGroup as HTMLElement).getByTitle("Reject"),
    ).toBeTruthy();
  });
});

describe("worker subtask lifecycle rows", () => {
  afterEach(() => {
    act(() => {
      useStore.setState({
        sessionId: null,
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

  it("renders Tech Lead and Reviewer cursor ages with raw cursor titles", () => {
    const techLeadCursor = "2026-06-16T11:58:00Z";
    const reviewerCursor = "2026-06-16T11:45:00Z";
    seedAgentStatus({
      lastActivity: new Date(NOW.getTime() - 60_000),
      cursor: techLeadCursor,
      reviewerLastActivity: new Date(NOW.getTime() - 10 * 60_000),
      reviewerCursor,
      reviewerPresent: true,
    });

    render(<TeamTodoPanel />);

    const techLeadLine = screen.getByTitle(`cursor: ${techLeadCursor}`);
    const reviewerLine = screen.getByTitle(`cursor: ${reviewerCursor}`);
    expect(techLeadLine.textContent).toContain("Tech Lead");
    expect(techLeadLine.textContent).toContain("cursor 2m ago");
    expect(reviewerLine.textContent).toContain("Reviewer");
    expect(reviewerLine.textContent).toContain("cursor 15m ago");
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
