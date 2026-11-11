import { describe, expect, it } from "vitest";
import {
  buildMachineChoices,
  sessionCanCloneJoin,
  type MachineChoiceInput,
} from "./machineChoice";
import type { SessionInstance } from "./projectInstances";
import type { MachineWithProjects, PaneConfig } from "./store";

const PROJECT = "30000000-0000-4000-8000-000000000001";

function instance(
  machineId: string | undefined,
  hostname: string,
  overrides: Partial<SessionInstance> = {},
): SessionInstance {
  return { machineId, hostname, primary: false, ...overrides };
}

function machine(
  machineId: string,
  overrides: Partial<MachineWithProjects> & { registered?: boolean } = {},
): MachineWithProjects {
  const { registered = false, ...rest } = overrides;
  return {
    machine: { machineId, hostname: machineId, os: "linux", arch: "x86_64" },
    projects: registered ? [{ projectId: PROJECT, path: "/work/project", isRunning: false }] : [],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
    ...rest,
  };
}

function selected(machine_id: string | null, hostname: string | null): Pick<PaneConfig, "pane_id" | "machine_id" | "hostname"> {
  return { pane_id: 7, machine_id, hostname };
}

function model(overrides: Partial<MachineChoiceInput>) {
  return buildMachineChoices({
    projectId: PROJECT,
    instances: [],
    machines: [],
    instancesSupported: true,
    canClone: true,
    ...overrides,
  });
}

function summary(result: ReturnType<typeof buildMachineChoices>) {
  return result.choices.map((choice) =>
    [choice.hostname, choice.kind, choice.disabledReason ?? null]);
}

