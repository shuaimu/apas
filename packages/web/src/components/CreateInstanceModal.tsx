"use client";

import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { X, FolderGit2 } from "lucide-react";
import { useStore } from "@/lib/store";

interface CreateInstanceModalProps {
  open: boolean;
  onClose: () => void;
  /** Canonical host/owner/repo key for the repo group. */
  gitRemote?: string;
  /** Raw cloneable origin URL captured from an existing checkout, if known. */
  cloneUrl?: string;
  /** Limit targets to one owned/shared cluster context. */
  clusterOwnerUserId?: string;
}

function repoBasename(gitRemote: string): string {
  const parts = gitRemote.split("/").filter(Boolean);
  return parts[parts.length - 1] || "instance";
}

function canonicalRemoteFromUrl(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "";
  const scp = trimmed.match(/^git@([^:]+):(.+)$/i);
  if (scp) return `${scp[1].toLowerCase()}/${scp[2].replace(/\.git$/i, "")}`;
  try {
    const parsed = new URL(trimmed);
    return `${parsed.hostname.toLowerCase()}/${parsed.pathname.replace(/^\/+|\/+$/g, "").replace(/\.git$/i, "")}`;
  } catch {
    return trimmed
      .replace(/^[a-z]+:\/\//i, "")
      .replace(/^\/+|\/+$/g, "")
      .replace(/\.git$/i, "");
  }
}

// Show just owner/repo for github.com; full key otherwise (mirrors the sidebar).
function repoLabel(gitRemote: string): string {
  return gitRemote.startsWith("github.com/")
    ? gitRemote.slice("github.com/".length)
    : gitRemote;
}

export function CreateInstanceModal({ open, onClose, gitRemote, cloneUrl, clusterOwnerUserId }: CreateInstanceModalProps) {
  const machines = useStore((s) => s.machines);
  const createProjectInstance = useStore((s) => s.createProjectInstance);

  const fixedRemote = gitRemote?.trim() ?? "";
  const [url, setUrl] = useState(cloneUrl ?? (fixedRemote ? `https://${fixedRemote}.git` : ""));
  const [machineId, setMachineId] = useState("");
  const [mounted, setMounted] = useState(false);
  const availableMachines = useMemo(
    () => clusterOwnerUserId
      ? machines.filter((machine) => machine.clusterOwnerUserId === clusterOwnerUserId)
      : machines,
    [clusterOwnerUserId, machines],
  );
  const selectedMachine = availableMachines.find((entry) => entry.machine.machineId === machineId);
  const sharedTarget = selectedMachine?.clusterAccess === "member";
  const submittedRemote = fixedRemote || canonicalRemoteFromUrl(url);
  const instanceName = submittedRemote ? repoBasename(submittedRemote) : "";
  const branch = instanceName ? `apas/${instanceName}` : "";

  useEffect(() => setMounted(true), []);

  // Default the machine picker to the only machine (or first) when opened.
  useEffect(() => {
    if (open && (!machineId || !availableMachines.some((entry) => entry.machine.machineId === machineId))) {
      setMachineId(availableMachines[0]?.machine.machineId ?? "");
    }
  }, [open, availableMachines, machineId]);

  const canSubmit = useMemo(
    () => instanceName.trim().length > 0
      && url.trim().length > 0
      && submittedRemote.length > 0
      && machineId.length > 0
      && !(sharedTarget && !selectedMachine?.sharedProvisioningAvailable),
    [instanceName, url, submittedRemote, machineId, selectedMachine?.sharedProvisioningAvailable, sharedTarget],
  );

  if (!open || !mounted) return null;

  const submit = () => {
    if (!canSubmit) return;
    const common: [string, string, string, string, string | undefined, string | undefined] = [
      machineId,
      submittedRemote,
      instanceName,
      branch,
      url.trim() || undefined,
      undefined,
    ];
    const sent = selectedMachine?.clusterOwnerUserId
      ? createProjectInstance(...common, selectedMachine.clusterOwnerUserId)
      : createProjectInstance(...common);
    // Keep the modal (and the entered values) open if the send was dropped
    // (e.g. the socket is reconnecting); the store shows an error toast.
    if (sent) onClose();
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/50"
      onClick={onClose}
    >
      <div
        className="mx-4 w-full max-w-md rounded-lg bg-white shadow-xl dark:bg-gray-800"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-gray-200 p-4 dark:border-gray-700">
          <h3 className="flex items-center gap-2 text-lg font-semibold">
            <FolderGit2 className="h-5 w-5 text-emerald-500" />
            {fixedRemote ? "New instance" : "New project"}
          </h3>
          <button
            onClick={onClose}
            className="rounded p-1 hover:bg-gray-200 dark:hover:bg-gray-700"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="space-y-3 p-4">
          <p className="text-sm text-gray-500 dark:text-gray-400">
            {fixedRemote ? (
              <>Clone <span className="font-medium text-gray-700 dark:text-gray-300">{repoLabel(fixedRemote)}</span> into a new project on a chosen machine and check out a fresh branch.</>
            ) : (
              <>Clone a GitHub repository into a new project on a chosen machine. The project and branch names are derived automatically.</>
            )}
          </p>

          {availableMachines.length === 0 ? (
            <div className="rounded border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700 dark:border-amber-700 dark:bg-amber-900/20 dark:text-amber-300">
              No machines are running the apas daemon. Run <code>apas daemon</code> on a machine to create instances.
            </div>
          ) : (
            <>
              <Field label="Machine">
                <select
                  value={machineId}
                  onChange={(e) => setMachineId(e.target.value)}
                  className="w-full rounded border border-gray-300 bg-white px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700"
                >
                  {availableMachines.map((m) => (
                    <option
                      key={m.machine.machineId}
                      value={m.machine.machineId}
                      disabled={m.clusterAccess === "member" && !m.sharedProvisioningAvailable}
                    >
                      {m.machine.hostname} · {m.clusterAccess === "member" ? "Shared cluster" : "My cluster"}
                      {m.clusterAccess === "member" && !m.sharedProvisioningAvailable ? " (update required)" : ""}
                    </option>
                  ))}
                </select>
              </Field>

              <Field label="Clone URL">
                <input
                  type="text"
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://github.com/owner/repository"
                  className="w-full rounded border border-gray-300 bg-white px-3 py-2 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                />
              </Field>

              {sharedTarget ? (
                <div className="rounded border border-amber-300 bg-amber-50 p-3 text-xs text-amber-800 dark:border-amber-700 dark:bg-amber-950/30 dark:text-amber-200">
                  Shared machines accept only public <span className="font-mono">https://github.com/owner/repository</span> URLs. The checkout uses the owner&apos;s managed projects directory and cannot use private credentials.
                </div>
              ) : null}

              {instanceName && (
                <p className="text-xs text-gray-400">
                  Creates <span className="font-mono">~/apas_projects/{instanceName}</span> on branch <span className="font-mono">{branch}</span> (auto-suffixed if either exists).
                </p>
              )}
            </>
          )}
        </div>

        <div className="flex justify-end gap-2 border-t border-gray-200 p-4 dark:border-gray-700">
          <button
            onClick={onClose}
            className="rounded px-3 py-1.5 text-sm text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700"
          >
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={!canSubmit}
            className="rounded bg-emerald-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-emerald-700 disabled:cursor-not-allowed disabled:opacity-50"
          >
            Create &amp; start
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs font-medium text-gray-600 dark:text-gray-400">{label}</span>
      {children}
    </label>
  );
}
