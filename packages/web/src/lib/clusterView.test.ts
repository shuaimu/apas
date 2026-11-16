import { describe, expect, it } from "vitest";
import type { MachineProject } from "@/lib/store";
import { clusterViewOffered, clusterViewUrl, machineRowsToShow, parseClusterView, sortMachines } from "./clusterView";

describe("parseClusterView", () => {
  it("defaults to Machines and reads every known view", () => {
    expect(parseClusterView("", true)).toEqual({ view: "machines", selected: null });
    expect(parseClusterView("?view=nonsense", true).view).toBe("machines");
    for (const view of ["projects", "sharing", "usage", "policies", "activity"]) {
      expect(parseClusterView(`?view=${view}`, true).view).toBe(view);
    }
  });

  it("gives a shared cluster only Machines and Projects", () => {
    expect(parseClusterView("?view=policies", false).view).toBe("machines");
    expect(parseClusterView("?view=projects", false).view).toBe("projects");
    expect(clusterViewOffered("sharing", false)).toBe(false);
    expect(clusterViewOffered("machines", false)).toBe(true);
  });

  it("opens Machines for a one-machine link, whatever else is asked", () => {
    expect(parseClusterView("?view=usage&machine=m&project=p", true)).toEqual({ view: "machines", selected: null });
    // Half a link is not a link.
    expect(parseClusterView("?view=usage&machine=m", true).view).toBe("usage");
  });

  it("keeps a selected project only for Projects", () => {
    expect(parseClusterView("?view=projects&selected=abc", true)).toEqual({ view: "projects", selected: "abc" });
    expect(parseClusterView("?view=usage&selected=abc", true).selected).toBeNull();
  });
});

describe("clusterViewUrl", () => {
  const location = { pathname: "/machines", search: "?cluster_owner=owned&machine=m&project=p&view=usage&selected=x" };

  it("keeps other parameters and drops the one-machine link", () => {
    expect(clusterViewUrl(location, "projects")).toBe("/machines?cluster_owner=owned&view=projects");
    expect(clusterViewUrl(location, "projects", "abc")).toBe("/machines?cluster_owner=owned&view=projects&selected=abc");
  });

  it("leaves the view out for Machines and the selection out elsewhere", () => {
    expect(clusterViewUrl(location, "machines", "abc")).toBe("/machines?cluster_owner=owned");
    expect(clusterViewUrl({ pathname: "/machines", search: "" }, "machines")).toBe("/machines");
    expect(clusterViewUrl(location, "activity", "abc")).toBe("/machines?cluster_owner=owned&view=activity");
  });
});

describe("machineRowsToShow", () => {
  const row = (name: string, extra: Partial<MachineProject> = {}): MachineProject =>
    ({ projectId: name, name, path: `/srv/${name}`, isRunning: false, ...extra });

  it("orders errors, then running, then stopped, each by name", () => {
    const rows = [row("b"), row("a", { isRunning: true }), row("z", { lastError: "boom" }), row("c", { isRunning: true })];
    expect(machineRowsToShow(rows, 0, false).shown.map((entry) => entry.name)).toEqual(["z", "a", "c", "b"]);
  });

  it("folds more than three stopped rows, never rows with an error", () => {
    const three = [row("a"), row("b"), row("c")];
    expect(machineRowsToShow(three, 0, false)).toMatchObject({ foldable: false, hiddenStopped: 0 });
    const four = [...three, row("d"), row("e", { lastError: "boom" })];
    const folded = machineRowsToShow(four, 0, false);
    expect(folded).toMatchObject({ foldable: true, hiddenStopped: 4 });
    expect(folded.shown.map((entry) => entry.name)).toEqual(["e"]);
    expect(machineRowsToShow(four, 0, true).shown).toHaveLength(5);
  });

  it("is empty only with no rows and nothing being created", () => {
    expect(machineRowsToShow([], 0, false).empty).toBe(true);
    expect(machineRowsToShow([], 1, false).empty).toBe(false);
  });
});

describe("sortMachines", () => {
  it("puts connected machines first, then orders by name", () => {
    const entry = (hostname: string, daemonConnected?: boolean) => ({
      machine: { machineId: hostname, hostname, os: "linux", arch: "x64" },
      projects: [],
      daemonConnected,
    });
    const order = sortMachines([entry("zoo-006", true), entry("mac", false), entry("zoo-005", true), entry("air")])
      .map(({ machine }) => machine.hostname);
    expect(order).toEqual(["zoo-005", "zoo-006", "air", "mac"]);
  });
});
