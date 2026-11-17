import { describe, expect, it } from "vitest";
import {
  buildProjectList,
  groupProjectsByRepo,
  NO_REMOTE_KEY,
  projectHue,
  projectInitials,
  projectStateLabel,
} from "./projectList";
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

  it("prefers the saved stopped host regardless of report order, but running inventory wins", () => {
    const first = registration("machine-002", "zoo-002");
    const preferred = registration("machine-005", "zoo-005");
    for (const entry of [first, preferred]) entry.projects[0].preferredMachineId = "machine-005";
    for (const machines of [[first, preferred], [preferred, first]]) {
      const [project] = buildProjectList([], [], machines);
      expect(project).toMatchObject({ hostname: "zoo-005", sessionId: null, isActive: false });
      expect(new URL(project.machinesHref!, "https://apas.invalid").searchParams.get("machine"))
        .toBe("machine-005");
    }
    first.projects[0].isRunning = true;
    for (const machines of [[first, preferred], [preferred, first]]) {
      const [project] = buildProjectList([], [], machines);
      expect(project).toMatchObject({ hostname: "zoo-002", sessionId: null, isActive: true });
      expect(new URL(project.machinesHref!, "https://apas.invalid").searchParams.get("machine"))
        .toBe("machine-002");
    }
  });

  it("falls back when the saved host is absent and never grants member inventory access", () => {
    const first = registration("machine-002", "zoo-002");
    const second = registration("machine-003", "zoo-003");
    const member = { ...registration("machine-005", "zoo-005"), clusterAccess: "member" as const };
    for (const entry of [first, second, member]) entry.projects[0].preferredMachineId = "machine-005";
    expect(buildProjectList([], [], [member])).toEqual([]);
    for (const machines of [[second, first], [member, second, first]]) {
      const [project] = buildProjectList([], [], machines);
      expect(project).toMatchObject({ hostname: "zoo-002", sessionId: null, isActive: false });
      expect(new URL(project.machinesHref!, "https://apas.invalid").searchParams.get("machine"))
        .toBe("machine-002");
    }
  });

  it("replaces registration with the real active session rather than inventing an attachment id", () => {
    const inventory = registration("machine-a", "alpha");
    inventory.projects[0].preferredMachineId = "machine-a";
    const history: SessionInfo = {
      id: "history-session",
      projectId: "registered-project",
      workingDir: "/work/q-index",
      status: "inactive",
      hostname: "session-host",
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
      hostname: "session-host",
      gitRemote: "github.com/example/q-index",
      panes: [{ pane_id: 42, is_working: true }],
    });
    expect(projects[0]).not.toHaveProperty("machinesHref");
  });
});

describe("repository grouping", () => {
  const remote = "github.com/example/chiral-network";

  it("files a checkout that never started under its repository, not \"(no remote)\"", () => {
    // The laptop checkout registered as its own project and has no session to
    // carry its remote; only the daemon inventory knows it.
    const laptop: MachineWithProjects = {
      machine: { machineId: "laptop", hostname: "laptop", os: "darwin", arch: "arm64" },
      clusterAccess: "owner",
      projects: [{ projectId: "laptop-project", path: "/Users/me/chiral-network", isRunning: false, gitRemote: remote }],
    };
    const zoo: SessionInfo = {
      id: "zoo-session",
      projectId: "zoo-project",
      workingDir: "/home/me/chiral-network",
      status: "active",
      isActive: true,
      hostname: "zoo-005",
      gitRemote: remote,
    };
    const groups = groupProjectsByRepo(buildProjectList([zoo], [], [laptop]));
    expect(groups.map((group) => [group.key, group.projects.map((project) => project.projectId)]))
      .toEqual([[remote, ["zoo-project", "laptop-project"]]]);
    expect(groups.some((group) => group.key === NO_REMOTE_KEY)).toBe(false);
  });

  it("takes the remote from any checkout that reports it", () => {
    const preferredOld = registration("machine-old", "zoo-002");
    preferredOld.projects[0].preferredMachineId = "machine-old";
    const updated = registration("machine-new", "zoo-003");
    updated.projects[0].preferredMachineId = "machine-old";
    updated.projects[0].gitRemote = remote;
    for (const machines of [[preferredOld, updated], [updated, preferredOld]]) {
      const [project] = buildProjectList([], [], machines);
      expect(project).toMatchObject({ hostname: "zoo-002", gitRemote: remote });
    }

    // A session from an older CLI that never sent its remote groups by its
    // checkout's.
    const session: SessionInfo = { id: "s", projectId: "registered-project", status: "inactive" };
    expect(buildProjectList([session], [], [updated])[0]).toMatchObject({ sessionId: "s", gitRemote: remote });
  });

  it("carries a session's attached instances for its host summary", () => {
    const instances = [
      { machineId: "zoo-5", hostname: "zoo-005", primary: true },
      { machineId: "zoo-6", hostname: "zoo-006", primary: false },
    ];
    const [project] = buildProjectList(
      [{ id: "s", projectId: "p", status: "active", isActive: true, hostname: "zoo-005", instances }],
      [],
      [],
    );
    expect(project.instances).toBe(instances);
  });
});

