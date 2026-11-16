import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { MachineWithProjects, MobileBootstrapResponse, MobileSessionSummary, ServerToWeb, SessionInstanceInfo } from "@apas/protocol";

import { sessionActivityStatus } from "@/components/SessionCard";
import { handleSessionInstancesMessage } from "@/connection/sessionInstances";
import {
  instancesForSession,
  knownMachines,
  paneHostname,
  paneMachineConnected,
  paneReachability,
  paneUnavailableReason,
  rosterSpansHosts,
  sessionHostSummary,
  tabUnavailableReason,
  type KnownMachine,
} from "./projectInstances";
import { useMobileStore } from "./store";

const ZOO_005 = "50000000-0000-4000-8000-000000000001";
const ZOO_006 = "50000000-0000-4000-8000-000000000002";
const LAPTOP = "50000000-0000-4000-8000-000000000003";

const zoo005: SessionInstanceInfo = { machine_id: ZOO_005, hostname: "zoo-005", working_dir: "/workspace/project", primary: true, runtime_access: true };
const zoo006: SessionInstanceInfo = { machine_id: ZOO_006, hostname: "zoo-006", working_dir: "/workspace/project", primary: false, runtime_access: true };
const laptop: SessionInstanceInfo = { machine_id: LAPTOP, hostname: "laptop", working_dir: "/home/me/project", primary: false };
// An attached CLI that reported no machine lacks the capability, so it is the
// session's only instance and owns the whole roster.
const legacySole: SessionInstanceInfo = { hostname: "laptop", working_dir: "/workspace/project", primary: true };

const session: MobileSessionSummary = {
  id: "20000000-0000-4000-8000-000000000001",
  project_id: "30000000-0000-4000-8000-000000000001",
  project_name: "project",
  hostname: "zoo-005",
  working_dir: "/workspace/project",
  status: "active",
  is_active: true,
};

function fixture(name: string): ServerToWeb {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8")) as ServerToWeb;
}

// The machines as a bootstrap lists them: zoo-006's daemon is connected and
// the laptop's is not.
const zoo006Connected: KnownMachine = { machine_id: ZOO_006, hostname: "zoo-006", connected: true };
const laptopOffline: KnownMachine = { machine_id: LAPTOP, hostname: "laptop", connected: false };

function bootstrapWith(sessions: MobileSessionSummary[], machines: MachineWithProjects[] = []): MobileBootstrapResponse {
  return {
    user_id: "1027ac43-3c54-467a-90e6-e50a170d4882",
    user_email: "mobile@example.test",
    cluster_role: "user",
    account_status: "active",
    protocol_min_version: 1,
    protocol_max_version: 1,
    features: {},
    sessions,
    machines,
    launch_targets: [],
  };
}

describe("tab hosts", () => {
  it("shows the host the server stamped, including an untagged pane's attributed host", () => {
    expect(paneHostname({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005])).toBe("zoo-006");
    expect(paneHostname({ hostname: "zoo-005" }, [zoo006])).toBe("zoo-005");
  });

  it("names a tagged pane from its own attached instance, never an unattributed pane from the primary", () => {
    expect(paneHostname({ machine_id: ZOO_006 }, [zoo005, zoo006])).toBe("zoo-006");
    expect(paneHostname({ machine_id: ZOO_006 }, [zoo005])).toBeNull();
    expect(paneHostname({}, [zoo005, zoo006])).toBeNull();
    expect(paneHostname({ hostname: "  " }, [zoo005])).toBeNull();
  });

  it("badges tabs only when they run on more than one named host", () => {
    expect(rosterSpansHosts(["zoo-005", "zoo-005", null])).toBe(false);
    expect(rosterSpansHosts([null, null])).toBe(false);
    expect(rosterSpansHosts(["zoo-005", null, "zoo-006"])).toBe(true);
  });
});

