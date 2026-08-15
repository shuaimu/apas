"use client";

import { useEffect, useState } from "react";

export interface LaunchProfile {
  key: string;
  label: string;
}

export interface EffectivePolicy {
  team_available: boolean;
  allowed_launch_profiles: string[];
  version: number;
  project_suspended: boolean;
}

/**
 * Shared by the system-administration surface and every account's own cluster
 * surface, so the two cannot drift into showing policy differently.
 *
 * `bound` is the policy one level up. Launch profiles narrow monotonically —
 * a level may only restrict what the level above allows — so anything outside
 * the bound is shown disabled with the reason, rather than offered and then
 * rejected by the server. Team availability is a default rather than a
 * ceiling, so it is never disabled here.
 */
export function PolicyEditor({
  title,
  description,
  policy,
  profiles,
  bound,
  onSave,
  onInherit,
  saveLabel = "Save policy",
}: {
  title: string;
  description?: string;
  policy: Pick<EffectivePolicy, "team_available" | "allowed_launch_profiles"> & {
    version?: number;
  };
  profiles: LaunchProfile[];
  bound?: EffectivePolicy | null;
  onSave: (policy: { team_available: boolean; allowed_launch_profiles: string[] }) => void;
  onInherit?: () => void;
  saveLabel?: string;
}) {
  const [team, setTeam] = useState(policy.team_available);
  const [allowed, setAllowed] = useState<string[]>(policy.allowed_launch_profiles);
  useEffect(() => {
    setTeam(policy.team_available);
    setAllowed(policy.allowed_launch_profiles);
  }, [policy]);

  const permitted = bound?.allowed_launch_profiles ?? null;
  return (
    <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900">
      <div className="mb-1 flex items-center justify-between gap-3">
        <h2 className="font-semibold">{title}</h2>
        {policy.version != null && <span className="text-xs text-gray-500">v{policy.version}</span>}
      </div>
      {description && <p className="mb-3 text-xs text-gray-500">{description}</p>}
      <label className="mb-3 flex items-center gap-2 text-sm">
        <input type="checkbox" checked={team} onChange={(event) => setTeam(event.target.checked)} />
        Team launch available
      </label>
      <div className="grid gap-2 sm:grid-cols-2">
        {profiles.map((profile) => {
          const blocked = permitted != null && !permitted.includes(profile.key);
          return (
            <label
              key={profile.key}
              title={blocked ? "Not permitted by the policy above this one" : undefined}
              className={`flex items-start gap-2 rounded border border-gray-200 p-2 text-xs dark:border-gray-700 ${blocked ? "opacity-50" : ""}`}
            >
              <input
                type="checkbox"
                disabled={blocked}
                checked={allowed.includes(profile.key) && !blocked}
                onChange={(event) =>
                  setAllowed((current) =>
                    event.target.checked
                      ? [...current, profile.key]
                      : current.filter((key) => key !== profile.key),
                  )
                }
              />
              <span>
                <span className="block font-medium">{profile.label}</span>
                <span className="font-mono text-gray-400">{profile.key}</span>
              </span>
            </label>
          );
        })}
      </div>
      <div className="mt-4 flex flex-wrap gap-2">
        <button
          onClick={() => onSave({ team_available: team, allowed_launch_profiles: allowed })}
          className="rounded-lg bg-blue-600 px-4 py-2 text-sm text-white"
        >
          {saveLabel}
        </button>
        {onInherit && (
          <button
            onClick={onInherit}
            className="rounded-lg border border-gray-300 px-4 py-2 text-sm dark:border-gray-700"
          >
            Inherit from above
          </button>
        )}
      </div>
    </div>
  );
}
