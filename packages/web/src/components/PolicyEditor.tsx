"use client";

import { useState } from "react";
import {
  overviewButton,
  overviewCard,
  overviewMuted,
  overviewPrimaryButton,
} from "@/components/overview/OverviewElements";

export interface LaunchProfile {
  key: string;
  label: string;
}

export interface EffectivePolicy {
  /** Retained on the wire for older builds. Team mode is removed: the value
   *  decides nothing and is not a setting. */
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
 * rejected by the server.
 *
 * `team_available` is not offered. Team mode is removed and the field decides
 * nothing; it stays in stored policy and on the wire only so older builds keep
 * parsing. A save sends back the value the editor was given, so saving the
 * launch profiles cannot change it.
 *
 * `appearance="overview"` draws it from the Overview's parts for the cluster
 * page: a part of a section (h3), each profile by its name with the key in
 * its title. `appearance="panel"` is the same without a card of its own, for
 * a place that is already one, such as the cluster page's project panel. The
 * default keeps the admin page as it was.
 *
 * Unsaved choices are kept until the policy's content changes (its version or
 * its profiles), not whenever a caller passes a new object: the cluster page
 * reloads the policy after every write, and that must not throw away a change
 * someone is still making.
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
  appearance = "admin",
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
  appearance?: "admin" | "overview" | "panel";
}) {
  const [allowed, setAllowed] = useState<string[]>(policy.allowed_launch_profiles);
  const contentKey = `${policy.version ?? ""}:${policy.allowed_launch_profiles.join("\n")}`;
  const [shownKey, setShownKey] = useState(contentKey);
  if (shownKey !== contentKey) {
    setShownKey(contentKey);
    setAllowed(policy.allowed_launch_profiles);
  }

  const permitted = bound?.allowed_launch_profiles ?? null;
  const toggle = (key: string, checked: boolean) =>
    setAllowed((current) => checked ? [...current, key] : current.filter((entry) => entry !== key));
  const save = () => onSave({ team_available: policy.team_available, allowed_launch_profiles: allowed });

  if (appearance === "overview" || appearance === "panel") {
    const panel = appearance === "panel";
    return (
      <div className={panel ? "min-w-0" : `${overviewCard} min-w-0 p-4`}>
        <div className="mb-1 flex items-center justify-between gap-3">
          <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">{title}</h3>
          {policy.version != null && <span className={overviewMuted}>v{policy.version}</span>}
        </div>
        {description && <p className={`mb-3 ${overviewMuted}`}>{description}</p>}
        <div className={panel ? "flex flex-col gap-1.5" : "grid grid-cols-[repeat(auto-fill,minmax(12rem,1fr))] gap-2"}>
          {profiles.map((profile) => {
            const blocked = permitted != null && !permitted.includes(profile.key);
            return (
              <label
                key={profile.key}
                title={blocked ? "Not permitted by the policy above this one" : profile.key}
                className={`flex min-w-0 items-center gap-2 text-xs text-gray-800 dark:text-gray-200 ${panel ? "" : "rounded border border-gray-200 px-2 py-1.5 dark:border-gray-700"} ${blocked ? "opacity-50" : ""}`}
              >
                <input
                  type="checkbox"
                  disabled={blocked}
                  checked={allowed.includes(profile.key) && !blocked}
                  onChange={(event) => toggle(profile.key, event.target.checked)}
                />
                <span className="min-w-0 truncate font-medium">{profile.label}</span>
              </label>
            );
          })}
        </div>
        <div className="mt-4 flex flex-wrap gap-2">
          <button type="button" onClick={save} className={overviewPrimaryButton}>{saveLabel}</button>
          {onInherit && (
            <button type="button" onClick={onInherit} className={overviewButton}>Inherit from above</button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900">
      <div className="mb-1 flex items-center justify-between gap-3">
        <h2 className="font-semibold">{title}</h2>
        {policy.version != null && <span className="text-xs text-gray-500">v{policy.version}</span>}
      </div>
      {description && <p className="mb-3 text-xs text-gray-500">{description}</p>}
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
                onChange={(event) => toggle(profile.key, event.target.checked)}
              />
              <span className="min-w-0">
                <span className="block font-medium">{profile.label}</span>
                <span className="min-w-0 break-all font-mono text-gray-400">{profile.key}</span>
              </span>
            </label>
          );
        })}
      </div>
      <div className="mt-4 flex flex-wrap gap-2">
        <button
          onClick={save}
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
