import { describe, expect, it } from "vitest";
import { resolveMachineProjectTarget } from "./machineProjectTarget";
import type { SessionInstance } from "./projectInstances";
import type { MachineProject, MachineWithProjects, SessionInfo } from "./store";

function machine(
  machineId: string,
  hostname: string,
  projectId: string,
  path: string,
  project: Partial<MachineProject> = {},
): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: [{ projectId, path, isRunning: false, ...project }],
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

function instance(machineId: string | undefined, hostname: string, primary = false): SessionInstance {
  return { machineId, hostname, primary };
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
      hostname: "builder-b",
    });
  });

  it("chooses deterministically instead of refusing when nothing else decides", () => {
    // The project view's Start and Close act on every machine, so a stable
    // answer is better than none: before instances this returned null and the
    // project view lost its boot target entirely.
    const machines = [
      machine("machine-b", "builder-b", "project-a", "/workspace/alpha"),
      machine("machine-a", "builder-a", "project-a", "/workspace/alpha"),
    ];
    const target = resolveMachineProjectTarget(session({ hostname: undefined }), machines);
    expect(target?.machineId).toBe("machine-a");
    expect(resolveMachineProjectTarget(session({ hostname: undefined }), [...machines].reverse()))
      .toEqual(target);
  });

  it("prefers the primary attached instance over the recorded and preferred hosts", () => {
    const machines = [
      machine("machine-a", "builder-a", "project-a", "/workspace/alpha", { preferredMachineId: "machine-a" }),
      machine("machine-b", "builder-b", "project-a", "/workspace/alpha", { preferredMachineId: "machine-a", isRunning: true }),
      machine("machine-c", "builder-c", "project-a", "/workspace/alpha", { preferredMachineId: "machine-a", isRunning: true }),
    ];
    const target = resolveMachineProjectTarget(
      session({
        hostname: "builder-a",
        instances: [instance("machine-b", "builder-b"), instance("machine-c", "builder-c", true)],
      }),
      machines,
    );
    expect(target).toMatchObject({ machineId: "machine-c", isRunning: true });
  });

  it("prefers live instances passed explicitly over the session's listed ones", () => {
    const machines = [
      machine("machine-a", "builder-a", "project-a", "/workspace/alpha"),
      machine("machine-b", "builder-b", "project-a", "/workspace/alpha"),
    ];
    const target = resolveMachineProjectTarget(
      session({ instances: [instance("machine-b", "builder-b", true)] }),
      machines,
      [instance("machine-a", "builder-a", true)],
    );
    // An attached instance is running even before the inventory says so.
    expect(target).toMatchObject({ machineId: "machine-a", isRunning: true });
  });

  it("matches a separate checkout by project id, whatever its path", () => {
    // A laptop checkout that joined the project lives at its own path. Path
    // equality with the primary's working directory could never find it.
    const target = resolveMachineProjectTarget(
      session({ workingDir: "/home/zoo/alpha", instances: [instance("laptop", "laptop", true)] }),
      [
        machine("zoo", "zoo-005", "project-a", "/home/zoo/alpha"),
        machine("laptop", "laptop", "project-a", "/Users/me/src/alpha"),
      ],
    );
    expect(target).toMatchObject({ machineId: "laptop", projectId: "project-a", hostname: "laptop" });
  });

  it("then prefers a running placement, then the preferred registration machine", () => {
    const stopped = [
      machine("machine-a", "builder-a", "project-a", "/workspace/alpha", { preferredMachineId: "machine-b" }),
      machine("machine-b", "builder-b", "project-a", "/workspace/alpha", { preferredMachineId: "machine-b" }),
    ];
    expect(resolveMachineProjectTarget(session({ hostname: "builder-a" }), stopped)?.machineId)
      .toBe("machine-b");

    const running = [
      stopped[0],
      machine("machine-b", "builder-b", "project-a", "/workspace/alpha", { preferredMachineId: "machine-b" }),
      machine("machine-c", "builder-c", "project-a", "/workspace/alpha", { isRunning: true }),
    ];
    expect(resolveMachineProjectTarget(session({ hostname: "builder-a" }), running))
      .toMatchObject({ machineId: "machine-c", isRunning: true });
  });

  it("follows a legacy instance that reported no machine by its hostname", () => {
    const target = resolveMachineProjectTarget(
      session({ hostname: "builder-a", instances: [instance(undefined, "builder-b", true)] }),
      [
        machine("machine-a", "builder-a", "project-a", "/workspace/alpha"),
        machine("machine-b", "builder-b", "project-a", "/workspace/alpha"),
      ],
    );
    expect(target).toMatchObject({ machineId: "machine-b", isRunning: true });
  });

  it("collapses a project a daemon repeats, keeping the running placement", () => {
    const repeated: MachineWithProjects = {
      machine: { machineId: "machine-a", hostname: "builder-a", os: "linux", arch: "x86_64" },
      projects: [
        { projectId: "project-a", path: "/workspace/alpha", isRunning: false },
        { projectId: "project-a", path: "/workspace/alpha", isRunning: true },
      ],
    };
    expect(resolveMachineProjectTarget(session({ hostname: undefined }), [repeated]))
      .toMatchObject({ machineId: "machine-a", isRunning: true });
  });

  it("falls back to the working directory only for rows the inventory knows by another id", () => {
    const legacyRow = session({ projectId: "unknown", id: "unknown-session", hostname: undefined });
    expect(resolveMachineProjectTarget(legacyRow, [
      machine("machine-a", "builder-a", "inventory-id", "/workspace/alpha/"),
    ])).toMatchObject({ machineId: "machine-a", projectId: "inventory-id" });

    // A path guess across machines is still refused.
    expect(resolveMachineProjectTarget(legacyRow, [
      machine("machine-a", "builder-a", "inventory-id", "/workspace/alpha"),
      machine("machine-b", "builder-b", "inventory-id", "/workspace/alpha"),
    ])).toBeNull();
    expect(resolveMachineProjectTarget({ ...legacyRow, hostname: "builder-b" }, [
      machine("machine-a", "builder-a", "inventory-id", "/workspace/alpha"),
      machine("machine-b", "builder-b", "inventory-id", "/workspace/alpha"),
    ])?.machineId).toBe("machine-b");
  });

  it("returns null without a session or a match", () => {
    expect(resolveMachineProjectTarget(undefined, [])).toBeNull();
    expect(resolveMachineProjectTarget(session(), [
      machine("machine-a", "builder-a", "other", "/elsewhere"),
    ])).toBeNull();
  });
});