describe("a project's open state", () => {
  function session(id: string, overrides: Partial<SessionInfo> = {}): SessionInfo {
    return { id, projectId: id, workingDir: `/work/${id}`, status: "active", createdAt: "2026-11-16T00:00:00Z", ...overrides };
  }
  function host(machineId: string, hostname: string, projects: MachineWithProjects["projects"]): MachineWithProjects {
    return { machine: { machineId, hostname, os: "linux", arch: "x64" }, clusterAccess: "owner", projects };
  }
  const byId = (projects: ReturnType<typeof buildProjectList>) =>
    Object.fromEntries(projects.map((project) => [project.projectId, project]));

  it("words the recorded state as Open or Closed, whatever runs", () => {
    const projects = byId(buildProjectList([
      // Open, and its machine is still starting it: nothing runs yet.
      session("open-idle", { openState: "open", isActive: false }),
      // Closed, and a machine that could not be stopped still runs it.
      session("closed-running", { openState: "closed", isActive: true }),
    ], [], []));
    expect(projects["open-idle"]).toMatchObject({ openState: "open", isActive: false });
    expect(projectStateLabel(projects["open-idle"])).toBe("Open");
    // `isActive` stays what runs, so its tabs keep their real status.
    expect(projects["closed-running"]).toMatchObject({ openState: "closed", isActive: true });
    expect(projectStateLabel(projects["closed-running"])).toBe("Closed");
  });

  it("shows a project the server has not recorded as Open while it runs and Closed otherwise", () => {
    const projects = byId(buildProjectList(
      [session("running", { isActive: true }), session("stopped", { isActive: false }), session("elsewhere")],
      [],
      [host("m1", "zoo-005", [{ projectId: "elsewhere", path: "/work/elsewhere", isRunning: true }])],
    ));
    expect(projects.running).toMatchObject({ openState: "open" });
    expect(projects.stopped).toMatchObject({ openState: "closed" });
    // Any machine running it makes it open.
    expect(projects.elsewhere).toMatchObject({ openState: "open", isActive: true });
    expect(projectStateLabel(projects.stopped)).toBe("Closed");
  });

  it("takes the state of a registration-only project from the machine list", () => {
    const projects = byId(buildProjectList([], [], [
      host("m1", "zoo-005", [
        { projectId: "kept-open", path: "/work/kept-open", isRunning: false, openState: "open", part: "not_started" },
        { projectId: "never-ran", path: "/work/never-ran", isRunning: false },
        { projectId: "runs", path: "/work/runs", isRunning: true, part: "running" },
      ]),
      // Another machine's entry for the same project carries the same state.
      host("m2", "zoo-006", [{ projectId: "kept-open", path: "/work/kept-open", isRunning: false, openState: "open" }]),
    ]));
    expect(projects["kept-open"]).toMatchObject({ sessionId: null, openState: "open", isActive: false });
    expect(projects["never-ran"]).toMatchObject({ openState: "closed" });
    expect(projects.runs).toMatchObject({ openState: "open" });
  });

  it("lets a machine entry record the state of a session whose own entry lacks it", () => {
    const [project] = buildProjectList(
      [session("p", { isActive: true })],
      [],
      [host("m1", "zoo-005", [{ projectId: "p", path: "/work/p", isRunning: true, openState: "closed", part: "still_running" }])],
    );
    expect(project).toMatchObject({ openState: "closed", isActive: true });
  });

  it("never turns a shared machine's entry into a project row, with or without a state", () => {
    const shared: MachineWithProjects = {
      ...host("m1", "zoo-005", [
        { projectId: "theirs", path: "/work/theirs", isRunning: true },
        { projectId: "theirs-too", path: "/work/theirs-too", isRunning: false, openState: "open" },
      ]),
      clusterAccess: "member",
    };
    expect(buildProjectList([], [], [shared])).toEqual([]);
  });
});
