import type { EffectivePolicy } from "@/components/PolicyEditor";

/** The cluster surface's HTTP shapes, as `/cluster/*` answers them. */

export interface Page<T> { items: T[]; limit: number; offset: number }

export interface ClusterProject {
  id: string;
  project_name?: string | null;
  hostname?: string | null;
  owner_user_id: string;
  owner_email: string;
  lifecycle_status: "active" | "suspended";
  member_count: number;
  active_session_count: number;
  hosting_emails: string[];
  last_activity?: string;
  connected: boolean;
  effective_policy: EffectivePolicy;
}

export interface ProjectMember { user_id: string; email: string; created_at?: string }

export interface ClusterProjectDetail {
  project: Omit<ClusterProject, "connected" | "effective_policy">;
  members: ProjectMember[];
  policy: EffectivePolicy;
}

export interface ClusterPolicy {
  cluster: {
    user_id: string;
    team_available: boolean | null;
    allowed_launch_profiles: string[] | null;
    version: number;
  } | null;
  deployment: EffectivePolicy;
}

export interface AuditEvent {
  id: number;
  actor_kind: string;
  actor_user_id: string;
  action: string;
  target_type: string;
  target_id: string;
  details?: string;
  created_at?: string;
}

export interface ClusterReference {
  owner_user_id: string;
  owner_email: string;
  access: "owner" | "member";
  accepted_at?: string | null;
}

export interface ClusterMembership {
  user_id: string;
  user_email: string;
  status: string;
  accepted_at?: string | null;
  allowed_machine_ids: string[] | null;
  default_launch_profile?: string | null;
}

export interface ClusterMemberDraft {
  allMachines: boolean;
  machineIds: string[];
  defaultLaunchProfile: string;
}

export interface UsageCounters {
  prompts: number;
  responses: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  cost_usd_reported: boolean;
}

export interface UsageWindows { lifetime: UsageCounters; last_7d: UsageCounters; today: UsageCounters }

export interface ClusterUsageProject {
  project_id: string;
  project_name?: string | null;
  owner_email: string;
  usage: UsageWindows;
}

export interface ClusterUsageReport extends UsageWindows {
  projects: ClusterUsageProject[];
}

/** One HTTP load's progress, so an empty state shows only after an answer. */
export type LoadState = "loading" | "loaded" | "failed";

export type ClusterView = "machines" | "projects" | "sharing" | "usage" | "policies" | "activity";

export type ClusterRequest = <T>(path: string, init?: RequestInit) => Promise<T>;

/** A write's success text and which notice shows it: the page's, or the
 *  selected project's panel. */
export interface MutateOptions {
  area: "page" | "project";
  success: string;
  after?: () => Promise<void>;
}

export type Mutate = (path: string, init: RequestInit, options: MutateOptions) => Promise<boolean>;

export const TRUST_WARNING = "Projects run on the cluster owner's machines. The owner can access files, processes, terminal output, and credentials exposed to those processes. Join only a cluster owner you trust.";

/** Resident memory as the machine rows show it: "2.0 GiB". Empty when the
 *  daemon reports none. */
export function formatMemory(memoryKb?: number): string {
  if (memoryKb == null) return "";
  if (memoryKb >= 1024 * 1024) return `${(memoryKb / (1024 * 1024)).toFixed(1)} GiB`;
  if (memoryKb >= 1024) return `${Math.round(memoryKb / 1024)} MiB`;
  return `${memoryKb} KiB`;
}
