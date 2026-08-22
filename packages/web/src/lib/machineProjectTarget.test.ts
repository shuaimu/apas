import { describe, expect, it } from "vitest";
import { resolveMachineProjectTarget } from "./machineProjectTarget";
import type { MachineWithProjects, SessionInfo } from "./store";

function machine(
  machineId: string,
  hostname: string,
  projectId: string,
  path: string,
): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: [{ projectId, path, isRunning: false }],
  };
}

function session(overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id: "session-a",
    projectId: "project-a",
    workingDir: "/workspace/alpha",
    hostname: "builder-b",
    status: "offline",
    ...overrides,
  };
}

describe("resolveMachineProjectTarget", () => {
  it("selects the session host when a project is registered on several machines", () => {
    const target = resolveMachineProjectTarget(session(), [
      machine("machine-a", "builder-a", "project-a", "/workspace/alpha"),
      machine("machine-b", "builder-b", "project-a", "/workspace/alpha/"),
    ]);

    expect(target).toEqual({
      machineId: "machine-b",
      projectId: "project-a",
      isRunning: false,
    });
  });

  it("does not guess between machines when a session has no matching host", () => {
    const target = resolveMachineProjectTarget(session({ hostname: undefined }), [
      machine("machine-a", "builder-a", "project-a", "/workspace/alpha"),
      machine("machine-b", "builder-b", "project-a", "/workspace/alpha"),
    ]);

    expect(target).toBeNull();
  });
});
