import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ProjectList, projectFacts, projectRowLabel } from "./ProjectList";
import type { ClusterProject } from "./types";

afterEach(cleanup);

function project(id: string, name: string, hostname: string): ClusterProject {
  return {
    id,
    project_name: name,
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

describe("projectRowLabel", () => {
  it("names the machine only when another project has the same name", () => {
    const projects = [project("a", "mako", "zoo-005"), project("b", "mako", "zoo-006"), project("c", "apas", "zoo-005")];
    expect(projectRowLabel(projects[0], projects)).toBe("Manage mako on zoo-005");
    expect(projectRowLabel(projects[1], projects)).toBe("Manage mako on zoo-006");
    expect(projectRowLabel(projects[2], projects)).toBe("Manage apas");
  });
});

describe("ProjectList open state", () => {
  const projects = [project("a", "mako", "zoo-005"), project("b", "apas", "zoo-005"), project("c", "idle", "zoo-006")];
  const machines = [{
    machine: { machineId: "m5", hostname: "zoo-005", os: "linux", arch: "x64" },
    projects: [
      { projectId: "a", path: "/work/mako", isRunning: true, openState: "open" as const, part: "running" as const },
      { projectId: "b", path: "/work/apas", isRunning: false },
    ],
    clusterAccess: "owner" as const,
    daemonConnected: true,
  }];
  const chip = (name: string) => within(screen.getByRole("button", { name })).queryByTestId("project-open-state");

  it("shows Open or Closed per project, and nothing for one it knows nothing about", () => {
    render(<ProjectList projects={projects} userId="me" openState={{ supported: true, machines, sessions: [] }} selectedId={null} onSelect={vi.fn()} />);

    expect(chip("Manage mako")?.textContent).toBe("Open");
    expect(chip("Manage mako")?.getAttribute("title")).toBeNull();
    // The server has recorded no state and nothing runs it: Closed, and
    // nothing says it is not recorded.
    expect(chip("Manage apas")?.textContent).toBe("Closed");
    expect(chip("Manage apas")?.getAttribute("title")).toBeNull();
    expect(document.body.textContent).not.toMatch(/Nobody has opened|Keep .*open/i);
    expect(chip("Manage idle")).toBeNull();
  });

  it("shows the state from what runs on a server that records none", () => {
    render(<ProjectList projects={projects} userId="me" openState={{ supported: false, machines, sessions: [] }} selectedId={null} onSelect={vi.fn()} />);

    expect(chip("Manage mako")?.textContent).toBe("Open");
    expect(chip("Manage apas")?.textContent).toBe("Closed");
    expect(chip("Manage apas")?.getAttribute("title")).toBeNull();
  });
});

describe("a project list row beside a known open state", () => {
  // What the cluster list sends: `connected` and the session count are about
  // the project's session (is a CLI of it attached now), and `hostname` is
  // the one host recorded for that session.
  const closed = { ...project("closed", "os-closed", "zoo-006"), connected: false };
  const two = { ...project("two", "os-two", "zoo-005"), active_session_count: 1 };
  const unknown = { ...project("unknown", "elsewhere", "zoo-009"), connected: false, active_session_count: 1 };
  const entry = (machineId: string, hostname: string, projects: { projectId: string; isRunning: boolean; openState: "open" | "closed"; part: "running" | "not_started" }[]) => ({
    machine: { machineId, hostname, os: "linux", arch: "x64" },
    projects: projects.map((item) => ({ path: `/work/${item.projectId}`, ...item })),
    clusterAccess: "owner" as const,
    daemonConnected: true,
    projectInstancesAvailable: true,
  });
  // Both machines are connected. os-closed is closed with tabs on both;
  // os-two is open and runs on both.
  const machines = [
    entry("m5", "zoo-005", [
      { projectId: "closed", isRunning: false, openState: "closed", part: "not_started" },
      { projectId: "two", isRunning: true, openState: "open", part: "running" },
    ]),
    entry("m6", "zoo-006", [
      { projectId: "closed", isRunning: false, openState: "closed", part: "not_started" },
      { projectId: "two", isRunning: true, openState: "open", part: "running" },
    ]),
  ];
  const row = (name: string) => screen.getByRole("button", { name });

  it("shows no Offline or Connected chip beside Open or Closed", () => {
    render(<ProjectList projects={[closed, two, unknown]} userId="me" openState={{ supported: true, machines, sessions: [] }} selectedId={null} onSelect={vi.fn()} />);

    // "Offline" beside "Closed" read as a second state, while both of the
    // project's machines were connected.
    expect(within(row("Manage os-closed")).getByTestId("project-open-state").textContent).toBe("Closed");
    expect(within(row("Manage os-closed")).queryByText("Offline")).toBeNull();
    expect(within(row("Manage os-two")).getByTestId("project-open-state").textContent).toBe("Open");
    expect(within(row("Manage os-two")).queryByText("Connected")).toBeNull();
    // Nothing is known of this one's open state: the older chip stays.
    expect(within(row("Manage elsewhere")).queryByTestId("project-open-state")).toBeNull();
    expect(within(row("Manage elsewhere")).getByText("Offline")).toBeTruthy();
  });

  it("names the machines the open state names, not one host and a session count", () => {
    render(<ProjectList projects={[closed, two, unknown]} userId="me" openState={{ supported: true, machines, sessions: [] }} selectedId={null} onSelect={vi.fn()} />);

    // It was "zoo-005 · 1 active session" for a project open on two machines.
    expect(row("Manage os-two").textContent).toContain("Running on zoo-005 and zoo-006");
    expect(row("Manage os-two").textContent).not.toContain("active session");
    // Runs nowhere: the machines that have its tabs.
    expect(row("Manage os-closed").textContent).toContain("zoo-005 and zoo-006");
    expect(row("Manage os-closed").textContent).not.toContain("Running on");
    // Without an open state the line is what it was.
    expect(row("Manage elsewhere").textContent).toContain("zoo-009 · 1 active session");
  });

  it("keeps the older line and chip when no open state is given at all", () => {
    expect(projectFacts(two, "me")).toBe("zoo-005 · 1 active session");
    expect(projectFacts(two, "someone-else")).toBe("Owner me@example.com · zoo-005 · 1 active session");
    expect(projectFacts({ ...two, member_count: 2 }, "me", { runningHosts: ["zoo-005", "zoo-006"], machines: [] }))
      .toBe("Running on zoo-005 and zoo-006 · 2 members");
    render(<ProjectList projects={[two]} userId="me" selectedId={null} onSelect={vi.fn()} />);
    expect(within(row("Manage os-two")).getByText("Connected")).toBeTruthy();
  });
});