describe("project host line", () => {
  it("names the primary plus the other running instances", () => {
    expect(sessionHostSummary(session, [zoo006, { ...zoo005 }])).toBe("zoo-005 +1");
    expect(sessionHostSummary(session, [zoo005, zoo006, laptop])).toBe("zoo-005 +2");
  });

  it("keeps the session's own host when one instance or none is attached", () => {
    expect(sessionHostSummary(session, [zoo005])).toBe("zoo-005");
    expect(sessionHostSummary({ ...session, is_active: false }, [zoo005, zoo006])).toBe("zoo-005");
    expect(sessionHostSummary({ ...session, hostname: null }, [])).toBe("/workspace/project");
    expect(sessionHostSummary({ ...session, hostname: null, working_dir: null }, [])).toBeNull();
  });

  it("prefers the live set to the bootstrap's, including a live empty set", () => {
    const withBootstrap = { ...session, instances: [zoo005, zoo006] };
    expect(instancesForSession(withBootstrap, undefined)).toEqual([zoo005, zoo006]);
    expect(instancesForSession(withBootstrap, [zoo005])).toEqual([zoo005]);
    expect(instancesForSession(withBootstrap, [])).toEqual([]);
    expect(instancesForSession(session, undefined)).toEqual([]);
    expect(instancesForSession(undefined, undefined)).toEqual([]);
  });
});

