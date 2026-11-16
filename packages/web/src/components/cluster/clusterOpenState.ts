import type { MachineWithProjects, SessionInfo } from "@/lib/store";
import { buildProjectOpenModel, type ProjectOpenModel } from "@/lib/projectOpenState";

/**
 * What the cluster page knows about open states without attaching to any
 * project: the machine list and the session list, both as the store holds
 * them. `machines` is every machine this viewer lists, not only the selected
 * cluster's, so another of the viewer's machines that lists the same project
 * counts.
 */
export interface ClusterOpenStateSource {
  /** `openStateNegotiated(state)`: the server records open states. */
  supported: boolean;
  machines: MachineWithProjects[];
  sessions: Pick<SessionInfo, "id" | "projectId" | "openState" | "isActive">[];
}

/**
 * The open-state model of one project of the cluster's project list, or
 * `null` when neither a machine entry nor a session lists the project: the
 * cluster's list comes from durable placements, so a project whose machines
 * have not registered since the server started is in it with nothing known
 * about what runs. Nothing is then shown, instead of "Closed".
 *
 * It names no machine for a request, so it offers no Open or Close; those are
 * on the Machines view.
 */
export function clusterProjectOpenModel(
  project: { id: string; name?: string | null },
  source: ClusterOpenStateSource,
  isOwner = false,
): ProjectOpenModel | null {
  const sessions = source.sessions.filter((session) => (session.projectId ?? session.id) === project.id);
  const session = sessions.find((candidate) => candidate.openState)
    ?? sessions.find((candidate) => candidate.isActive)
    ?? sessions[0];
  const listed = source.machines.some((entry) =>
    entry.projects.some((candidate) => candidate.projectId === project.id));
  if (!session && !listed) return null;
  return buildProjectOpenModel({
    supported: source.supported,
    projectIds: [project.id],
    projectName: project.name ?? undefined,
    session: session ? { openState: session.openState, isActive: session.isActive } : null,
    machines: source.machines,
    target: null,
    isOwner,
  });
}
