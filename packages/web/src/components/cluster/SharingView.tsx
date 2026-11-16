"use client";

import { useState } from "react";
import type { MachineWithProjects } from "@/lib/store";
import type { LaunchProfile } from "@/components/PolicyEditor";
import {
  OverviewEmpty,
  OverviewNotice,
  OverviewSection,
  overviewCard,
} from "@/components/overview/OverviewElements";
import { MemberRow } from "./MemberRow";
import { AddMemberForm } from "./AddMemberForm";
import type { ConfirmRequest } from "./ConfirmDialog";
import type { ClusterMembership, LoadState, Mutate } from "./types";

/**
 * Who else may run projects on these machines. Adding a member is immediate
 * and shares compute only: no existing project is shared by it.
 */
export function SharingView({
  members,
  load,
  machines,
  profiles,
  mutate,
  requestConfirm,
}: {
  members: ClusterMembership[];
  load: LoadState;
  machines: MachineWithProjects[];
  profiles: LaunchProfile[];
  mutate: Mutate;
  requestConfirm: (request: ConfirmRequest) => void;
}) {
  const [editing, setEditing] = useState<string | null>(null);

  return (
    <OverviewSection
      title="Share machine access"
      description={<>
        <span className="block">
          Add an APAS account that already exists. It gets access at once. Choose the machines it can run on, and
          the default AI agent for projects it creates here.
        </span>
        <span className="mt-1 block">
          This does not share any existing or unrelated project. Project owners share those themselves. A person
          using a shared project hosted here also needs access to its selected hosting machine.
        </span>
      </>}
    >
      <div className="flex flex-col gap-6">
        <OverviewSection title="Cluster members" level={3}>
          {load === "loading" ? (
            <OverviewNotice status>Loading members…</OverviewNotice>
          ) : load === "failed" ? (
            <OverviewEmpty>Could not load members.</OverviewEmpty>
          ) : members.length === 0 ? (
            <OverviewEmpty>No members have access to this cluster.</OverviewEmpty>
          ) : (
            <div className={`${overviewCard} divide-y divide-gray-100 dark:divide-gray-700/60`}>
              {members.map((member) => (
                <MemberRow
                  key={member.user_id}
                  member={member}
                  machines={machines}
                  profiles={profiles}
                  editing={editing === member.user_id}
                  onEdit={() => setEditing(member.user_id)}
                  onCancel={() => setEditing(null)}
                  onSave={(draft) => void mutate(`/cluster/members/${member.user_id}`, {
                    method: "PATCH",
                    body: JSON.stringify({
                      allowed_machine_ids: draft.allMachines ? null : draft.machineIds,
                      default_launch_profile: draft.defaultLaunchProfile || null,
                    }),
                  }, { area: "page", success: "Access saved." }).then((ok) => { if (ok) setEditing(null); })}
                  onRevoke={() => requestConfirm({
                    title: `Revoke ${member.user_email}'s compute access?`,
                    body: <p>They can no longer use your machines. Their projects and project roles are kept.</p>,
                    confirmLabel: "Revoke access",
                    tone: "danger",
                    onConfirm: () => void mutate(`/cluster/members/${member.user_id}`, { method: "DELETE" }, { area: "page", success: "Access revoked." }),
                  })}
                />
              ))}
            </div>
          )}
        </OverviewSection>
        <OverviewSection title="Add a member" level={3}>
          <AddMemberForm
            machines={machines}
            profiles={profiles}
            onAdd={(body) => mutate("/cluster/members", { method: "POST", body: JSON.stringify(body) }, { area: "page", success: "Member added." })}
          />
        </OverviewSection>
      </div>
    </OverviewSection>
  );
}