describe("tab reachability", () => {
  it("routes a tagged tab to its own machine only", () => {
    expect(paneReachability({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005, zoo006], true)).toBe("usable");
    expect(paneReachability({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005], true)).toBe("stopped");
  });

  it("refuses a tab whose machine this viewer cannot use", () => {
    expect(paneReachability({ machine_id: ZOO_006 }, [zoo005, { ...zoo006, runtime_access: false }], true)).toBe("no_access");
    // Not evaluated means usable: the server decides.
    expect(paneReachability({ machine_id: ZOO_006 }, [zoo005, { ...zoo006, runtime_access: null }], true)).toBe("usable");
  });

  it("follows the untagged-pane routing rule", () => {
    // The instance on the attributed host takes it.
    expect(paneReachability({ hostname: "zoo-006" }, [zoo005, zoo006], true)).toBe("usable");
    // Side-by-side instances are all capable and never take another host's tab.
    expect(paneReachability({ hostname: "zoo-006" }, [zoo005, laptop], true)).toBe("stopped");
    // A sole instance without a machine id lacks the capability and owns everything.
    expect(paneReachability({ hostname: "zoo-005" }, [legacySole], true)).toBe("usable");
    // A sole instance with one may or may not have it, so nothing is inferred.
    expect(paneReachability({ hostname: "zoo-005" }, [{ ...laptop, primary: true }], true)).toBe("usable");
    // No attributed host: the primary takes it, with the primary's access.
    expect(paneReachability({}, [zoo005, zoo006], true)).toBe("usable");
    expect(paneReachability({}, [{ ...zoo005, runtime_access: false }, zoo006], true)).toBe("no_access");
  });

  it("infers nothing without a live instance set", () => {
    expect(paneReachability({ machine_id: ZOO_006 }, [zoo005], false)).toBe("usable");
    expect(paneReachability({ machine_id: ZOO_006 }, [], true)).toBe("usable");
  });

  it("says why a tab cannot take input, by whether its machine's daemon is connected", () => {
    expect(paneUnavailableReason("stopped", "zoo-006", true)).toBe("zoo-006 is not running this project. Open the project again to start it there.");
    expect(paneUnavailableReason("stopped", "zoo-006", false)).toBe("zoo-006 is not running this project because it is offline.");
    expect(paneUnavailableReason("no_access", "zoo-006", true)).toBe("Your access does not cover zoo-006, so this tab is read-only here.");
    expect(paneUnavailableReason("stopped", null, true)).toBe("This tab's machine is not running this project.");
    expect(paneUnavailableReason("stopped", null, false)).toBe("This tab's machine is not running this project.");
    expect(paneUnavailableReason("usable", "zoo-006", false)).toBeNull();
    // The sentence this app composed before the open state names a control
    // no client offers any more.
    for (const connected of [true, false]) {
      expect(paneUnavailableReason("stopped", "zoo-006", connected)).not.toMatch(/Start it on/);
    }
  });

  it("describes a stopped machine in the words the server begins its own refusal with", () => {
    // The golden refusal of typed input for a tab on a stopped machine. Its
    // sentence and both of this app's start with the same statement.
    const refusal = fixture("server-mutation-ack-input.json");
    expect(refusal.type).toBe("mutation_ack");
    const error = refusal.type === "mutation_ack" ? refusal.error ?? "" : "";
    const statement = "zoo-005 is not running this project";
    expect(error.startsWith(statement)).toBe(true);
    expect(paneUnavailableReason("stopped", "zoo-005", true)?.startsWith(`${statement}. `)).toBe(true);
    expect(paneUnavailableReason("stopped", "zoo-005", false)?.startsWith(`${statement} `)).toBe(true);
  });

  it("takes a machine's daemon for connected only where the bootstrap lists it so", () => {
    const [listed] = (fixture("server-machines-open-state.json") as Extract<ServerToWeb, { type: "machines" }>).machines;
    expect(knownMachines([listed])).toEqual([zoo006Connected]);
    // An older server sends no `daemon_connected`, and a bootstrap may list none.
    expect(knownMachines([{ ...listed, daemon_connected: undefined }])).toEqual([{ ...zoo006Connected, connected: false }]);
    expect(knownMachines(undefined)).toEqual([]);

    const machines = [zoo006Connected, laptopOffline];
    expect(paneMachineConnected({ machine_id: ZOO_006, hostname: "zoo-006" }, "zoo-006", machines)).toBe(true);
    expect(paneMachineConnected({ machine_id: LAPTOP, hostname: "laptop" }, "laptop", machines)).toBe(false);
    // A machine the list does not carry is not taken for connected.
    expect(paneMachineConnected({ machine_id: ZOO_005, hostname: "zoo-005" }, "zoo-005", machines)).toBe(false);
    // A tagged tab is matched by its machine, never by a host of the same name.
    expect(paneMachineConnected({ machine_id: ZOO_005, hostname: "zoo-006" }, "zoo-006", machines)).toBe(false);
    // An untagged tab has only the host it is shown under.
    expect(paneMachineConnected({ hostname: "zoo-006" }, "zoo-006", machines)).toBe(true);
    expect(paneMachineConnected({ hostname: "laptop" }, "laptop", machines)).toBe(false);
    expect(paneMachineConnected({}, null, machines)).toBe(false);
  });

  it("judges one tab for screens that show only that tab", () => {
    const machines = [zoo006Connected, laptopOffline];
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005], true, machines))
      .toBe("zoo-006 is not running this project. Open the project again to start it there.");
    expect(tabUnavailableReason({ machine_id: LAPTOP, hostname: "laptop" }, [zoo005], true, machines))
      .toBe("laptop is not running this project because it is offline.");
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005], true, []))
      .toBe("zoo-006 is not running this project because it is offline.");
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005, { ...zoo006, runtime_access: false }], true, machines))
      .toBe("Your access does not cover zoo-006, so this tab is read-only here.");
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005, zoo006], true, machines)).toBeNull();
    // Without a live set, or for a tab the roster does not list, the server decides.
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005], false, machines)).toBeNull();
    expect(tabUnavailableReason(undefined, [zoo005], true, machines)).toBeNull();
  });
});

describe("live instance sets", () => {
  beforeEach(() => useMobileStore.getState().reset());

  it("keeps the server's session_instances frame for a known session", () => {
    useMobileStore.setState({ sessions: [session] });
    const frame = fixture("server-session-instances.json");

    expect(handleSessionInstancesMessage(frame)).toBe(true);
    expect(useMobileStore.getState().instancesBySession[session.id]?.map((item) => item.hostname)).toEqual(["zoo-005", "zoo-006"]);
    expect(useMobileStore.getState().instancesBySession[session.id]?.[1]?.runtime_access).toBe(false);

    expect(handleSessionInstancesMessage(fixture("server-session-instances-legacy.json"))).toBe(true);
    expect(useMobileStore.getState().instancesBySession[session.id]).toEqual([legacySole]);
  });

  it("ignores frames for sessions this account does not list and leaves other frames alone", () => {
    expect(handleSessionInstancesMessage(fixture("server-session-instances.json"))).toBe(true);
    expect(useMobileStore.getState().instancesBySession).toEqual({});

    useMobileStore.setState({ sessions: [session] });
    expect(handleSessionInstancesMessage(fixture("server-pane-list-instances.json"))).toBe(false);
    expect(useMobileStore.getState().instancesBySession).toEqual({});
  });

  it("lets a new bootstrap's own instances replace sets pushed before it", () => {
    useMobileStore.setState({ sessions: [session] });
    useMobileStore.getState().setSessionInstances(session.id, [zoo005, zoo006]);
    useMobileStore.getState().applyBootstrap(bootstrapWith([{ ...session, instances: [zoo005] }]));

    const state = useMobileStore.getState();
    expect(state.instancesBySession).toEqual({});
    expect(instancesForSession(state.sessions[0], state.instancesBySession[session.id])).toEqual([zoo005]);
  });
});

