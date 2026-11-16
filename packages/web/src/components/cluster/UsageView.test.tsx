import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { UsageView, costText, usageProjectContext, usageRows } from "./UsageView";
import type { ClusterProject, ClusterUsageProject, ClusterUsageReport, UsageCounters } from "./types";

const initialStore = useStore.getState();
afterEach(() => {
  cleanup();
  act(() => { useStore.setState(initialStore, true); });
});

function counters(tokens: number, prompts = tokens > 0 ? 1 : 0): UsageCounters {
  return { prompts, responses: prompts, input_tokens: tokens, output_tokens: 0, cost_usd: 0, cost_usd_reported: false };
}

function usageProject(id: string, tokens: number, owner = "me@example.com"): ClusterUsageProject {
  const c = counters(tokens);
  return { project_id: id, project_name: id, owner_email: owner, usage: { lifetime: c, last_7d: c, today: c } };
}

function clusterProject(id: string, hostname: string): ClusterProject {
  return {
    id,
    project_name: id,
    hostname,
    owner_user_id: "me",
    owner_email: "me@example.com",
    lifecycle_status: "active",
    member_count: 0,
    active_session_count: 0,
    hosting_emails: [],
    connected: true,
    effective_policy: { team_available: false, allowed_launch_profiles: [], version: 1, project_suspended: false },
  };
}

function report(projects: ClusterUsageProject[]): ClusterUsageReport {
  const total = counters(0);
  return { lifetime: total, last_7d: total, today: total, projects };
}

describe("usageRows", () => {
  it("orders by tokens, then name, and keeps the idle projects apart", () => {
    const rows = usageRows([usageProject("b", 10), usageProject("a", 10), usageProject("c", 50), usageProject("idle", 0)], "last_7d");
    expect(rows.used.map((row) => row.project_id)).toEqual(["c", "a", "b"]);
    expect(rows.idle.map((row) => row.project_id)).toEqual(["idle"]);
  });

  it("lists every project when none was used", () => {
    const rows = usageRows([usageProject("b", 0), usageProject("a", 0)], "today");
    expect(rows.used.map((row) => row.project_id)).toEqual(["a", "b"]);
    expect(rows.idle).toEqual([]);
  });
});

describe("costText", () => {
  it("shows a reported zero as money and keeps one blank for unreported cost", () => {
    expect(costText(0, true)).toBe("$0.00");
    expect(costText(1.5, true)).toBe("$1.50");
    expect(costText(0, false)).toBe("Unavailable");
    expect(costText(3, false)).toBe("Unavailable");
  });
});

describe("usageProjectContext", () => {
  it("joins the machine and another owner, leaving out the viewer", () => {
    const projects = [clusterProject("a", "zoo-005")];
    expect(usageProjectContext(usageProject("a", 1, "other@example.com"), projects, "me@example.com")).toBe("zoo-005 · other@example.com");
    expect(usageProjectContext(usageProject("a", 1), projects, "me@example.com")).toBe("zoo-005");
    expect(usageProjectContext(usageProject("unknown", 1), projects, "me@example.com")).toBe("");
  });
});

describe("UsageView", () => {
  function renderView(projects: ClusterUsageProject[], onOpenProject = vi.fn()) {
    render(
      <UsageView
        usage={report(projects)}
        load="loaded"
        period="last_7d"
        onPeriod={vi.fn()}
        clusterProjects={[clusterProject("p1", "zoo-005")]}
        userEmail="me@example.com"
        onOpenProject={onOpenProject}
      />,
    );
    return onOpenProject;
  }

  it("folds after fifteen used projects and folds the idle ones apart", () => {
    const used = Array.from({ length: 17 }, (_, index) => usageProject(`p${index + 1}`, 1000 - index));
    renderView([...used, usageProject("idle-1", 0), usageProject("idle-2", 0)]);
    expect(screen.queryByText("p16")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show 2 more projects" }));
    expect(screen.getByText("p17")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Show fewer" })).toBeTruthy();

    expect(screen.queryByText("idle-1")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show 2 projects with no usage" }));
    expect(screen.getByText("idle-1")).toBeTruthy();
  });

  it("links a project the cluster lists to Projects", () => {
    const onOpenProject = renderView([usageProject("p1", 5), usageProject("elsewhere", 4)]);
    fireEvent.click(screen.getByRole("button", { name: "Open p1 in Projects" }));
    expect(onOpenProject).toHaveBeenCalledWith("p1");
    expect(screen.queryByRole("button", { name: "Open elsewhere in Projects" })).toBeNull();
    const table = screen.getByRole("table");
    expect(within(table).getByText("zoo-005")).toBeTruthy();
  });

  it("says when nothing is recorded, and only after loading", () => {
    render(<UsageView usage={null} load="loading" period="today" onPeriod={vi.fn()} clusterProjects={[]} userEmail={null} onOpenProject={vi.fn()} />);
    expect(screen.getByText("Loading usage…")).toBeTruthy();
    expect(screen.queryByText("No usage recorded in this cluster yet.")).toBeNull();
    cleanup();
    render(<UsageView usage={null} load="loaded" period="today" onPeriod={vi.fn()} clusterProjects={[]} userEmail={null} onOpenProject={vi.fn()} />);
    expect(screen.getByText("No usage recorded in this cluster yet.")).toBeTruthy();
  });
});
