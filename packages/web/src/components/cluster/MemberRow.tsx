"use client";

import { useEffect, useRef, useState } from "react";
import type { MachineWithProjects } from "@/lib/store";
import type { LaunchProfile } from "@/components/PolicyEditor";
import { launchProfileLabel } from "@/lib/launchProfileLabels";
import {
  overviewButton,
  overviewMuted,
  overviewPrimaryButton,
  overviewQuietDangerButton,
} from "@/components/overview/OverviewElements";
import type { ClusterMemberDraft, ClusterMembership } from "./types";

const SELECT = "h-8 w-full rounded border border-gray-300 bg-white px-2 text-base pointer-coarse:h-10 sm:text-sm dark:border-gray-600 dark:bg-gray-800";

/** What a member may use, in one line: the machines, then the agent their
 *  new projects get. */
export function memberAccessSummary(
  member: ClusterMembership,
  machines: MachineWithProjects[],
  profileName: (key: string) => string = (key) => launchProfileLabel(key).label,
): string {
  let machinePart: string;
  if (member.allowed_machine_ids == null) {
    machinePart = "All current and future machines";
  } else {
    const names = member.allowed_machine_ids
      .map((id) => machines.find((entry) => entry.machine.machineId === id)?.machine.hostname)
      .filter((name): name is string => Boolean(name));
    const unknown = member.allowed_machine_ids.length - names.length;
    machinePart = [
      names.join(", "),
      unknown > 0 ? `${unknown} ${unknown === 1 ? "machine" : "machines"} not reported` : "",
    ].filter(Boolean).join(", ") || "No machines";
  }
  const agent = member.default_launch_profile
    ? profileName(member.default_launch_profile)
    : "Inherits cluster policy";
  return `${machinePart} · Agent for new projects: ${agent}`;
}

export function MemberRow({
  member,
  machines,
  profiles,
  editing,
  onEdit,
  onCancel,
  onSave,
  onRevoke,
}: {
  member: ClusterMembership;
  machines: MachineWithProjects[];
  profiles: LaunchProfile[];
  editing: boolean;
  onEdit: () => void;
  onCancel: () => void;
  onSave: (draft: ClusterMemberDraft) => void;
  onRevoke: () => void;
}) {
  const profileName = (key: string) =>
    profiles.find((profile) => profile.key === key)?.label ?? launchProfileLabel(key).label;
  // Edit hides itself while the editor is open. Focus goes into the editor,
  // and back to Edit when this row's Cancel or Save closes it (not when
  // another member's Edit does).
  const editButton = useRef<HTMLButtonElement | null>(null);
  const wasEditing = useRef(editing);
  const returnFocus = useRef(false);
  useEffect(() => {
    if (!editing && wasEditing.current && returnFocus.current) editButton.current?.focus();
    returnFocus.current = false;
    wasEditing.current = editing;
  }, [editing]);
  return (
    <div className="px-4 py-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <div className="min-w-0 flex-1 basis-60">
          <div className="break-all text-sm font-medium text-gray-900 dark:text-gray-100">{member.user_email}</div>
          <p className={overviewMuted}>{memberAccessSummary(member, machines, profileName)}</p>
        </div>
        <div className="ml-auto flex items-center gap-1">
          {!editing && (
            <button ref={editButton} type="button" aria-label={`Edit access for ${member.user_email}`} onClick={onEdit} className={overviewButton}>Edit</button>
          )}
          <button type="button" aria-label={`Revoke access for ${member.user_email}`} onClick={onRevoke} className={overviewQuietDangerButton}>Revoke access</button>
        </div>
      </div>
      {editing && (
        <MemberEditor
          member={member}
          machines={machines}
          profiles={profiles}
          onCancel={() => { returnFocus.current = true; onCancel(); }}
          onSave={(draft) => { returnFocus.current = true; onSave(draft); }}
        />
      )}
    </div>
  );
}

function MemberEditor({
  member,
  machines,
  profiles,
  onCancel,
  onSave,
}: {
  member: ClusterMembership;
  machines: MachineWithProjects[];
  profiles: LaunchProfile[];
  onCancel: () => void;
  onSave: (draft: ClusterMemberDraft) => void;
}) {
  const [draft, setDraft] = useState<ClusterMemberDraft>(() => ({
    allMachines: member.allowed_machine_ids == null,
    machineIds: member.allowed_machine_ids ?? [],
    defaultLaunchProfile: member.default_launch_profile ?? "",
  }));
  const first = useRef<HTMLInputElement | null>(null);
  useEffect(() => { first.current?.focus(); }, []);
  return (
    <div
      className="mt-3 space-y-3 rounded border border-gray-200 p-3 dark:border-gray-700"
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        onCancel();
      }}
    >
      <fieldset className="space-y-1.5">
        <legend className="mb-1 text-xs font-semibold text-gray-700 dark:text-gray-300">Machine access</legend>
        <label className="flex items-center gap-2 text-sm text-gray-800 dark:text-gray-200">
          <input
            ref={first}
            type="checkbox"
            checked={draft.allMachines}
            onChange={(event) => setDraft({ ...draft, allMachines: event.target.checked })}
          />
          All current and future machines
        </label>
        <div className="flex flex-wrap gap-x-4 gap-y-1.5 pl-6">
          {machines.map(({ machine }) => (
            <label key={machine.machineId} className="flex min-w-0 items-center gap-2 break-all text-sm text-gray-800 dark:text-gray-200">
              <input
                type="checkbox"
                disabled={draft.allMachines}
                checked={draft.allMachines || draft.machineIds.includes(machine.machineId)}
                onChange={(event) => setDraft({
                  ...draft,
                  machineIds: event.target.checked
                    ? [...draft.machineIds, machine.machineId]
                    : draft.machineIds.filter((id) => id !== machine.machineId),
                })}
              />
              {machine.hostname}
            </label>
          ))}
        </div>
      </fieldset>
      <label className="block space-y-1 text-xs font-semibold text-gray-700 dark:text-gray-300">
        <span>Agent for new projects</span>
        <select
          aria-label={`Default AI agent for ${member.user_email}`}
          value={draft.defaultLaunchProfile}
          onChange={(event) => setDraft({ ...draft, defaultLaunchProfile: event.target.value })}
          className={`${SELECT} font-normal sm:max-w-[calc(50%-0.375rem)]`}
        >
          <option value="">Inherit cluster policy</option>
          {profiles.map((profile) => <option key={profile.key} value={profile.key}>{profile.label}</option>)}
        </select>
      </label>
      <div className="flex justify-end gap-2">
        <button type="button" onClick={onCancel} className={overviewButton}>Cancel</button>
        <button
          type="button"
          disabled={!draft.allMachines && draft.machineIds.length === 0}
          onClick={() => onSave(draft)}
          className={overviewPrimaryButton}
        >
          Save access
        </button>
      </div>
    </div>
  );
}
