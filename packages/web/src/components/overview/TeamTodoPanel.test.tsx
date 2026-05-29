import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { parsePrLine } from "./TeamTodoPanel";
import { TeamTodoPanel } from "./TeamTodoPanel";
import { useStore, type TeamTodoState } from "@/lib/store";

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
  useStore.setState({
    sessionId: "test-session",
    teamTodoState: state,
    fetchTeamTodo: vi.fn(),
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

describe("PrStateBadge fetch-driven color", () => {
  const PR_URL = "https://github.com/shuaimu/apas/pull/1";
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    seedTeamTodo(mkGlobal("pr_open", PR_URL));
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    useStore.setState({ sessionId: null, teamTodoState: null });
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

  it("skips fetch entirely and shows MERGED when global is status: done", async () => {
    seedTeamTodo(mkGlobal("done", PR_URL));
    const fetchSpy = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({}),
    });
    globalThis.fetch = fetchSpy as unknown as typeof fetch;
    render(<TeamTodoPanel />);
    const badge = await screen.findByTestId("pr-state-badge");
    expect(badge.getAttribute("data-pr-state")).toBe("done");
    expect(badge.textContent).toBe("MERGED");
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
