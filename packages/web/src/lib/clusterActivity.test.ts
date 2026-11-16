import { describe, expect, it } from "vitest";
import type { ClusterMembership, ClusterProject } from "@/components/cluster/types";
import { actionLabel, resolveActor, resolveTarget } from "./clusterActivity";

const LABELS: Record<string, string> = {
  "project.opened": "Project opened",
  "project.closed": "Project closed",
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

function project(id: string, name: string, extra: Partial<ClusterProject> = {}): ClusterProject {
  return {
    id,
    project_name: name,
    hostname: `${id}-host`,
    owner_user_id: `${id}-owner`,
    owner_email: `${id}@example.com`,
    lifecycle_status: "active",
    member_count: 0,
    active_session_count: 0,
    hosting_emails: [],
    connected: true,
    effective_policy: { team_available: false, allowed_launch_profiles: [], version: 1, project_suspended: false },
    ...extra,
  };
}

const members: ClusterMembership[] = [
  { user_id: "member-1", user_email: "member@example.com", status: "active", allowed_machine_ids: null },
];

describe("actionLabel", () => {
  it.each(Object.entries(LABELS))("names %s", (code, label) => {
    expect(actionLabel(code)).toEqual({ label, known: true });
  });

  it("shows an unknown code as itself", () => {
    expect(actionLabel("cluster.something_new")).toEqual({ label: "cluster.something_new", known: false });
  });
});

describe("resolveTarget", () => {
  const projects = [project("p1", "apas"), project("p2", "mako"), project("p3", "mako")];

  it("names a known project, with its machine only when the name repeats", () => {
    expect(resolveTarget({ target_type: "project", target_id: "p1" }, projects, members))
      .toEqual({ kind: "project", projectId: "p1", label: "apas", hostname: undefined });
    expect(resolveTarget({ target_type: "project", target_id: "p2" }, projects, members))
      .toEqual({ kind: "project", projectId: "p2", label: "mako", hostname: "p2-host" });
  });

  it("names a member or a project owner by email", () => {
    expect(resolveTarget({ target_type: "user", target_id: "member-1" }, projects, members))
      .toEqual({ kind: "person", label: "member@example.com" });
    expect(resolveTarget({ target_type: "user", target_id: "p1-owner" }, projects, members))
      .toEqual({ kind: "person", label: "p1@example.com" });
  });

  it("shortens anything else and keeps the full id for its title", () => {
    expect(resolveTarget({ target_type: "project", target_id: "0123456789abcdef" }, projects, members))
      .toEqual({ kind: "other", label: "project 01234567", title: "0123456789abcdef" });
  });
});

describe("resolveActor", () => {
  const projects = [project("p1", "apas")];

  it("says who acted", () => {
    expect(resolveActor({ actor_kind: "system_admin", actor_user_id: "x" }, "me", projects, members)).toBe("System administrator");
    expect(resolveActor({ actor_kind: "user", actor_user_id: "me" }, "me", projects, members)).toBe("You");
    expect(resolveActor({ actor_kind: "user", actor_user_id: "member-1" }, "me", projects, members)).toBe("member@example.com");
    expect(resolveActor({ actor_kind: "user", actor_user_id: "p1-owner" }, "me", projects, members)).toBe("p1@example.com");
    expect(resolveActor({ actor_kind: "user", actor_user_id: "stranger" }, "me", projects, members)).toBe("Another account");
  });
});
