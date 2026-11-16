"use client";

import { RotateCcw } from "lucide-react";
import type { CreationOperation, MachineWithProjects } from "@/lib/store";
import { isMachineBehind, type ReleaseVersion } from "@/lib/daemonVersion";
import { OverviewEmpty, OverviewSection, overviewButton } from "@/components/overview/OverviewElements";
import { MachineCard } from "./MachineCard";
import { DeepseekKeyList } from "./DeepseekKeyList";
import type { ConfirmRequest } from "./ConfirmDialog";

function versionText(version: ReleaseVersion | null): string | null {
  if (!version) return null;
  return `${String(version.year).padStart(2, "0")}.${String(version.month).padStart(2, "0")}.${version.build}`;
}

export function MachinesView({
  machines,
  sharedView,
  latestVersion,
  connected,
  pendingInstances,
  registrationTarget,
  stoppedOpen,
  onToggleStopped,
  canRebootAll,
  onReboot,
  onRebootAll,
  onStart,
  onStop,
  setMachineDeepseekConfig,
  requestConfirm,
}: {
  machines: MachineWithProjects[];
  sharedView: boolean;
  latestVersion: ReleaseVersion | null;
  connected: boolean;
  pendingInstances: Record<string, CreationOperation>;
  registrationTarget: { machineId: string; projectId: string } | null;
  stoppedOpen: (machineId: string) => boolean;
  onToggleStopped: (machineId: string) => void;
  canRebootAll: boolean;
  onReboot: (target: { id: string; hostname: string; behind: boolean }) => void;
  onRebootAll: () => void;
  onStart: (machineId: string, projectId: string) => boolean;
  onStop: (machineId: string, projectId: string) => boolean;
  setMachineDeepseekConfig: (machineId: string, apiKey?: string, clearApiKey?: boolean) => boolean;
  requestConfirm: (request: ConfirmRequest) => void;
}) {
  // The page passes the machines already in their order (`sortMachines`).
  const sorted = machines;
  const latest = versionText(latestVersion);

  const reported = machines.filter((entry) => typeof entry.daemonConnected === "boolean");
  const connectedCount = machines.filter((entry) => entry.daemonConnected === true).length;
  const behindCount = machines.filter((entry) => isMachineBehind(entry.machine.daemonVersion, latestVersion)).length;
  const parts = [
    sharedView ? "Machines the owner shares with you." : "",
    reported.length > 0 ? `${connectedCount} of ${machines.length} connected.` : "",
    !sharedView && behindCount > 0 ? `${behindCount} ${behindCount === 1 ? "needs" : "need"} an update.` : "",
  ].filter(Boolean);

  const pendingFor = (machineId: string) => Object.values(pendingInstances)
    .filter((pending) => pending.machineId === machineId && pending.status !== "registered");

  return (
    <>
      <OverviewSection
        title="Machines"
        description={parts.length > 0 ? parts.join(" ") : undefined}
        aside={canRebootAll ? (
          <button type="button" onClick={onRebootAll} className={overviewButton}>
            <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" /> Reboot all daemons
          </button>
        ) : undefined}
      >
        {sorted.length === 0 ? (
          <OverviewEmpty>
            No machines reported yet. Start <code className="font-mono">apas daemon</code> on a machine first.
          </OverviewEmpty>
        ) : (
          <div className="flex flex-col gap-3">
            {sorted.map((entry) => {
              const behind = isMachineBehind(entry.machine.daemonVersion, latestVersion);
              const id = entry.machine.machineId;
              return (
                <MachineCard
                  key={id}
                  entry={entry}
                  behind={behind}
                  latestVersion={latest}
                  showReboot={!sharedView && entry.clusterAccess !== "member"}
                  connected={connected}
                  pending={pendingFor(id)}
                  stoppedOpen={stoppedOpen(id)}
                  onToggleStopped={() => onToggleStopped(id)}
                  registrationProjectId={registrationTarget?.machineId === id ? registrationTarget.projectId : undefined}
                  onReboot={() => onReboot({ id, hostname: entry.machine.hostname, behind })}
                  onStart={(projectId) => onStart(id, projectId)}
                  onStop={(projectId) => onStop(id, projectId)}
                />
              );
            })}
          </div>
        )}
      </OverviewSection>
      {!sharedView && sorted.length > 0 && (
        <DeepseekKeyList
          machines={sorted}
          connected={connected}
          setMachineDeepseekConfig={setMachineDeepseekConfig}
          requestConfirm={requestConfirm}
        />
      )}
    </>
  );
}
