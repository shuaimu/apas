"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, Play, RefreshCw, Square } from "lucide-react";
import { useStore } from "@/lib/store";
import { AllProvidersUsage } from "@/components/UsageLimits";

const MINIMAX_API_BASE_URL = "https://api.minimax.io/anthropic";
const GLM_API_BASE_URL = "https://api.z.ai/api/anthropic";
const DEEPSEEK_API_BASE_URL = "https://api.deepseek.com/anthropic";

function formatMemory(memoryKb?: number): string {
  if (memoryKb == null) return "";
  if (memoryKb >= 1024 * 1024) {
    return ` · ${(memoryKb / (1024 * 1024)).toFixed(1)} GiB`;
  }
  if (memoryKb >= 1024) {
    return ` · ${Math.round(memoryKb / 1024)} MiB`;
  }
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
    setMachineMiniMaxConfig,
    setMachineGlmConfig,
    setMachineDeepseekConfig,
  } = useStore();
  const [minimaxDrafts, setMinimaxDrafts] = useState<Record<string, { apiKey: string }>>({});
  const [minimaxSaved, setMinimaxSaved] = useState<Record<string, boolean>>({});
  const [glmDrafts, setGlmDrafts] = useState<Record<string, { apiKey: string }>>({});
  const [glmSaved, setGlmSaved] = useState<Record<string, boolean>>({});
  const [deepseekDrafts, setDeepseekDrafts] = useState<Record<string, { apiKey: string }>>({});
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
            aria-label="Refresh machines"
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

        <section className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm dark:border-gray-800 dark:bg-gray-900">
          <h2 className="mb-3 text-sm font-semibold text-gray-700 dark:text-gray-300">Usage</h2>
          <AllProvidersUsage />
        </section>

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
              <div className="mb-2 text-xs text-gray-500">
                Backend URL: <span className="font-mono">{MINIMAX_API_BASE_URL}</span>
              </div>
              <div className="grid gap-2 md:grid-cols-[1fr_auto_auto]">
                <input
                  aria-label={`MiniMax API key for ${machine.hostname}`}
                  type="text"
                  value={minimaxDrafts[machine.machineId]?.apiKey ?? machine.minimaxBackend?.apiKey ?? ""}
                  onChange={(e) => {
                    const nextApiKey = e.target.value;
                    setMinimaxDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: nextApiKey,
                      },
                    }));
                    setMinimaxSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: false,
                    }));
                  }}
                  placeholder="MiniMax API key"
                  className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-950"
                />
                <button
                  aria-label={`Save MiniMax API key for ${machine.hostname}`}
                  onClick={() => {
                    const draft = minimaxDrafts[machine.machineId];
                    const apiKey = draft?.apiKey ?? machine.minimaxBackend?.apiKey ?? "";
                    const trimmedApiKey = apiKey.trim();
                    setMachineMiniMaxConfig(
                      machine.machineId,
                      trimmedApiKey.length > 0 ? trimmedApiKey : undefined,
                      false,
                    );
                    setMinimaxDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: trimmedApiKey,
                      },
                    }));
                    setMinimaxSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: true,
                    }));
                  }}
                  className="rounded bg-cyan-600 px-3 py-2 text-sm text-white hover:bg-cyan-700"
                >
                  {minimaxSaved[machine.machineId] ? "Saved" : "Save"}
                </button>
                <button
                  aria-label={`Clear MiniMax API key for ${machine.hostname}`}
                  onClick={() => {
                    setMachineMiniMaxConfig(machine.machineId, undefined, true);
                    setMinimaxDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: "",
                      },
                    }));
                    setMinimaxSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: false,
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

            <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <div className="mb-2 text-sm font-medium">GLM Backend (Claude Runtime)</div>
              <div className="mb-2 text-xs text-gray-500">
                Backend URL: <span className="font-mono">{GLM_API_BASE_URL}</span>
              </div>
              <div className="grid gap-2 md:grid-cols-[1fr_auto_auto]">
                <input
                  aria-label={`GLM API key for ${machine.hostname}`}
                  type="text"
                  value={glmDrafts[machine.machineId]?.apiKey ?? machine.glmBackend?.apiKey ?? ""}
                  onChange={(e) => {
                    const nextApiKey = e.target.value;
                    setGlmDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: nextApiKey,
                      },
                    }));
                    setGlmSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: false,
                    }));
                  }}
                  placeholder="GLM API key"
                  className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-950"
                />
                <button
                  aria-label={`Save GLM API key for ${machine.hostname}`}
                  onClick={() => {
                    const draft = glmDrafts[machine.machineId];
                    const apiKey = draft?.apiKey ?? machine.glmBackend?.apiKey ?? "";
                    const trimmedApiKey = apiKey.trim();
                    setMachineGlmConfig(
                      machine.machineId,
                      trimmedApiKey.length > 0 ? trimmedApiKey : undefined,
                      false,
                    );
                    setGlmDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: trimmedApiKey,
                      },
                    }));
                    setGlmSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: true,
                    }));
                  }}
                  className="rounded bg-emerald-600 px-3 py-2 text-sm text-white hover:bg-emerald-700"
                >
                  {glmSaved[machine.machineId] ? "Saved" : "Save"}
                </button>
                <button
                  aria-label={`Clear GLM API key for ${machine.hostname}`}
                  onClick={() => {
                    setMachineGlmConfig(machine.machineId, undefined, true);
                    setGlmDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: "",
                      },
                    }));
                    setGlmSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: false,
                    }));
                  }}
                  className="rounded border border-gray-300 px-3 py-2 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
                >
                  Clear Key
                </button>
              </div>
              <div className="mt-2 text-xs text-gray-500">
                {machine.glmBackend?.apiKeyConfigured ? "API key configured" : "API key not configured"}
              </div>
            </div>

            <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-800">
              <div className="mb-2 text-sm font-medium">DeepSeek Backend (Claude Runtime)</div>
              <div className="mb-2 text-xs text-gray-500">
                Backend URL: <span className="font-mono">{DEEPSEEK_API_BASE_URL}</span>
              </div>
              <div className="grid gap-2 md:grid-cols-[1fr_auto_auto]">
                <input
                  aria-label={`DeepSeek API key for ${machine.hostname}`}
                  type="text"
                  value={deepseekDrafts[machine.machineId]?.apiKey ?? machine.deepseekBackend?.apiKey ?? ""}
                  onChange={(e) => {
                    const nextApiKey = e.target.value;
                    setDeepseekDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: nextApiKey,
                      },
                    }));
                    setDeepseekSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: false,
                    }));
                  }}
                  placeholder="DeepSeek API key"
                  className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-950"
                />
                <button
                  aria-label={`Save DeepSeek API key for ${machine.hostname}`}
                  onClick={() => {
                    const draft = deepseekDrafts[machine.machineId];
                    const apiKey = draft?.apiKey ?? machine.deepseekBackend?.apiKey ?? "";
                    const trimmedApiKey = apiKey.trim();
                    setMachineDeepseekConfig(
                      machine.machineId,
                      trimmedApiKey.length > 0 ? trimmedApiKey : undefined,
                      false,
                    );
                    setDeepseekDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: trimmedApiKey,
                      },
                    }));
                    setDeepseekSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: true,
                    }));
                  }}
                  className="rounded bg-indigo-600 px-3 py-2 text-sm text-white hover:bg-indigo-700"
                >
                  {deepseekSaved[machine.machineId] ? "Saved" : "Save"}
                </button>
                <button
                  aria-label={`Clear DeepSeek API key for ${machine.hostname}`}
                  onClick={() => {
                    setMachineDeepseekConfig(machine.machineId, undefined, true);
                    setDeepseekDrafts((prev) => ({
                      ...prev,
                      [machine.machineId]: {
                        apiKey: "",
                      },
                    }));
                    setDeepseekSaved((prev) => ({
                      ...prev,
                      [machine.machineId]: false,
                    }));
                  }}
                  className="rounded border border-gray-300 px-3 py-2 text-sm hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
                >
                  Clear Key
                </button>
              </div>
              <div className="mt-2 text-xs text-gray-500">
                {machine.deepseekBackend?.apiKeyConfigured ? "API key configured" : "API key not configured"}
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
                      {project.isRunning
                        ? `Running${project.pid ? ` (pid ${project.pid})` : ""}${formatMemory(project.memoryKb)}`
                        : "Stopped"}
                    </span>
                    {project.isRunning ? (
                      <button
                        aria-label={`Stop ${project.name || project.path} on ${machine.hostname}`}
                        onClick={() => stopMachineProjectCli(machine.machineId, project.projectId)}
                        className="inline-flex items-center gap-1 rounded bg-red-600 px-3 py-1.5 text-sm text-white hover:bg-red-700"
                      >
                        <Square className="h-4 w-4" />
                        Stop
                      </button>
                    ) : (
                      <button
                        aria-label={`Start ${project.name || project.path} on ${machine.hostname}`}
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
