import { describe, expect, it } from "vitest";
import { projectRowLabel } from "./ProjectList";
import type { ClusterProject } from "./types";

function project(id: string, name: string, hostname: string): ClusterProject {
  return {
    id,
    project_name: name,
    hostname,
    owner_user_id: "me",
    owner_email: "me@example.com",
    lifecycle_status: "active",
    member_count: 0,
    active_session_count: 0,
    hosting_emails: [],
    connected: true,
    effective_policy: { team_available: false, allowed_launch_profiles: [], version: 1, project_suspended: false },
  };
}

describe("projectRowLabel", () => {
  it("names the machine only when another project has the same name", () => {
    const projects = [project("a", "mako", "zoo-005"), project("b", "mako", "zoo-006"), project("c", "apas", "zoo-005")];
    expect(projectRowLabel(projects[0], projects)).toBe("Manage mako on zoo-005");
    expect(projectRowLabel(projects[1], projects)).toBe("Manage mako on zoo-006");
    expect(projectRowLabel(projects[2], projects)).toBe("Manage apas");
  });
});
