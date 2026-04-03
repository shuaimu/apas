"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, Play, RefreshCw, Square } from "lucide-react";
import { useStore } from "@/lib/store";

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
    setMachineMiniMaxConfig,
  } = useStore();
  const [minimaxDrafts, setMinimaxDrafts] = useState<Record<string, { apiBaseUrl: string; apiKey: string }>>({});

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
    <main className="min-h-screen bg-gray-50 dark:bg-gray-950 p-4 md:p-6">
      <div className="mx-auto max-w-6xl space-y-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Link
              href="/"
              className="inline-flex items-center gap-1 rounded border border-gray-300 px-2 py-1 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
            >
              <ArrowLeft className="h-4 w-4" />
              Back
            </Link>
            <h1 className="text-xl font-semibold">Machines</h1>
          </div>
          <button
            onClick={listMachines}
            className="inline-flex items-center gap-1 rounded border border-gray-300 px-3 py-1.5 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
          >
            <RefreshCw className="h-4 w-4" />
            Refresh
          </button>
        </div>

        {machines.length === 0 && (
          <div className="rounded border border-dashed border-gray-300 bg-white p-6 text-sm text-gray-600 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-300">
            No machines reported yet. Start `apas daemon` on a machine first.
          </div>
        )}

        {machines.map(({ machine, projects }) => (
          <section
            key={machine.machineId}
            className="rounded-lg border border-gray-200 bg-white shadow-sm dark:border-gray-800 dark:bg-gray-900"
          >
            <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <div className="font-medium">{machine.hostname}</div>
              <div className="text-xs text-gray-500">
                {machine.os}/{machine.arch}
                {machine.lastSeen ? ` • Last seen ${new Date(machine.lastSeen).toLocaleString()}` : ""}
              </div>
            </div>

            <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <div className="mb-2 text-sm font-medium">MiniMax Backend (Claude Runtime)</div>
              <div className="grid gap-2 md:grid-cols-[1fr_1fr_auto_auto]">
                <input
                  type="text"
                  value={minimaxDrafts[machine.machineId]?.apiBaseUrl ?? machine.minimaxBackend?.apiBaseUrl ?? ""}
                  onChange={(e) => {
                    const nextBaseUrl = e.target.value;
                    setMinimaxDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiBaseUrl: nextBaseUrl,
                        apiKey: prev[machine.machineId]?.apiKey ?? "",
                      },
                    }));
                  }}
                  placeholder="https://your-minimax-endpoint/anthropic"
                  className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-950"
                />
                <input
                  type="password"
                  value={minimaxDrafts[machine.machineId]?.apiKey ?? ""}
                  onChange={(e) => {
                    const nextApiKey = e.target.value;
                    setMinimaxDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiBaseUrl:
                          prev[machine.machineId]?.apiBaseUrl ?? machine.minimaxBackend?.apiBaseUrl ?? "",
                        apiKey: nextApiKey,
                      },
                    }));
                  }}
                  placeholder="API key (leave blank to keep current)"
                  className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-950"
                />
                <button
                  onClick={() => {
                    const draft = minimaxDrafts[machine.machineId];
                    const baseUrl = draft?.apiBaseUrl ?? machine.minimaxBackend?.apiBaseUrl ?? "";
                    const apiKey = draft?.apiKey ?? "";
                    setMachineMiniMaxConfig(
                      machine.machineId,
                      baseUrl,
                      apiKey.trim().length > 0 ? apiKey : undefined,
                      false,
                    );
                    setMinimaxDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiBaseUrl: baseUrl,
                        apiKey: "",
                      },
                    }));
                  }}
                  className="rounded bg-cyan-600 px-3 py-2 text-sm text-white hover:bg-cyan-700"
                >
                  Save
                </button>
                <button
                  onClick={() => {
                    const draft = minimaxDrafts[machine.machineId];
                    const baseUrl = draft?.apiBaseUrl ?? machine.minimaxBackend?.apiBaseUrl ?? "";
                    setMachineMiniMaxConfig(machine.machineId, baseUrl, undefined, true);
                    setMinimaxDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiBaseUrl: baseUrl,
                        apiKey: "",
                      },
                    }));
                  }}
                  className="rounded border border-gray-300 px-3 py-2 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
                >
                  Clear Key
                </button>
              </div>
              <div className="mt-2 text-xs text-gray-500">
                {machine.minimaxBackend?.apiKeyConfigured ? "API key configured" : "API key not configured"}
              </div>
            </div>

            <div className="divide-y divide-gray-200 dark:divide-gray-800">
              {projects.map((project) => (
                <div
                  key={project.projectId}
                  className="flex flex-col gap-3 px-4 py-3 md:flex-row md:items-center md:justify-between"
                >
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium">
                      {project.name || project.path.split("/").pop() || project.path}
                    </div>
                    <div className="truncate text-xs text-gray-500">{project.path}</div>
                    {project.lastError && (
                      <div className="mt-1 text-xs text-red-600 dark:text-red-400">
                        {project.lastError}
                      </div>
                    )}
                  </div>

                  <div className="flex items-center gap-2">
                    <span
                      className={`rounded px-2 py-1 text-xs ${
                        project.isRunning
                          ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-300"
                          : "bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-300"
                      }`}
                    >
                      {project.isRunning ? `Running${project.pid ? ` (pid ${project.pid})` : ""}` : "Stopped"}
                    </span>
                    {project.isRunning ? (
                      <button
                        onClick={() => stopMachineProjectCli(machine.machineId, project.projectId)}
                        className="inline-flex items-center gap-1 rounded bg-red-600 px-3 py-1.5 text-sm text-white hover:bg-red-700"
                      >
                        <Square className="h-4 w-4" />
                        Stop
                      </button>
                    ) : (
                      <button
                        onClick={() => startMachineProjectCli(machine.machineId, project.projectId)}
                        className="inline-flex items-center gap-1 rounded bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700"
                      >
                        <Play className="h-4 w-4" />
                        Start
                      </button>
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
