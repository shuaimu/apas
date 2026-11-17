import { describe, expect, it } from "vitest";
import type { MachineProject, MachineWithProjects } from "@/lib/store";
import { clusterProjectOpenModel, type ClusterOpenStateSource } from "./clusterOpenState";

const PROJECT = "30000000-0000-4000-8000-000000000001";

function machine(
  hostname: string,
  project: Partial<MachineProject> | null,
  overrides: Partial<MachineWithProjects> = {},
): MachineWithProjects {
  return {
    machine: { machineId: `id-${hostname}`, hostname, os: "linux", arch: "x64" },
    projects: project ? [{ projectId: PROJECT, path: "/work/mako", isRunning: false, ...project }] : [],
    clusterAccess: "owner",
    daemonConnected: true,
    ...overrides,
  };
}

function source(overrides: Partial<ClusterOpenStateSource> = {}): ClusterOpenStateSource {
  return { supported: true, machines: [], sessions: [], ...overrides };
}

const project = { id: PROJECT, name: "mako" };

describe("clusterProjectOpenModel", () => {
  it("knows nothing about a project that no machine and no session lists", () => {
    expect(clusterProjectOpenModel(project, source())).toBeNull();
    // A machine that lists other projects only says nothing about this one.
    expect(clusterProjectOpenModel(project, source({ machines: [machine("zoo-005", null)] }))).toBeNull();
  });

  it("reads the recorded state from a machine's entry", () => {
    const closed = clusterProjectOpenModel(project, source({
      machines: [machine("zoo-005", { openState: "closed", part: "not_started" })],
    }));
    expect(closed).toMatchObject({ state: "closed", stateLabel: "Closed" });
    // It names no machine for a request, so it offers neither action.
    expect(closed?.openAction).toBeUndefined();
    expect(closed?.closeAction).toBeUndefined();
  });

  it("shows a project the server has not recorded as Open while it runs and Closed otherwise", () => {
    expect(clusterProjectOpenModel(project, source({
      machines: [machine("zoo-005", { isRunning: true, part: "running" })],
    }))).toMatchObject({ state: "open", stateLabel: "Open" });
    expect(clusterProjectOpenModel(project, source({
      machines: [machine("zoo-005", {})],
    }))).toMatchObject({ state: "closed", stateLabel: "Closed" });
  });

  it("reads the state from the session list when no machine lists the project", () => {
    expect(clusterProjectOpenModel(project, source({
      sessions: [{ id: "session-1", projectId: PROJECT, isActive: false, openState: "open" }],
    }))).toMatchObject({ state: "open" });
    // A legacy session row is the project.
    expect(clusterProjectOpenModel(project, source({
      sessions: [{ id: PROJECT, isActive: true }],
    }))).toMatchObject({ state: "open" });
    expect(clusterProjectOpenModel(project, source({
      sessions: [{ id: "other", projectId: "another-project", isActive: true }],
    }))).toBeNull();
  });

  it("names the machines a deletion stops first and the offline tab owners, for the owner", () => {
    const model = clusterProjectOpenModel(project, source({
      machines: [machine("zoo-005", {
        isRunning: true,
        openState: "open",
        part: "running",
        otherHosts: [
          { hostname: "zoo-006", connected: true, running: true, ownsTabs: true },
          { hostname: "laptop", connected: false, running: false, ownsTabs: true },
        ],
      })],
    }), true);
    expect(model?.stoppedFirstHosts).toEqual(["zoo-005", "zoo-006"]);
    expect(model?.offlineTabOwnerHosts).toEqual(["laptop"]);
    expect(model?.canDelete).toBe(true);
    expect(clusterProjectOpenModel(project, source({ machines: [machine("zoo-005", {})] }))?.canDelete).toBe(false);
  });

  it("derives the state from what runs on a server that records none", () => {
    expect(clusterProjectOpenModel(project, source({
      supported: false,
      machines: [machine("zoo-005", { isRunning: true })],
    }))).toMatchObject({ state: "open" });
  });
});
