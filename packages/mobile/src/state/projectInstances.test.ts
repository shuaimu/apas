import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { MobileBootstrapResponse, MobileSessionSummary, ServerToWeb, SessionInstanceInfo } from "@apas/protocol";

import { handleSessionInstancesMessage } from "@/connection/sessionInstances";
import {
  instancesForSession,
  paneHostname,
  paneReachability,
  paneUnavailableReason,
  rosterSpansHosts,
  sessionHostSummary,
  tabUnavailableReason,
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

function bootstrapWith(sessions: MobileSessionSummary[]): MobileBootstrapResponse {
  return {
    user_id: "1027ac43-3c54-467a-90e6-e50a170d4882",
    user_email: "mobile@example.test",
    cluster_role: "user",
    account_status: "active",
    protocol_min_version: 1,
    protocol_max_version: 1,
    features: {},
    sessions,
    machines: [],
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

  it("says why a tab cannot take input, naming where to start it", () => {
    expect(paneUnavailableReason("stopped", "zoo-006")).toBe("zoo-006 is not running this project. Start it on zoo-006 first.");
    expect(paneUnavailableReason("no_access", "zoo-006")).toBe("Your access does not cover zoo-006, so this tab is read-only here.");
    expect(paneUnavailableReason("stopped", null)).toBe("This tab's machine is not running this project.");
    expect(paneUnavailableReason("usable", "zoo-006")).toBeNull();
  });

  it("describes a stopped machine in the words of the protocol's own refusal", () => {
    // The golden refusal of typed input for a tab on a stopped machine.
    const refusal = fixture("server-mutation-ack-input.json");
    expect(refusal.type).toBe("mutation_ack");
    expect(paneUnavailableReason("stopped", "zoo-005")).toBe(refusal.type === "mutation_ack" ? refusal.error : undefined);
  });

  it("judges one tab for screens that show only that tab", () => {
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005], true))
      .toBe("zoo-006 is not running this project. Start it on zoo-006 first.");
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005, { ...zoo006, runtime_access: false }], true))
      .toBe("Your access does not cover zoo-006, so this tab is read-only here.");
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005, zoo006], true)).toBeNull();
    // Without a live set, or for a tab the roster does not list, the server decides.
    expect(tabUnavailableReason({ machine_id: ZOO_006, hostname: "zoo-006" }, [zoo005], false)).toBeNull();
    expect(tabUnavailableReason(undefined, [zoo005], true)).toBeNull();
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
