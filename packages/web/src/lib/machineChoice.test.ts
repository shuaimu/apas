import { describe, expect, it } from "vitest";
import {
  buildMachineChoices,
  installedProvidersSummary,
  parseMachineProviders,
  providerOfferOn,
  sessionCanCloneJoin,
  type MachineChoiceInput,
} from "./machineChoice";
import type { SessionInstance } from "./projectInstances";
import type { MachineProviderStatus, MachineWithProjects, PaneConfig } from "./store";

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

  // Before, a stopped project's only registered machine was a hidden default:
  // `+` started the project, relaunching its saved tabs and bots, with nothing
  // in the menu saying so. The step is now shown, preselected, so the start is
  // disclosed before the click.
  it("shows the step when the only eligible machine would start the project", () => {
    const result = model({ machines: [machine("zoo-005", { registered: true })], canClone: false });
    expect(result.showChoice).toBe(true);
    expect(result.defaultChoice).toMatchObject({ machineId: "zoo-005", kind: "registered" });
    expect(summary(result)).toEqual([["zoo-005", "registered", null]]);

    // The same when another instance runs the project but this viewer may
    // not use it: the viewer's own machine would be started.
    const elsewhere = model({
      instances: [instance("zoo-006", "zoo-006", { primary: true, runtimeAccess: false })],
      machines: [machine("zoo-005", { registered: true })],
      canClone: false,
    });
    expect(elsewhere.showChoice).toBe(true);
    expect(elsewhere.defaultChoice).toMatchObject({ machineId: "zoo-005", kind: "registered" });
  });

  it("says which other tab owners start when a tab opens the project", () => {
    const input = {
      machines: [machine("zoo-005", { registered: true }), machine("zoo-006", { registered: true })],
      projectMachines: [
        { machineId: "zoo-005", hostname: "zoo-005", part: "not_started", ownsTabs: true, connected: true },
        { machineId: "zoo-006", hostname: "zoo-006", part: "not_started", ownsTabs: true, connected: true },
        // Offline, running, or with no tabs: an Open starts none of these now.
        { machineId: "laptop", hostname: "laptop", part: "not_started", ownsTabs: true, connected: false },
        { machineId: "mini", hostname: "mini", part: "running", ownsTabs: true, connected: true },
        { machineId: "spare", hostname: "spare", part: "not_started", ownsTabs: false, connected: true },
      ],
    };
    const result = model({ ...input, openStateSupported: true });
    expect(result.choices.map((choice) => [choice.hostname, choice.kind, choice.alsoStarts]))
      .toEqual([["zoo-005", "registered", ["zoo-006"]], ["zoo-006", "registered", ["zoo-005"]]]);
    // The step and its default are what they were.
    expect(result.showChoice).toBe(true);
    expect(result.defaultChoice?.machineId).toBe("zoo-005");

    // A server that records no open state starts the chosen machine alone.
    const older = model(input);
    expect(older.choices.every((choice) => choice.alsoStarts === undefined)).toBe(true);
  });

  // "Clone here" ends in a tab request on a machine that is not running the
  // project, and the server opens the project for that like for any other:
  // every connected tab owner that is not running it starts. The choice
  // carries those hosts so the step and the confirmation can name them.
  it("says which tab owners start when a tab on a cloned machine opens the project", () => {
    // What `buildProjectOpenModel(...).machines` holds for a closed project,
    // and equally for one nobody has recorded that runs nowhere: tab owners
    // that are not running it. zoo-006 is another account's machine, which
    // this viewer does not list.
    const idle = [
      { machineId: "zoo-005", hostname: "zoo-005", part: "not_started", ownsTabs: true, connected: true },
      { hostname: "zoo-006", part: "not_started", ownsTabs: true, connected: true },
      { machineId: "mini", hostname: "mini", part: "not_started", ownsTabs: true, connected: false },
      { machineId: "spare", hostname: "spare", part: "not_started", ownsTabs: false, connected: true },
    ];
    const result = model({
      openStateSupported: true,
      machines: [machine("laptop"), machine("zoo-005", { registered: true })],
      projectMachines: idle,
    });
    expect(result.choices.map((choice) => [choice.hostname, choice.kind, choice.alsoStarts])).toEqual([
      ["zoo-005", "registered", ["zoo-006"]],
      ["laptop", "clone", ["zoo-005", "zoo-006"]],
    ]);
    // A clone is still never preselected.
    expect(result.defaultChoice?.machineId).toBe("zoo-005");

    // An open project that runs on its only tab owner: the tab still opens
    // the project, and nothing else starts.
    const running = model({
      openStateSupported: true,
      instances: [instance("zoo-005", "zoo-005", { primary: true })],
      machines: [machine("laptop"), machine("zoo-005", { registered: true })],
      projectMachines: [
        { machineId: "zoo-005", hostname: "zoo-005", part: "running", ownsTabs: true, connected: true },
      ],
    });
    expect(running.choices.find((choice) => choice.kind === "clone")?.alsoStarts).toEqual([]);
    // No parts given: the clone is known to open the project, with nobody named.
    const unnamed = model({ openStateSupported: true, machines: [machine("zoo-007")] });
    expect(unnamed.choices.map((choice) => [choice.kind, choice.alsoStarts])).toEqual([["clone", []]]);

    // A server that records no open state starts the cloned machine alone.
    const older = model({ machines: [machine("laptop")], projectMachines: idle });
    expect(older.choices.map((choice) => [choice.kind, choice.alsoStarts])).toEqual([["clone", undefined]]);
  });

  it("does not name a machine whose part the server did not state among those a tab starts", () => {
    // From another entry's `other_hosts` alone, a connected tab owner of an
    // open project that is not running it may be starting already.
    const result = model({
      openStateSupported: true,
      machines: [machine("laptop"), machine("zoo-005", { registered: true })],
      projectMachines: [
        { machineId: "zoo-005", hostname: "zoo-005", part: "not_started", ownsTabs: true, connected: true },
        { hostname: "zoo-006", part: "not_started", ownsTabs: true, connected: true, partUnconfirmed: true },
      ],
    });
    expect(result.choices.map((choice) => [choice.hostname, choice.alsoStarts])).toEqual([
      ["zoo-005", []],
      ["laptop", ["zoo-005"]],
    ]);
  });

  it("does not name a machine the server will not start among those a tab starts", () => {
    // zoo-006's daemon cannot run the project beside other machines
    // (`ProjectOpenMachine.cannotStart`): an Open sends it nothing.
    const result = model({
      openStateSupported: true,
      machines: [machine("laptop", { registered: true }), machine("zoo-005", { registered: true })],
      projectMachines: [
        { machineId: "zoo-005", hostname: "zoo-005", part: "not_started", ownsTabs: true, connected: true },
        { machineId: "zoo-006", hostname: "zoo-006", part: "not_started", ownsTabs: true, connected: true, cannotStart: "zoo-006 runs an older APAS." },
      ],
    });
    expect(result.choices.map((choice) => [choice.hostname, choice.alsoStarts])).toEqual([
      ["laptop", ["zoo-005"]],
      ["zoo-005", []],
    ]);
  });

  it("marks a choice that starts a machine of a project that is open already", () => {
    const input = {
      openStateSupported: true,
      instances: [instance("zoo-005", "zoo-005", { primary: true })],
      machines: [machine("zoo-005", { registered: true }), machine("zoo-006", { registered: true }), machine("laptop")],
    };
    const open = model({ ...input, projectOpen: true });
    expect(open.choices.map((choice) => [choice.hostname, choice.kind, choice.projectOpen])).toEqual([
      ["zoo-005", "instance", undefined],
      ["zoo-006", "registered", true],
      ["laptop", "clone", true],
    ]);
    // Closed, not recorded, or a server that records nothing: not marked.
    expect(model(input).choices.every((choice) => choice.projectOpen === undefined)).toBe(true);
    expect(model({ ...input, openStateSupported: false, projectOpen: true }).choices
      .every((choice) => choice.projectOpen === undefined)).toBe(true);
  });

  it("keeps the step hidden when the only eligible machine already runs the project", () => {
    // Also with a registered machine that cannot take the tab: only eligible
    // choices count.
    const result = model({
      instances: [instance("zoo-005", "zoo-005", { primary: true })],
      machines: [
        machine("zoo-005", { registered: true }),
        machine("zoo-006", { registered: true, daemonConnected: false, projectInstancesAvailable: false }),
      ],
      canClone: false,
    });
    expect(result.showChoice).toBe(false);
    expect(result.defaultChoice).toMatchObject({ machineId: "zoo-005", kind: "instance" });
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

describe("provider availability on the machine that receives a tab", () => {
  const MAC = "Shuais-MacBook-Air-2.local";
  const claude: MachineProviderStatus = { provider: "claude", installed: true, version: "2.1.233", canInstall: false };
  const ompInstallable: MachineProviderStatus = { provider: "omp", installed: false, canInstall: true };
  const ompMissing: MachineProviderStatus = { provider: "omp", installed: false, canInstall: false };
  const codexMissing: MachineProviderStatus = { provider: "codex", installed: false, canInstall: false };

  it("carries each machine's report onto its choice, for instances and registered machines", () => {
    const providers = [claude, ompInstallable];
    const result = model({
      instances: [instance("mac", MAC, { primary: true }), instance(undefined, "zoo-005")],
      machines: [
        { ...machine("mac"), machine: { machineId: "mac", hostname: MAC, os: "macos", arch: "aarch64", providers } },
        // A sole instance that reported no machine still finds its machine
        // by host.
        { ...machine("zoo-005"), machine: { machineId: "zoo-005", hostname: "zoo-005", os: "linux", arch: "x86_64", providers: [claude] } },
        machine("zoo-006", { registered: true }),
      ],
    });
    const byHost = new Map(result.choices.map((choice) => [choice.hostname, choice.providers]));
    expect(byHost.get(MAC)).toEqual(providers);
    expect(byHost.get("zoo-005")).toEqual([claude]);
    // An older daemon reports nothing: unknown, not "none installed".
    expect(byHost.get("zoo-006")).toBeUndefined();
  });

  it("changes nothing for an installed provider", () => {
    expect(providerOfferOn({ hostname: MAC, providers: [claude] }, "claude")).toEqual({});
  });

  it("offers a provider the machine can install, with a note", () => {
    const offer = providerOfferOn({ hostname: MAC, providers: [claude, ompInstallable] }, "omp");
    expect(offer.disabledReason).toBeUndefined();
    expect(offer.note).toBe("Installs OMP first");
    expect(offer.noteTitle).toBe(`The tab installs OMP on ${MAC}, then starts it. You may need to log in to OMP in its terminal.`);
  });

  it("disables a provider the machine has not got and cannot install, and says only that", () => {
    expect(providerOfferOn({ hostname: MAC, providers: [ompMissing] }, "omp"))
      .toEqual({ disabledReason: `OMP is not installed on ${MAC}.` });
    expect(providerOfferOn({ hostname: "zoo-005", providers: [codexMissing] }, "codex"))
      .toEqual({ disabledReason: "Codex is not installed on zoo-005." });
  });

  it("reads DeepSeek as the Claude binary it runs", () => {
    const missingClaude: MachineProviderStatus = { provider: "claude", installed: false, canInstall: false };
    expect(providerOfferOn({ hostname: MAC, providers: [missingClaude] }, "deepseek"))
      .toEqual({ disabledReason: `Claude is not installed on ${MAC}.` });
  });

  it("changes nothing when the machine reports nothing, an empty list, or not this provider", () => {
    expect(providerOfferOn(undefined, "omp")).toEqual({});
    expect(providerOfferOn({ hostname: MAC }, "omp")).toEqual({});
    expect(providerOfferOn({ hostname: MAC, providers: [] }, "omp")).toEqual({});
    expect(providerOfferOn({ hostname: MAC, providers: [claude] }, "omp")).toEqual({});
  });

  it("reads the wire's list leniently", () => {
    expect(parseMachineProviders(undefined)).toBeUndefined();
    expect(parseMachineProviders(null)).toBeUndefined();
    expect(parseMachineProviders([])).toEqual([]);
    expect(parseMachineProviders([
      { provider: "claude", installed: true, version: "2.1.233", can_install: false },
      { provider: "omp", installed: false, version: null, can_install: true },
      { provider: "codex", installed: true },
      { installed: true, can_install: false },
      "omp",
    ])).toEqual([
      { provider: "claude", installed: true, version: "2.1.233", canInstall: false },
      { provider: "omp", installed: false, version: undefined, canInstall: true },
    ]);
  });

  it("names a machine's installed providers in the menu's order, with versions in the title", () => {
    expect(installedProvidersSummary(undefined)).toBeNull();
    expect(installedProvidersSummary([])).toEqual({ text: "No providers installed", title: "No providers installed" });
    expect(installedProvidersSummary([
      { provider: "omp", installed: true, version: "18.20.0", canInstall: true },
      ompMissing,
      { provider: "codex", installed: true, canInstall: false },
      claude,
    ])).toEqual({
      text: "Providers: Claude, Codex, OMP",
      title: "Claude 2.1.233, Codex, OMP 18.20.0",
    });
  });
});
