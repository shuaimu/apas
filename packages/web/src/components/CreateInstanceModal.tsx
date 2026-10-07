"use client";

import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { X, FolderGit2 } from "lucide-react";
import { useStore, type MachineWithProjects } from "@/lib/store";

interface CreateInstanceModalProps {
  open: boolean;
  onClose: () => void;
  /** Canonical host/owner/repo key for the repo group. */
  gitRemote?: string;
  /** Raw cloneable origin URL captured from an existing checkout, if known. */
  cloneUrl?: string;
  /** Limit targets to one owned/shared cluster context. */
  clusterOwnerUserId?: string;
  /** Mobile bootstrap targets before the first pushed inventory arrives. */
  machineOptions?: MachineWithProjects[];
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

export function CreateInstanceModal({ open, onClose, gitRemote, cloneUrl, clusterOwnerUserId, machineOptions }: CreateInstanceModalProps) {
  const inventory = useStore((s) => s.machines);
  const machines = machineOptions ?? inventory;
  const createProjectInstance = useStore((s) => s.createProjectInstance);
  const registerLocalProject = useStore((s) => s.registerLocalProject);
  const dismissCreationOperation = useStore((s) => s.dismissCreationOperation);
  const userId = useStore((s) => s.userId);
  const [source, setSource] = useState<"clone" | "local">("clone");
  const [path, setPath] = useState("");
  const [requestId, setRequestId] = useState<string | null>(null);
  const result = useStore((s) => requestId ? s.pendingInstances[requestId] : undefined);
  const operation = result?.source === "local" ? result : undefined;
  const pending = operation?.status === "pending";
  const registered = operation?.status === "registered" ? operation.project : undefined;
  const fixedRemote = gitRemote?.trim() ?? "";
  const [url, setUrl] = useState(cloneUrl ?? (fixedRemote ? `https://${fixedRemote}.git` : ""));
  const [machineId, setMachineId] = useState("");
  const [mounted, setMounted] = useState(false);
  const contextMachines = useMemo(
    () => clusterOwnerUserId
      ? machines.filter((machine) => machine.clusterOwnerUserId === clusterOwnerUserId)
      : machines,
    [clusterOwnerUserId, machines],
  );
  const availableMachines = useMemo(
    () => source === "local"
      ? contextMachines.filter((machine) => machine.clusterAccess !== "member" && machine.localProjectRegistrationAvailable)
      : contextMachines,
    [contextMachines, source],
  );
  const selectedMachine = availableMachines.find((entry) => entry.machine.machineId === machineId);
  const sharedTarget = selectedMachine?.clusterAccess === "member";
  // Git treats a bare github.com path as a local directory.
  const submittedCloneUrl = url.trim().replace(/^github\.com\//i, "https://github.com/");
  const submittedRemote = fixedRemote || canonicalRemoteFromUrl(submittedCloneUrl);
  const instanceName = submittedRemote ? repoBasename(submittedRemote) : "";
  const branch = instanceName ? `apas/${instanceName}` : "";

  useEffect(() => setMounted(true), []);

  // Default the machine picker to the only machine (or first) when opened.
  useEffect(() => {
    if (open && !pending && !registered && (!machineId || !availableMachines.some((entry) => entry.machine.machineId === machineId))) {
      setMachineId(availableMachines[0]?.machine.machineId ?? "");
    }
  }, [open, availableMachines, machineId, pending, registered]);

  const validPath = (path.startsWith("/") || path.startsWith("~/")) && !path.includes("\0");
  const canSubmit = !pending && !registered && !!selectedMachine && (source === "local"
    ? validPath && !!selectedMachine.localProjectRegistrationAvailable && !sharedTarget
    : instanceName.trim().length > 0
      && url.trim().length > 0
      && submittedRemote.length > 0
      && !(sharedTarget && !selectedMachine.sharedProvisioningAvailable));

  const close = () => {
    if (pending) return;
    if (requestId) dismissCreationOperation(requestId);
    setRequestId(null);
    onClose();
  };

  if (!open || !mounted) return null;

  const submit = () => {
    if (!canSubmit) return;
    if (source === "local") {
      if (requestId) dismissCreationOperation(requestId);
      setRequestId(registerLocalProject(machineId, path, selectedMachine?.clusterOwnerUserId ?? userId ?? undefined));
      return;
    }
    const common: [string, string, string, string, string | undefined, string | undefined] = [
      machineId,
      submittedRemote,
      instanceName,
      branch,
      submittedCloneUrl || undefined,
      undefined,
    ];
    const sent = selectedMachine?.clusterOwnerUserId
      ? createProjectInstance(...common, selectedMachine.clusterOwnerUserId)
      : createProjectInstance(...common);
    // Keep the modal (and the entered values) open if the send was dropped
    // (e.g. the socket is reconnecting); the store shows an error toast.
    if (sent) close();
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/50"
      onClick={close}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={fixedRemote ? "New instance" : "New project"}
        className="mx-4 max-h-[90dvh] w-full max-w-md overflow-y-auto rounded-lg bg-white shadow-xl dark:bg-gray-800"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-gray-200 p-4 dark:border-gray-700">
          <h3 className="flex items-center gap-2 text-lg font-semibold">
            <FolderGit2 className="h-5 w-5 text-emerald-500" />
            {fixedRemote ? "New instance" : "New project"}
          </h3>
          <button
            onClick={close}
            disabled={pending}
            aria-label="Close new project"
            className="rounded p-1 hover:bg-gray-200 dark:hover:bg-gray-700"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="space-y-3 p-4">
          {!fixedRemote && !registered && (
            <div role="group" aria-label="Project source" className="flex gap-2">
              {(["clone", "local"] as const).map((value) => (
                <button key={value} type="button" aria-pressed={source === value} disabled={pending}
                  onClick={() => setSource(value)}
                  className={`rounded border px-3 py-2 text-sm ${source === value ? "border-emerald-600 bg-emerald-50 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-200" : "border-gray-300 dark:border-gray-600"}`}>
                  {value === "clone" ? "Clone from GitHub" : "Existing folder"}
                </button>
              ))}
            </div>
          )}
          {registered ? (
            <div role="status" className="space-y-3">
              <p className="font-medium">Folder registered</p>
              <p className="break-all font-mono text-sm">{registered.path}</p>
              <p className="text-sm text-gray-500">
                {registered.isRunning ? "This project was already running; its runtime is unchanged." : "The project is stopped. Use Start on Machines when you are ready. Saved agents only run when you explicitly start the project."}
              </p>
              <a className="inline-block rounded bg-emerald-600 px-3 py-2 text-sm text-white"
                href={`/machines?cluster_owner=${encodeURIComponent(operation?.clusterOwnerUserId || "owned")}&machine=${encodeURIComponent(operation!.machineId)}&project=${encodeURIComponent(registered.projectId)}#project-${encodeURIComponent(operation!.machineId)}-${encodeURIComponent(registered.projectId)}`}>
                View on Machines
              </a>
            </div>
          ) : <>
          <p className="text-sm text-gray-500 dark:text-gray-400">
            {source === "local" ? (
              <>Register an existing folder in place on your APAS machine, not an upload from this browser. Files and existing APAS configuration are preserved. Nothing is cloned or started.</>
            ) : fixedRemote ? (
              <>Clone <span className="font-medium text-gray-700 dark:text-gray-300">{repoLabel(fixedRemote)}</span> into a new project on a chosen machine and check out a fresh branch.</>
            ) : (
              <>Clone a GitHub repository into a new project on a chosen machine. The project and branch names are derived automatically.</>
            )}
          </p>

          {availableMachines.length === 0 ? (
            <div className="rounded border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700 dark:border-amber-700 dark:bg-amber-900/20 dark:text-amber-300">
              {source === "local"
                ? "Existing folders require a connected, updated daemon on a machine you own. Shared-cluster members cannot register host folders. Select your own cluster explicitly to use this source."
                : <>No machines are running the apas daemon. Run <code>apas daemon</code> on a machine to create instances.</>}
            </div>
          ) : (
            <>
              <Field label="Machine">
                <select
                  value={machineId}
                  disabled={pending}
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

              {source === "local" ? (
                <>
                  <Field label={`Folder path on ${selectedMachine?.machine.hostname || "selected machine"}`}>
                    <input type="text" value={path} onChange={(event) => setPath(event.target.value)}
                      disabled={pending} placeholder="~/work/my-project" aria-describedby="local-folder-help"
                      className="w-full rounded border border-gray-300 bg-white px-3 py-2 font-mono text-xs dark:border-gray-600 dark:bg-gray-700" />
                  </Field>
                  <p id="local-folder-help" className="text-xs text-gray-500">
                    Use an absolute path or ~/ relative to the daemon user&apos;s home on {selectedMachine?.machine.hostname}. The folder must already exist. Registration does not start projects or agents; Start is a separate action on Machines.
                  </p>
                  {path && !validPath && <p role="alert" className="text-sm text-red-600">Use an absolute path or a path starting with ~/.</p>}
                </>
              ) : <>
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
              </>}
            </>
          )}
          {source === "local" && availableMachines.length > 0 && <p className="text-xs text-gray-500">Only your connected machines with local-folder support are listed. Shared, offline, and older daemons are unavailable.</p>}
          </>}
          {pending && <p role="status" className="text-sm">Registering folder on the selected machine… Waiting for confirmed registration.</p>}
          {source === "local" && operation?.error && <p role="alert" className="text-sm text-red-600 dark:text-red-400">{operation.error}</p>}
        </div>

        <div className="flex justify-end gap-2 border-t border-gray-200 p-4 dark:border-gray-700">
          <button
            onClick={close}
            disabled={pending}
            className="rounded px-3 py-1.5 text-sm text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700"
          >
            {registered ? "Close" : "Cancel"}
          </button>
          {!registered && <button
            onClick={submit}
            disabled={!canSubmit}
            className="rounded bg-emerald-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-emerald-700 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {source === "local" ? pending ? "Registering…" : "Register folder" : "Create & start"}
          </button>}
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