describe("a project's open state on the entries this app already reads", () => {
  beforeEach(() => useMobileStore.getState().reset());

  // The bootstrap's session entries flatten the same session entry a live
  // `sessions` frame carries, so the fixture's entries stand for both.
  const listed = (fixture("server-sessions-open-state.json") as Extract<ServerToWeb, { type: "sessions" }>).sessions;
  const machines = (fixture("server-machines-open-state.json") as Extract<ServerToWeb, { type: "machines" }>).machines;

  it("keeps bootstrap sessions that carry it, and reads nothing from it", () => {
    expect(listed.map((entry) => entry.open_state)).toEqual(["open", "closed", undefined]);
    const summaries: MobileSessionSummary[] = listed.map((entry) => ({ ...entry, project_name: "project", attention_count: 0 }));
    useMobileStore.getState().applyBootstrap(bootstrapWith(summaries, machines));

    const state = useMobileStore.getState();
    expect(state.error).toBeNull();
    expect(state.sessions).toEqual(summaries);
    const shown = (sessions: MobileSessionSummary[]) => sessions.map((entry) => [
      sessionActivityStatus(entry),
      sessionHostSummary(entry, instancesForSession(entry, undefined)),
    ]);
    expect(shown(state.sessions)).toEqual([["idle", "zoo-005"], ["offline", "zoo-006"], ["idle", "zoo-005"]]);

    // What this app shows comes from what runs, as before. The state can
    // differ from it: an open project that runs nowhere yet, a closed one a
    // machine still runs. Neither changes a row, nor does leaving it out.
    const [open, closed, notRecorded] = summaries;
    const contrary: MobileSessionSummary[] = [
      { ...open, open_state: "closed" },
      { ...closed, open_state: "open" },
      { ...notRecorded, open_state: "closed" },
    ];
    const without = summaries.map((entry) => {
      const copy = { ...entry };
      delete copy.open_state;
      return copy;
    });
    for (const variant of [contrary, without]) {
      useMobileStore.getState().applyBootstrap(bootstrapWith(variant, machines));
      expect(shown(useMobileStore.getState().sessions)).toEqual(shown(summaries));
    }
  });

  it("keeps only each machine's connection from a machine list whose projects carry the new fields", () => {
    const [open, closed, notRecorded] = machines[0].projects;
    expect([open.open_state, open.part, open.part_reason, open.other_hosts?.length]).toEqual([
      "open", "not_started", "zoo-006 could not start the project: the checkout is missing.", 2,
    ]);
    expect([closed.open_state, closed.part]).toEqual(["closed", undefined]);
    expect([notRecorded.open_state, notRecorded.part]).toEqual([undefined, "running"]);

    useMobileStore.getState().applyBootstrap(bootstrapWith([session], machines));
    expect(useMobileStore.getState().knownMachines).toEqual([zoo006Connected]);
    // The entry says zoo-006 failed to start an open project. This app shows
    // no part of a machine; it words the stopped tab from the connection.
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005], true, useMobileStore.getState().knownMachines))
      .toBe("zoo-006 is not running this project. Open the project again to start it there.");

    // Signing out forgets them with everything else.
    useMobileStore.getState().reset();
    expect(useMobileStore.getState().knownMachines).toEqual([]);
  });
});
