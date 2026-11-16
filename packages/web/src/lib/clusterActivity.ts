/**
 * The cluster activity list in words: what happened, to what, and who did it.
 * The server records codes and ids; the page resolves them against what it
 * already loaded and keeps the raw code visible, muted, beside the words.
 */
import type { AuditEvent, ClusterMembership, ClusterProject } from "@/components/cluster/types";

const ACTION_LABELS: Record<string, string> = {
  "project.runtime_stopped": "Runtime stopped",
  "project.lifecycle_changed": "Project state changed",
  "project.owner_transferred": "Owner transferred",
  "project.member_added": "Project member added",
  "project.member_removed": "Project member removed",
  "project.member_left": "Project member left",
  "project.policy_changed": "Project policy changed",
  "project.provisioned": "Project created",
  "cluster.member_added": "Cluster member added",
  "cluster.member_joined": "Cluster member joined",
  "cluster.member_revoked": "Cluster access revoked",
  "cluster.member_policy_changed": "Member access changed",
  "cluster.policy_changed": "Cluster policy changed",
  "cluster.default_policy_changed": "Cluster policy changed",
  "cluster.invitation_created": "Invitation created",
  "cluster.invitation_revoked": "Invitation revoked",
  "autonomous_job.policy_updated": "Worker policy changed",
};

/** The action in words, or the code itself when this build has no words for
 *  it. `known` says which. */
export function actionLabel(code: string): { label: string; known: boolean } {
  const label = ACTION_LABELS[code];
  return label ? { label, known: true } : { label: code, known: false };
}

export type ActivityTarget =
  | { kind: "project"; projectId: string; label: string; hostname?: string }
  | { kind: "person"; label: string }
  | { kind: "other"; label: string; title: string };

function projectLabel(project: ClusterProject): string {
  return project.project_name || project.id;
}

export function resolveTarget(
  event: Pick<AuditEvent, "target_type" | "target_id">,
  projects: ClusterProject[],
  members: ClusterMembership[],
): ActivityTarget {
  if (event.target_type === "project") {
    const project = projects.find((entry) => entry.id === event.target_id);
    if (project) {
      const label = projectLabel(project);
      const sameName = projects.some((entry) => entry.id !== project.id && projectLabel(entry) === label);
      return { kind: "project", projectId: project.id, label, hostname: sameName ? project.hostname || undefined : undefined };
    }
  }
  const email = members.find((member) => member.user_id === event.target_id)?.user_email
    ?? projects.find((project) => project.owner_user_id === event.target_id)?.owner_email;
  if (email) return { kind: "person", label: email };
  return { kind: "other", label: `${event.target_type} ${event.target_id.slice(0, 8)}`, title: event.target_id };
}

export function resolveActor(
  event: Pick<AuditEvent, "actor_kind" | "actor_user_id">,
  userId: string | null | undefined,
  projects: ClusterProject[],
  members: ClusterMembership[],
): string {
  if (event.actor_kind === "system_admin") return "System administrator";
  if (userId && event.actor_user_id === userId) return "You";
  return members.find((member) => member.user_id === event.actor_user_id)?.user_email
    ?? projects.find((project) => project.owner_user_id === event.actor_user_id)?.owner_email
    ?? "Another account";
}
