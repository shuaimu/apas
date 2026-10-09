import { describe, expect, it } from "vitest";
import { buildProjectList, projectHue, projectInitials } from "./projectList";
import type { MachineWithProjects, SessionInfo } from "./store";

describe("projectInitials", () => {
  it("takes the first letter of the first two words", () => {
    expect(projectInitials("my-project")).toBe("MP");
    expect(projectInitials("apas_web")).toBe("AW");
    expect(projectInitials("foo.bar.baz")).toBe("FB");
    expect(projectInitials("hello world")).toBe("HW");
  });

  it("takes the first two letters of a one-word name", () => {
    expect(projectInitials("apas")).toBe("AP");
    expect(projectInitials("x")).toBe("X");
  });

  it("handles non-ASCII names without dropping them", () => {
    expect(projectInitials("项目")).toBe("项目");
    expect(projectInitials("ünïcode-test")).toBe("ÜT");
  });

  it("falls back to a placeholder when the name has no letters", () => {
    expect(projectInitials("---")).toBe("?");
    expect(projectInitials("")).toBe("?");
  });
});

describe("projectHue", () => {
  it("is stable for the same id and within the hue circle", () => {
    const hue = projectHue("project-a");
    expect(hue).toBe(projectHue("project-a"));
    expect(hue).toBeGreaterThanOrEqual(0);
    expect(hue).toBeLessThan(360);
  });

  it("separates ids that differ only slightly", () => {
    expect(projectHue("project-a")).not.toBe(projectHue("project-b"));
  });
});

function registration(machineId: string, hostname: string, isRunning = false): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x64" },
    clusterOwnerUserId: "owner",
    clusterAccess: "owner",
    projects: [{ projectId: "registered-project", name: "q-index", path: "/work/q-index", isRunning }],
  };
}

describe("registered project visibility", () => {
  it("adds owned registrations without turning shared compute into project access", () => {
    const owned = registration("owned-machine", "owned-host");
    const shared = {
      ...registration("shared-machine", "shared-host"),
      clusterAccess: "member" as const,
    };

    expect(buildProjectList([], [], [shared])).toEqual([]);
    const [project] = buildProjectList([], [], [owned, shared]);
    expect(project).toMatchObject({
      projectId: "registered-project",
      sessionId: null,
      name: "q-index",
      workingDir: "/work/q-index",
      isActive: false,
    });
    expect(project).not.toHaveProperty("panes");
    expect(project).not.toHaveProperty("createdAt");

    const authorizedSession: SessionInfo = {
      id: "authorized-session",
      projectId: "registered-project",
      status: "inactive",
      isShared: true,
    };
    expect(buildProjectList([authorizedSession], [], [shared])).toMatchObject([
      { projectId: "registered-project", sessionId: "authorized-session", isShared: true },
    ]);
  });

  it("deduplicates hosts with a stable navigation target and prefers a running placement", () => {
    const first = registration("machine-a", "alpha");
    const second = registration("machine-b", "beta");
    const projects = buildProjectList([], [], [second, first]);
    expect(projects.map((project) => project.projectId)).toEqual(["registered-project"]);
    expect(buildProjectList([], [], [first, second])).toEqual(projects);
    expect(new URL(projects[0].machinesHref!, "https://apas.invalid").searchParams.get("machine"))
      .toBe("machine-a");

    second.projects[0].isRunning = true;
    const [running] = buildProjectList([], [], [first, second]);
    expect(running.isActive).toBe(true);
    expect(running.sessionId).toBeNull();
    expect(new URL(running.machinesHref!, "https://apas.invalid").searchParams.get("machine"))
      .toBe("machine-b");
  });

  it("replaces registration with the real active session rather than inventing an attachment id", () => {
    const inventory = registration("machine-a", "alpha");
    const history: SessionInfo = {
      id: "history-session",
      projectId: "registered-project",
      workingDir: "/work/q-index",
      status: "inactive",
      createdAt: "2026-10-09T01:00:00Z",
    };
    const active: SessionInfo = {
      ...history,
      id: "actual-session",
      status: "active",
      isActive: true,
      createdAt: "2026-10-08T01:00:00Z",
      gitRemote: "github.com/example/q-index",
      panes: [{ pane_id: 42, kind: "terminal", provider: "codex", is_working: true }],
    };
    expect(buildProjectList([], [], [inventory])[0].sessionId).toBeNull();
    const projects = buildProjectList([history, active], [], [inventory]);
    expect(projects.map((project) => project.sessionId)).toEqual(["actual-session"]);
    expect(projects[0]).toMatchObject({
      isActive: true,
      gitRemote: "github.com/example/q-index",
      panes: [{ pane_id: 42, is_working: true }],
    });
    expect(projects[0]).not.toHaveProperty("machinesHref");
  });
});
