"use client";

import { useStore } from "@/lib/store";
import { useCanManageCurrentProject } from "@/lib/projectRole";
import { ALL_TAB_TYPES, isTabTypeAllowed } from "@/lib/tabTypes";

const DEFAULT_FLAGS = {
  autoApproveTodos: false,
  autoMergePrs: false,
  teamEnabled: false,
  disallowedTabTypes: [] as string[],
};

/**
 * Which tab types users may create on this project.
 *
 * Presented as an *allow* list because that is how someone thinks about it,
 * but stored as a deny list — see `shared::tab_type_allowed`. An allow list on
 * the wire would make an absent field mean "nothing permitted", so every
 * project predating the feature would refuse to open any tab.
 *
 * Not gated on team mode: these are ordinary user tabs, which exist whether or
 * not a team is running. Managed team panes are exempt from the restriction
 * entirely — restricting user tab types is not a request to break your own
 * Tech Lead.
 */
export function AllowedTabTypesCard() {
  const sessionId = useStore((s) => s.sessionId);
  const projectFlags = useStore((s) => s.projectFlags);
  const updateProjectFlags = useStore((s) => s.updateProjectFlags);
  const showToast = useStore((s) => s.showToast);
  const canManage = useCanManageCurrentProject();

  const flags = sessionId ? projectFlags[sessionId] ?? DEFAULT_FLAGS : DEFAULT_FLAGS;
  // `?? []` because a flags object cached before this field existed has no
  // array at all — reading `.some` off undefined took down the whole page.
  const disallowed = flags.disallowedTabTypes ?? [];

  const toggle = (key: string, kind: string, provider: string) => {
    if (!canManage) return;
    const allowedNow = isTabTypeAllowed(disallowed, kind as never, provider);
    const next = allowedNow
      ? [...disallowed, key]
      : disallowed.filter((d) => d.trim().toLowerCase() !== key.toLowerCase());

    if (allowedNow && next.length === ALL_TAB_TYPES.length) {
      // Every type off means nobody can open any tab. Recoverable, but it is
      // not obviously what someone meant by unticking the last box.
      if (
        typeof window !== "undefined" &&
        !window.confirm(
          "Disallow every tab type? Users will not be able to create any new tab on this project until you allow at least one.",
        )
      ) {
        return;
      }
    }
    updateProjectFlags({ ...flags, disallowedTabTypes: next });
    showToast(
      allowedNow ? `Disallowed ${key}` : `Allowed ${key}`,
      "success",
    );
  };

  const allowedCount = ALL_TAB_TYPES.filter((t) =>
    isTabTypeAllowed(disallowed, t.kind, t.provider),
  ).length;

  return (
    <div className="mb-4 rounded-lg border border-sky-200 bg-sky-50/60 px-4 py-3 dark:border-sky-700/40 dark:bg-sky-900/10">
      <div className="mb-1.5 flex flex-wrap items-center gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-sky-700 dark:text-sky-400">
          Allowed tab types
        </span>
        <span className="text-[11px] text-sky-700/70 dark:text-sky-400/70">
          {canManage
            ? `${allowedCount} of ${ALL_TAB_TYPES.length} allowed`
            : "(read-only — only the project owner or an admin can change these)"}
        </span>
      </div>
      <p className="mb-2 text-xs text-gray-500 dark:text-gray-400">
        Unticked types disappear from the + menu and are refused if requested
        anyway. Managed team panes are unaffected.
      </p>
      <div className="flex flex-wrap gap-x-6 gap-y-2">
        {ALL_TAB_TYPES.map((t) => {
          const allowed = isTabTypeAllowed(disallowed, t.kind, t.provider);
          return (
            <label
              key={t.key}
              className={`flex items-center gap-2 text-sm ${
                canManage ? "cursor-pointer" : "cursor-not-allowed opacity-60"
              }`}
            >
              <input
                type="checkbox"
                className="h-4 w-4"
                checked={allowed}
                disabled={!canManage}
                onChange={() => toggle(t.key, t.kind, t.provider)}
              />
              <span className="font-medium text-gray-800 dark:text-gray-200">
                {t.label}
              </span>
            </label>
          );
        })}
      </div>
      {allowedCount === 0 && (
        <p className="mt-2 text-xs text-amber-700 dark:text-amber-400">
          No tab types are allowed — users cannot create any new tab on this
          project.
        </p>
      )}
    </div>
  );
}
