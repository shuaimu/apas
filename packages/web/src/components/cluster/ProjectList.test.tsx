import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ProjectList, projectRowLabel } from "./ProjectList";
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
    // Nobody has recorded it: shown from what runs, and the chip says so.
    expect(chip("Manage apas")?.textContent).toBe("Closed");
    expect(chip("Manage apas")?.getAttribute("title"))
      .toBe("Nobody has opened or closed this project yet. It is shown from what runs.");
    expect(chip("Manage idle")).toBeNull();
  });

  it("shows the state from what runs on a server that records none, without the note", () => {
    render(<ProjectList projects={projects} userId="me" openState={{ supported: false, machines, sessions: [] }} selectedId={null} onSelect={vi.fn()} />);

    expect(chip("Manage mako")?.textContent).toBe("Open");
    expect(chip("Manage apas")?.textContent).toBe("Closed");
    expect(chip("Manage apas")?.getAttribute("title")).toBeNull();
  });
});
