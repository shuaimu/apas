"use client";

import { useEffect, useRef, useState } from "react";
import type { MachineWithProjects } from "@/lib/store";
import type { LaunchProfile } from "@/components/PolicyEditor";
import { overviewCard, overviewPrimaryButton } from "@/components/overview/OverviewElements";

const FIELD = "h-8 w-full rounded border border-gray-300 bg-white px-2 text-base pointer-coarse:h-10 sm:text-sm dark:border-gray-600 dark:bg-gray-800";

/**
 * Adds an existing account to the cluster at once: its email, the machines it
 * may use, and the agent its new projects get, or the cluster policy.
 */
export function AddMemberForm({
  machines,
  profiles,
  onAdd,
}: {
  machines: MachineWithProjects[];
  profiles: LaunchProfile[];
  /** Resolves true when the member was added. */
  onAdd: (body: { email: string; allowed_machine_ids: string[]; default_launch_profile: string | null }) => Promise<boolean>;
}) {
  const [email, setEmail] = useState("");
  const [machineIds, setMachineIds] = useState<string[]>([]);
  const [profile, setProfile] = useState("");
  // The first profile is chosen for the person until they choose themselves;
  // after that "Inherit cluster policy" is a choice, not an empty value.
  const chosen = useRef(false);

  useEffect(() => {
    const known = profiles.some((entry) => entry.key === profile);
    if (known) return;
    if (chosen.current && profile === "") return;
    setProfile(profiles[0]?.key ?? "");
  }, [profiles, profile]);

  return (
    <form
      className={`${overviewCard} grid gap-3 p-4 sm:grid-cols-2`}
      onSubmit={(event) => {
        event.preventDefault();
        void onAdd({ email, allowed_machine_ids: machineIds, default_launch_profile: profile || null })
          .then((ok) => {
            if (!ok) return;
            setEmail("");
            setMachineIds([]);
            chosen.current = false;
            setProfile(profiles[0]?.key ?? "");
          });
      }}
    >
      <label className="block space-y-1 text-xs font-semibold text-gray-700 dark:text-gray-300">
        <span>Account email</span>
        <input
          aria-label="Member account email"
          type="email"
          required
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          placeholder="person@example.com"
          className={`${FIELD} font-normal`}
        />
      </label>
      <label className="block space-y-1 text-xs font-semibold text-gray-700 dark:text-gray-300">
        <span>Default AI agent for new projects</span>
        <select
          aria-label="Default AI agent for new member projects"
          value={profile}
          onChange={(event) => { chosen.current = true; setProfile(event.target.value); }}
          className={`${FIELD} font-normal`}
        >
          <option value="">Inherit cluster policy</option>
          {profiles.map((entry) => <option key={entry.key} value={entry.key}>{entry.label}</option>)}
        </select>
      </label>
      <fieldset className="min-w-0 sm:col-span-2">
        <legend className="mb-1 text-xs font-semibold text-gray-700 dark:text-gray-300">Machines this member can use</legend>
        {machines.length === 0 ? (
          <p className="text-sm text-amber-700 dark:text-amber-300">Connect a machine before adding a member.</p>
        ) : (
          <div className="flex flex-wrap gap-x-4 gap-y-1.5">
            {machines.map(({ machine }) => (
              <label key={machine.machineId} className="flex min-w-0 items-center gap-2 break-all text-sm text-gray-800 dark:text-gray-200">
                <input
                  type="checkbox"
                  checked={machineIds.includes(machine.machineId)}
                  onChange={(event) => setMachineIds((current) => event.target.checked
                    ? [...current, machine.machineId]
                    : current.filter((id) => id !== machine.machineId))}
                />
                {machine.hostname}
              </label>
            ))}
          </div>
        )}
      </fieldset>
      <p className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-900 sm:col-span-2 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
        This member can run code on every selected machine. Add only accounts you trust.
      </p>
      <div className="flex justify-end sm:col-span-2">
        <button type="submit" disabled={!email.trim() || machineIds.length === 0} className={overviewPrimaryButton}>Add member</button>
      </div>
    </form>
  );
}