describe("buildMachineChoices", () => {
  it("offers nothing to choose from a server without instances", () => {
    const result = model({
      instancesSupported: false,
      instances: [instance("zoo-005", "zoo-005", { primary: true })],
      machines: [machine("zoo-005", { registered: true }), machine("zoo-006", { registered: true })],
    });
    expect(result).toEqual({ choices: [], showChoice: false });
  });

  it("leaves the menu unchanged with a single eligible machine", () => {
    const result = model({
      instances: [instance("zoo-005", "zoo-005", { primary: true })],
      machines: [machine("zoo-005", { registered: true })],
    });
    expect(result.showChoice).toBe(false);
    expect(result.defaultChoice).toMatchObject({ machineId: "zoo-005", kind: "instance", primary: true });
    expect(summary(result)).toEqual([["zoo-005", "instance", null]]);
  });

  it("orders attached instances, registered machines, clones, then disabled machines", () => {
    const result = model({
      instances: [
        instance("zoo-006", "zoo-006"),
        instance("zoo-005", "zoo-005", { primary: true }),
        instance("shared", "shared-host", { runtimeAccess: false }),
      ],
      machines: [
        machine("zoo-005", { registered: true }),
        machine("zoo-006", { registered: true }),
        machine("zoo-007", { registered: true }),
        machine("zoo-008", { registered: true, daemonConnected: false, projectInstancesAvailable: false }),
        machine("zoo-009", { registered: true, projectInstancesAvailable: false }),
        machine("laptop"),
        machine("desktop", { daemonConnected: false, projectInstancesAvailable: false }),
        machine("guest", { registered: true, clusterAccess: "member" }),
      ],
    });
    expect(summary(result)).toEqual([
      ["zoo-005", "instance", null],
      ["zoo-006", "instance", null],
      ["zoo-007", "registered", null],
      ["laptop", "clone", null],
      ["shared-host", "instance", "no runtime access"],
      ["zoo-008", "registered", "offline"],
      ["zoo-009", "registered", "Reboot zoo-009 to update"],
      ["desktop", "clone", "offline"],
    ]);
    expect(result.showChoice).toBe(true);
    expect(result.defaultChoice?.machineId).toBe("zoo-005");
  });

  it("defaults to the selected tab's machine, including a registered one to start", () => {
    const base = {
      instances: [
        instance("zoo-005", "zoo-005", { primary: true }),
        instance("zoo-006", "zoo-006"),
      ],
      machines: [
        machine("zoo-005", { registered: true }),
        machine("zoo-006", { registered: true }),
        machine("zoo-007", { registered: true }),
      ],
    };
    expect(model({ ...base, selectedPane: selected("zoo-006", "zoo-006") }).defaultChoice?.machineId)
      .toBe("zoo-006");
    // A tab of a stopped, registered machine: the new tab starts it there.
    expect(model({ ...base, selectedPane: selected("zoo-007", "zoo-007") }).defaultChoice)
      .toMatchObject({ machineId: "zoo-007", kind: "registered" });
    // An untagged tab follows its attributed host.
    expect(model({ ...base, selectedPane: selected(null, "zoo-007") }).defaultChoice?.machineId)
      .toBe("zoo-007");
    // A tab whose machine cannot take one falls back to the primary.
    expect(model({ ...base, selectedPane: selected("gone", "gone") }).defaultChoice?.machineId)
      .toBe("zoo-005");
  });

  it("never defaults to a clone, and asks when a clone is the only option", () => {
    const cloneOnly = model({
      machines: [machine("zoo-005", { registered: true, daemonConnected: false }), machine("laptop")],
    });
    expect(cloneOnly.defaultChoice).toBeUndefined();
    expect(cloneOnly.showChoice).toBe(true);
    expect(summary(cloneOnly)).toEqual([
      ["laptop", "clone", null],
      ["zoo-005", "registered", "offline"],
    ]);

    // Without a repository to clone, machines lacking the project are not
    // listed at all.
    expect(summary(model({ canClone: false, machines: [machine("laptop")] }))).toEqual([]);
  });

  it("starts a stopped project on its only registered machine without asking", () => {
    const result = model({ machines: [machine("zoo-005", { registered: true })], canClone: false });
    expect(result.showChoice).toBe(false);
    expect(result.defaultChoice).toMatchObject({ machineId: "zoo-005", kind: "registered" });
  });

  it("uses the fallback machine when neither a tab nor a primary decides", () => {
    const result = model({
      machines: [machine("zoo-005", { registered: true }), machine("zoo-006", { registered: true })],
      fallbackMachineId: "zoo-006",
      canClone: false,
    });
    expect(result.showChoice).toBe(true);
    expect(result.defaultChoice?.machineId).toBe("zoo-006");
  });

  it("blocks every other machine while an older CLI owns the project", () => {
    const result = model({
      instances: [instance(undefined, "laptop", { primary: true })],
      machines: [machine("laptop-id", { registered: true }), machine("zoo-005", { registered: true }), machine("zoo-006")],
      selectedPane: selected(null, "laptop"),
    });
    expect(summary(result)).toEqual([
      ["laptop", "instance", null],
      ["laptop-id", "registered", "Reboot laptop to update"],
      ["zoo-005", "registered", "Reboot laptop to update"],
      ["zoo-006", "clone", "Reboot laptop to update"],
    ]);
    expect(result.showChoice).toBe(false);
    // The legacy instance is reached by naming no machine.
    expect(result.defaultChoice).toMatchObject({ kind: "instance", machineId: undefined, hostname: "laptop" });
  });

  it("treats an attached instance on an older daemon as owning the project too", () => {
    const result = model({
      instances: [instance("zoo-005", "zoo-005", { primary: true })],
      machines: [
        machine("zoo-005", { registered: true, projectInstancesAvailable: false }),
        machine("zoo-006", { registered: true }),
      ],
    });
    expect(summary(result)).toEqual([
      ["zoo-005", "instance", null],
      ["zoo-006", "registered", "Reboot zoo-005 to update"],
    ]);
    expect(result.showChoice).toBe(false);
  });

  it("keeps an attached instance on a shared machine but offers no other shared machine", () => {
    const result = model({
      instances: [instance("guest", "guest", { primary: true })],
      machines: [machine("guest", { registered: true, clusterAccess: "member" }), machine("other", { registered: true, clusterAccess: "member" })],
    });
    expect(summary(result)).toEqual([["guest", "instance", null]]);
  });
});

describe("sessionCanCloneJoin", () => {
  it("needs a repository and the owner path", () => {
    expect(sessionCanCloneJoin({ gitRemote: "github.com/acme/repo" })).toBe(true);
    expect(sessionCanCloneJoin({ gitRemote: "github.com/acme/repo", isShared: true })).toBe(false);
    expect(sessionCanCloneJoin({})).toBe(false);
    expect(sessionCanCloneJoin(undefined)).toBe(false);
  });
});
