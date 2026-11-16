/** Plain names for the reasons the server gives when autonomous workers
 *  cannot run (`routes/job_policy.rs`). Several are raised once per machine
 *  report, so they say "A machine". The reasons a worker profile reports
 *  about itself are shown as they came: their codes are the host's. */
export const JOB_REASON_LABELS: Record<string, string> = {
  job_policy_disabled: "Worker jobs are turned off by the policy that applies here.",
  job_policy_invalid: "The stored worker policy cannot be read.",
  job_policy_inherited_invalid: "A worker policy above this one cannot be read.",
  job_readiness_not_reported: "No machine has reported whether it can run workers.",
  job_readiness_expired: "A machine's worker report is out of date.",
  job_target_offline: "A machine that reported worker support is offline.",
  job_worker_capability_missing: "A machine runs an APAS that cannot run workers.",
  job_worker_protocol_unsupported: "A machine runs an APAS whose worker version does not match the server's.",
  job_worker_profiles_unavailable: "A machine reports no worker profile.",
  job_worker_readiness_failed: "A machine reports a worker profile that is not ready.",
};
