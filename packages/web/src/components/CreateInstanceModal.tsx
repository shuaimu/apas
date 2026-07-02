"use client";

import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { X, FolderGit2 } from "lucide-react";
import { useStore } from "@/lib/store";

interface CreateInstanceModalProps {
  open: boolean;
  onClose: () => void;
  /** Canonical host/owner/repo key for the repo group. */
  gitRemote: string;
  /** Raw cloneable origin URL captured from an existing checkout, if known. */
  cloneUrl?: string;
}

function repoBasename(gitRemote: string): string {
  const parts = gitRemote.split("/").filter(Boolean);
  return parts[parts.length - 1] || "instance";
}

// Show just owner/repo for github.com; full key otherwise (mirrors the sidebar).
function repoLabel(gitRemote: string): string {
  return gitRemote.startsWith("github.com/")
    ? gitRemote.slice("github.com/".length)
    : gitRemote;
}

export function CreateInstanceModal({ open, onClose, gitRemote, cloneUrl }: CreateInstanceModalProps) {
  const machines = useStore((s) => s.machines);
  const createProjectInstance = useStore((s) => s.createProjectInstance);

  const base = repoBasename(gitRemote);
  const [instanceName, setInstanceName] = useState(base);
  const [branch, setBranch] = useState(`apas/${base}`);
  const [url, setUrl] = useState(cloneUrl ?? `https://${gitRemote}.git`);
  const [basePath, setBasePath] = useState("");
  const [machineId, setMachineId] = useState("");
  const [mounted, setMounted] = useState(false);

  useEffect(() => setMounted(true), []);

  // Default the machine picker to the only machine (or first) when opened.
  useEffect(() => {
    if (open && !machineId && machines.length > 0) {
      setMachineId(machines[0].machine.machineId);
    }
  }, [open, machines, machineId]);

  const canSubmit = useMemo(
    () => instanceName.trim().length > 0 && url.trim().length > 0 && machineId.length > 0,
    [instanceName, url, machineId],
  );

  if (!open || !mounted) return null;

  const submit = () => {
    if (!canSubmit) return;
    const sent = createProjectInstance(
      machineId,
      gitRemote,
      instanceName.trim(),
      branch.trim() || `apas/${instanceName.trim()}`,
      url.trim() || undefined,
      basePath.trim() || undefined,
    );
    // Keep the modal (and the entered values) open if the send was dropped
    // (e.g. the socket is reconnecting); the store shows an error toast.
    if (sent) onClose();
  };

  const previewPath = `${basePath.trim() || "~/apas_projects"}/${instanceName.trim() || base}`;

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
            New instance
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
            Clone <span className="font-medium text-gray-700 dark:text-gray-300">{repoLabel(gitRemote)}</span> into a
            new project on a chosen machine and check out a fresh branch.
          </p>

          {machines.length === 0 ? (
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
                  {machines.map((m) => (
                    <option key={m.machine.machineId} value={m.machine.machineId}>
                      {m.machine.hostname}
                    </option>
                  ))}
                </select>
              </Field>

              <Field label="Instance name">
                <input
                  type="text"
                  value={instanceName}
                  onChange={(e) => setInstanceName(e.target.value)}
                  placeholder={base}
                  className="w-full rounded border border-gray-300 bg-white px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700"
                />
              </Field>

              <Field label="New branch">
                <input
                  type="text"
                  value={branch}
                  onChange={(e) => setBranch(e.target.value)}
                  placeholder={`apas/${base}`}
                  className="w-full rounded border border-gray-300 bg-white px-3 py-2 text-sm dark:border-gray-600 dark:bg-gray-700"
                />
              </Field>

              <Field label="Clone URL">
                <input
                  type="text"
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  className="w-full rounded border border-gray-300 bg-white px-3 py-2 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                />
              </Field>

              <Field label="Projects root (optional)">
                <input
                  type="text"
                  value={basePath}
                  onChange={(e) => setBasePath(e.target.value)}
                  placeholder="~/apas_projects"
                  className="w-full rounded border border-gray-300 bg-white px-3 py-2 font-mono text-xs dark:border-gray-600 dark:bg-gray-700"
                />
              </Field>

              <p className="text-xs text-gray-400">
                Clones into <span className="font-mono">{previewPath}</span> (auto-suffixed if it exists).
              </p>
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
