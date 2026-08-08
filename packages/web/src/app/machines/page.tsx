"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, Play, RefreshCw, Square } from "lucide-react";
import { useStore } from "@/lib/store";
import { AllProvidersUsage } from "@/components/UsageLimits";

const DEEPSEEK_API_BASE_URL = "https://api.deepseek.com/anthropic";

function formatMemory(memoryKb?: number): string {
  if (memoryKb == null) return "";
  if (memoryKb >= 1024 * 1024) return ` · ${(memoryKb / (1024 * 1024)).toFixed(1)} GiB`;
  if (memoryKb >= 1024) return ` · ${Math.round(memoryKb / 1024)} MiB`;
  return ` · ${memoryKb} KiB`;
}

export default function MachinesPage() {
  const router = useRouter();
  const {
    connected,
    connect,
    token,
    machines,
    listMachines,
    startMachineProjectCli,
    stopMachineProjectCli,
    setMachineDeepseekConfig,
    pendingInstances,
  } = useStore();
  const [deepseekDrafts, setDeepseekDrafts] = useState<Record<string, string>>({});
  const [deepseekSaved, setDeepseekSaved] = useState<Record<string, boolean>>({});

  useEffect(() => {
    const storedToken = localStorage.getItem("apas_token");
    if (!storedToken && !token) {
      router.push("/login");
      return;
    }
    if (!connected) {
      connect();
      return;
    }
    listMachines();
  }, [connected, connect, listMachines, router, token]);

  return (
    <main className="min-h-screen bg-gray-50 p-4 dark:bg-gray-950 md:p-6">
      <div className="mx-auto max-w-6xl space-y-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Link href="/" className="inline-flex items-center gap-1 rounded border border-gray-300 px-2 py-1 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800">
              <ArrowLeft className="h-4 w-4" /> Back
            </Link>
            <h1 className="text-xl font-semibold">Machines</h1>
          </div>
          <button aria-label="Refresh machines" onClick={listMachines} className="inline-flex items-center gap-1 rounded border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800">
            <RefreshCw className="h-4 w-4" /> Refresh
          </button>
        </div>

        {machines.length === 0 && (
          <div className="rounded border border-dashed border-gray-300 bg-white p-6 text-sm text-gray-600 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-300">
            No machines reported yet. Start `apas daemon` on a machine first.
          </div>
        )}

        <section className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm dark:border-gray-800 dark:bg-gray-900">
          <h2 className="mb-3 text-sm font-semibold text-gray-700 dark:text-gray-300">Usage</h2>
          <AllProvidersUsage />
        </section>

        {machines.map(({ machine, projects }) => (
          <section key={machine.machineId} className="rounded-lg border border-gray-200 bg-white shadow-sm dark:border-gray-800 dark:bg-gray-900">
            <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <div className="font-medium">{machine.hostname}</div>
              <div className="text-xs text-gray-500">
                {machine.os}/{machine.arch}
                {machine.lastSeen ? ` • Last seen ${new Date(machine.lastSeen).toLocaleString()}` : ""}
              </div>
            </div>

            <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <div className="mb-2 text-sm font-medium">DeepSeek Backend (Claude Runtime)</div>
              <div className="mb-2 text-xs text-gray-500">Backend URL: <span className="font-mono">{DEEPSEEK_API_BASE_URL}</span></div>
              <div className="grid gap-2 md:grid-cols-[1fr_auto_auto]">
                <input
                  aria-label={`DeepSeek API key for ${machine.hostname}`}
                  type="text"
                  value={deepseekDrafts[machine.machineId] ?? machine.deepseekBackend?.apiKey ?? ""}
                  onChange={(event) => {
                    setDeepseekDrafts((previous) => ({ ...previous, [machine.machineId]: event.target.value }));
                    setDeepseekSaved((previous) => ({ ...previous, [machine.machineId]: false }));
                  }}
                  placeholder="DeepSeek API key"
                  className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-950"
                />
                <button
                  aria-label={`Save DeepSeek API key for ${machine.hostname}`}
                  onClick={() => {
                    const apiKey = (deepseekDrafts[machine.machineId] ?? machine.deepseekBackend?.apiKey ?? "").trim();
                    setMachineDeepseekConfig(machine.machineId, apiKey || undefined, false);
                    setDeepseekDrafts((previous) => ({ ...previous, [machine.machineId]: apiKey }));
                    setDeepseekSaved((previous) => ({ ...previous, [machine.machineId]: true }));
                  }}
                  className="rounded bg-indigo-600 px-3 py-2 text-sm text-white hover:bg-indigo-700"
                >
                  {deepseekSaved[machine.machineId] ? "Saved" : "Save"}
                </button>
                <button
                  aria-label={`Clear DeepSeek API key for ${machine.hostname}`}
                  onClick={() => {
                    setMachineDeepseekConfig(machine.machineId, undefined, true);
                    setDeepseekDrafts((previous) => ({ ...previous, [machine.machineId]: "" }));
                    setDeepseekSaved((previous) => ({ ...previous, [machine.machineId]: false }));
                  }}
                  className="rounded border border-gray-300 px-3 py-2 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
                >Clear Key</button>
              </div>
              <div className="mt-2 text-xs text-gray-500">
                {machine.deepseekBackend?.apiKeyConfigured ? "API key configured" : "API key not configured"}
              </div>
            </div>

            <div className="divide-y divide-gray-200 dark:divide-gray-800">
              {Object.values(pendingInstances).filter((pending) => pending.machineId === machine.machineId).map((pending) => (
                <div key={pending.requestId} className="flex items-center justify-between px-4 py-3">
                  <div>
                    <div className="text-sm font-medium">{pending.instanceName}</div>
                    <div className="text-xs text-gray-500">Cloning {pending.gitRemote}…</div>
                  </div>
                  <span className="rounded bg-amber-100 px-2 py-1 text-xs text-amber-700 dark:bg-amber-900/30 dark:text-amber-300">Creating…</span>
                </div>
              ))}
              {projects.map((project) => (
                <div key={project.projectId} className="flex flex-col gap-3 px-4 py-3 md:flex-row md:items-center md:justify-between">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium">{project.name || project.path.split("/").pop() || project.path}</div>
                    <div className="truncate text-xs text-gray-500">{project.path}</div>
                    {project.lastError && <div className="mt-1 text-xs text-red-600 dark:text-red-400">{project.lastError}</div>}
                  </div>
                  <div className="flex items-center gap-2">
                    <span className={`rounded px-2 py-1 text-xs ${project.isRunning ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-300" : "bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-300"}`}>
                      {project.isRunning ? `Running${project.pid ? ` (pid ${project.pid})` : ""}${formatMemory(project.memoryKb)}` : "Stopped"}
                    </span>
                    {project.isRunning ? (
                      <button aria-label={`Stop ${project.name || project.path} on ${machine.hostname}`} onClick={() => stopMachineProjectCli(machine.machineId, project.projectId)} className="inline-flex items-center gap-1 rounded bg-red-600 px-3 py-1.5 text-sm text-white hover:bg-red-700"><Square className="h-4 w-4" /> Stop</button>
                    ) : (
                      <button aria-label={`Start ${project.name || project.path} on ${machine.hostname}`} onClick={() => startMachineProjectCli(machine.machineId, project.projectId)} className="inline-flex items-center gap-1 rounded bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700"><Play className="h-4 w-4" /> Start</button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </section>
        ))}
      </div>
    </main>
  );
}
