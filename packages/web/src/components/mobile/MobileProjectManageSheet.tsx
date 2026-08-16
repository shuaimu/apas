"use client";

import { X } from "lucide-react";
import { ALL_TAB_TYPES } from "@/lib/tabTypes";
import { useStore } from "@/lib/store";
import { useCanManageCurrentProject } from "@/lib/projectRole";

/**
 * Project management for the session being viewed.
 *
 * The only setting so far is which tab types the project permits — and this is
 * the first interface for it anywhere. The deny list has always been enforced
 * by the CLI on every `AddPane`; nothing could write it but a hand-edited
 * `.apas`.
 *
 * Presented as an allow list, stored as a deny list. That inversion is
 * deliberate and lives in the storage format: an absent allow list would mean
 * "nothing permitted", so every project predating the feature would refuse to
 * open any tab, and a provider added later would vanish from menus until
 * someone opted in. Empty deny list = everything allowed.
 */
export function MobileProjectManageSheet({ onClose }: { onClose: () => void }) {
  const sessionId = useStore((state) => state.sessionId);
  const flags = useStore((state) => (sessionId ? state.projectFlags?.[sessionId] : undefined));
  const updateProjectFlags = useStore((state) => state.updateProjectFlags);
  const canManage = useCanManageCurrentProject();

  const disallowed = flags?.disallowedTabTypes ?? [];
  const allowed = (key: string) => !disallowed.includes(key);

  const toggle = (key: string) => {
    if (!canManage || !flags) return;
    const next = allowed(key)
      ? [...disallowed, key]
      : disallowed.filter((entry) => entry !== key);
    // Carry the other flags through untouched: this path replaces all of them,
    // so sending only the tab types would silently reset team mode and the
    // autonomy switches.
    updateProjectFlags({
      autoApproveTodos: flags.autoApproveTodos,
      autoMergePrs: flags.autoMergePrs,
      teamEnabled: flags.teamEnabled,
      disallowedTabTypes: next,
    });
  };

  return (
    <div className="fixed inset-0 z-[95] flex items-end bg-black/45" onClick={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Manage project"
        onClick={(event) => event.stopPropagation()}
        className="max-h-[82dvh] w-full overflow-y-auto rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]"
      >
        <div className="flex items-start justify-between gap-3">
          <div>
            <h2 className="text-xl font-extrabold">Manage project</h2>
            <p className="mt-1 text-sm text-[#686873] dark:text-[#aaaab6]">
              {canManage
                ? "Tab types anyone may create in this project."
                : "Tab types anyone may create in this project. Only the project owner can change these."}
            </p>
          </div>
          <button
            type="button"
            aria-label="Close manage project"
            onClick={onClose}
            className="rounded-lg p-2 hover:bg-[#efeff5] dark:hover:bg-[#25252d]"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {!flags ? (
          <p className="mt-4 text-sm text-[#686873] dark:text-[#aaaab6]">
            Waiting for this project&apos;s settings…
          </p>
        ) : (
          <div className="mt-4 space-y-2">
            {ALL_TAB_TYPES.map((option) => {
              const on = allowed(option.key);
              return (
                <button
                  key={option.key}
                  type="button"
                  role="switch"
                  aria-checked={on}
                  aria-label={option.label}
                  disabled={!canManage}
                  onClick={() => toggle(option.key)}
                  className="flex w-full items-center justify-between gap-3 rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left disabled:opacity-60 dark:border-[#383842] dark:bg-[#1b1b21]"
                >
                  <span className="min-w-0">
                    <span className="block font-bold">{option.label}</span>
                    <span className="mt-0.5 block font-mono text-[11px] text-[#686873] dark:text-[#aaaab6]">
                      {option.key}
                    </span>
                  </span>
                  <span
                    aria-hidden="true"
                    className={`flex h-6 w-10 shrink-0 items-center rounded-full p-0.5 transition-colors ${
                      on ? "bg-[#6d5efc]" : "bg-[#dedee7] dark:bg-[#383842]"
                    }`}
                  >
                    <span
                      className={`h-5 w-5 rounded-full bg-white transition-transform ${
                        on ? "translate-x-4" : "translate-x-0"
                      }`}
                    />
                  </span>
                </button>
              );
            })}
            <p className="pt-1 text-xs text-[#686873] dark:text-[#aaaab6]">
              Turning one off stops new tabs of that type being created. Panes that already exist
              keep running and can still be relaunched.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
