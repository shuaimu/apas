"use client";

import { useMemo } from "react";
import { PolicyEditor, type LaunchProfile } from "@/components/PolicyEditor";
import { AutonomousJobPolicyEditor } from "@/components/AutonomousJobPolicyEditor";
import { OverviewEmpty, OverviewNotice, OverviewSection, overviewCard } from "@/components/overview/OverviewElements";
import { Disclosure } from "./Disclosure";
import { JOB_REASON_LABELS } from "./jobReasonLabels";
import type { ConfirmRequest } from "./ConfirmDialog";
import type { ClusterPolicy, ClusterRequest, LoadState, Mutate } from "./types";

/** What projects in the cluster may run: the launch profiles, and the
 *  autonomous worker limits behind a closed part, since workers are off
 *  until a deployment turns them on. */
export function PoliciesView({
  clusterPolicy,
  load,
  profiles,
  mutate,
  requestConfirm,
  request,
}: {
  clusterPolicy: ClusterPolicy | null;
  load: LoadState;
  profiles: LaunchProfile[];
  mutate: Mutate;
  requestConfirm: (request: ConfirmRequest) => void;
  request: ClusterRequest;
}) {
  // The editor keeps unsaved choices until this content changes, so a reload
  // after another write does not undo them.
  const policy = useMemo(() => clusterPolicy && ({
    team_available: clusterPolicy.cluster?.team_available ?? clusterPolicy.deployment.team_available,
    allowed_launch_profiles:
      clusterPolicy.cluster?.allowed_launch_profiles ?? clusterPolicy.deployment.allowed_launch_profiles,
    version: clusterPolicy.cluster?.version,
  }), [clusterPolicy]);

  return (
    <OverviewSection
      title="Policies"
      description="What projects in your cluster may run. You can narrow what the deployment allows, never widen it."
    >
      <div className="flex flex-col gap-4">
        {clusterPolicy && policy ? (
          <PolicyEditor
            appearance="overview"
            title="Cluster default policy"
            description="Applies to every project hosted in your cluster, including projects other accounts own."
            policy={policy}
            profiles={profiles}
            bound={clusterPolicy.deployment}
            saveLabel="Save cluster policy"
            onSave={(next) => void mutate("/cluster/policy/default", { method: "PATCH", body: JSON.stringify(next) }, { area: "page", success: "Cluster policy saved." })}
            onInherit={() => requestConfirm({
              title: "Inherit the deployment policy?",
              body: <p>Your cluster&apos;s own list is removed. Projects here follow the deployment&apos;s launch profiles.</p>,
              confirmLabel: "Inherit from above",
              tone: "primary",
              onConfirm: () => void mutate("/cluster/policy/default", {
                method: "PATCH",
                body: JSON.stringify({ team_available: null, allowed_launch_profiles: null }),
              }, { area: "page", success: "Cluster policy saved." }),
            })}
          />
        ) : load === "failed" ? (
          <OverviewEmpty>Could not load the cluster policy.</OverviewEmpty>
        ) : (
          <OverviewNotice status>Loading policy…</OverviewNotice>
        )}
        <div className={overviewCard}>
          <Disclosure
            id="cluster-job-policy"
            label="Cluster autonomous worker policy"
            description="Limits for autonomous workers in this cluster."
          >
            {() => (
              <AutonomousJobPolicyEditor
                embedded
                reasonLabels={JOB_REASON_LABELS}
                title="Cluster autonomous worker policy"
                endpoint="/cluster/job-policy"
                request={request}
              />
            )}
          </Disclosure>
        </div>
      </div>
    </OverviewSection>
  );
}
