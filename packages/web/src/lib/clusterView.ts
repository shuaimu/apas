/**
 * The cluster page's view and selected project in its address, and which of
 * a machine's project rows it shows.
 *
 * The page reads `window.location.search` itself and writes with
 * `history.replaceState`, never the router: a view change must not remount
 * the page or refetch anything, and a test or a link that lands on the page
 * needs no router to work.
 */
import type { MachineProject, MachineWithProjects } from "@/lib/store";
import type { ClusterView } from "@/components/cluster/types";

export const CLUSTER_VIEWS: readonly ClusterView[] = ["machines", "projects", "sharing", "usage", "policies", "activity"];

/** The views a shared cluster offers; its owner gets every view. */
export const MEMBER_VIEWS: readonly ClusterView[] = ["machines", "projects"];

/** Whether a cluster offers a view: a shared cluster shows only Machines and
 *  Projects. */
export function clusterViewOffered(view: ClusterView, isOwner: boolean): boolean {
  return isOwner || MEMBER_VIEWS.includes(view);
}

/**
 * The view and project the address asks for. A link to one machine's project
 * (`?machine=&project=`) always opens Machines, where that row is; a view the
 * cluster does not offer opens Machines too. `selected` is kept only for
 * Projects.
 */
export function parseClusterView(search: string, isOwner: boolean): { view: ClusterView; selected: string | null } {
  const params = new URLSearchParams(search);
  const asked = params.get("view");
  let view: ClusterView = CLUSTER_VIEWS.includes(asked as ClusterView) ? asked as ClusterView : "machines";
  if (!clusterViewOffered(view, isOwner)) view = "machines";
  if (params.get("machine") && params.get("project")) view = "machines";
  const selected = view === "projects" ? params.get("selected") || null : null;
  return { view, selected };
}

/**
 * The address for a view. Every other parameter (such as `cluster_owner`)
 * stays. The one-machine link's parameters and its `#project-…` hash are
 * dropped: they have done their job once the page has moved on.
 */
export function clusterViewUrl(
  location: { pathname: string; search: string },
  view: ClusterView,
  selected?: string | null,
): string {
  const params = new URLSearchParams(location.search);
  for (const key of ["view", "selected", "machine", "project"]) params.delete(key);
  if (view !== "machines") params.set("view", view);
  if (view === "projects" && selected) params.set("selected", selected);
  const query = params.toString();
  return `${location.pathname}${query ? `?${query}` : ""}`;
}

/** More stopped projects than this on one machine fold into one line. */
export const STOPPED_FOLD_THRESHOLD = 3;

/** A machine project's name as every list shows it. */
export function machineProjectName(project: { name?: string; path: string }): string {
  return project.name || project.path.split("/").pop() || project.path;
}

function byName(a: MachineProject, b: MachineProject): number {
  return machineProjectName(a).localeCompare(machineProjectName(b));
}

/** Connected machines first, then by name. Every list of machines on the
 *  page uses this order, and a project starting or stopping does not move a
 *  machine. */
export function sortMachines(machines: MachineWithProjects[]): MachineWithProjects[] {
  return [...machines].sort((a, b) =>
    Number(b.daemonConnected === true) - Number(a.daemonConnected === true)
    || a.machine.hostname.localeCompare(b.machine.hostname));
}

/**
 * A machine's project rows in the order the card shows them: rows with an
 * error first, then running, then stopped, each by name. Rows with an error
 * are never folded. More than three stopped rows fold unless `open`.
 */
export function machineRowsToShow(rows: MachineProject[], pendingCount: number, open: boolean): {
  shown: MachineProject[];
  foldable: boolean;
  hiddenStopped: number;
  empty: boolean;
} {
  const errored = rows.filter((row) => row.lastError).sort(byName);
  const running = rows.filter((row) => !row.lastError && row.isRunning).sort(byName);
  const stopped = rows.filter((row) => !row.lastError && !row.isRunning).sort(byName);
  const foldable = stopped.length > STOPPED_FOLD_THRESHOLD;
  const folded = foldable && !open;
  return {
    shown: [...errored, ...running, ...(folded ? [] : stopped)],
    foldable,
    hiddenStopped: folded ? stopped.length : 0,
    empty: rows.length === 0 && pendingCount === 0,
  };
}
