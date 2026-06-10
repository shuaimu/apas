"use client";

import { useMemo, useState } from "react";
import { Play } from "lucide-react";
import { useStore, type PaneConfig } from "@/lib/store";

/**
 * Pre-launch team setup. Renders 4 role rows (Manager, Tech Lead,
 * Reviewer, Developer) with an agent (provider × model) dropdown per
 * row and a single "Start team" button at the right. Hides itself
 * once any of those roles already has a managed pane — the live
 * PaneGrid takes over from there.
 *
 * Replaces the v3.4-era pre-start StartTeamBanner: the user wanted to
 * pick the model BEFORE spawning, not after, because respawning a
 * pane with a fresh provider loses the chat-history context.
 */

const MINIMAX_DEFAULT_MODEL = "MiniMax-M2.7";
const GLM_DEFAULT_MODEL = "glm-5.1";
const DEEPSEEK_DEFAULT_MODEL = "deepseek-v4-pro";

const AGENT_OPTS: ReadonlyArray<{
  value: string;
  label: string;
  provider: string;
  model: string | null;
}> = [
  { value: "claude/official", label: "Claude / Official", provider: "claude", model: null },
  // Official Anthropic backend pinned to the Fable 5 model. Unlike the
  // pane-level switchers there's no separate model knob here, so Fable
  // gets its own agent option.
  { value: "claude/fable", label: "Claude / Fable", provider: "claude", model: "claude-fable-5" },
  { value: "claude/minimax", label: "Claude / MiniMax 2.7", provider: "claude", model: MINIMAX_DEFAULT_MODEL },
  { value: "claude/glm", label: "Claude / GLM 5.1", provider: "claude", model: GLM_DEFAULT_MODEL },
  { value: "claude/deepseek", label: "Claude / DeepSeek", provider: "claude", model: DEEPSEEK_DEFAULT_MODEL },
  { value: "codex/official", label: "Codex / Official", provider: "codex", model: null },
  { value: "opencode/official", label: "OpenCode / Official", provider: "opencode", model: null },
  { value: "cursor-agent/official", label: "Cursor / Official", provider: "cursor-agent", model: null },
];

type RoleKey = "manager" | "techLead" | "reviewer" | "developer";

const ROLES: ReadonlyArray<{
  key: RoleKey;
  label: string;
  blurb: string;
}> = [
  { key: "manager", label: "Manager", blurb: "User-facing chat. Owns project_goal.md and team-todo intake." },
  { key: "techLead", label: "Tech Lead", blurb: "Autonomous orchestrator. Expands TODOs and dispatches workers." },
  { key: "reviewer", label: "Reviewer", blurb: "Reads worker diffs and approves / rejects each TODO." },
  { key: "developer", label: "Developer", blurb: "Default worker. Picks up dispatched subtasks and writes code." },
];

function isTeamRole(p: PaneConfig): boolean {
  if (!p.managed) return false;
  const lower = (p.role ?? "").toLowerCase();
  return (
    lower.includes("manager") ||
    lower.includes("tech lead") ||
    lower.includes("reviewer") ||
    lower.includes("developer")
  );
}

export function TeamSetupCard() {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const startTeam = useStore((s) => s.startTeam);

  const [picks, setPicks] = useState<Record<RoleKey, string>>({
    manager: "claude/official",
    techLead: "claude/official",
    reviewer: "claude/official",
    developer: "claude/official",
  });

  const teamPresent = useMemo(() => paneConfigs.some(isTeamRole), [paneConfigs]);
  if (teamPresent) return null;

  const optByValue = (v: string) =>
    AGENT_OPTS.find((o) => o.value === v) ?? AGENT_OPTS[0];

  const handleStart = () => {
    const m = optByValue(picks.manager);
    const t = optByValue(picks.techLead);
    const r = optByValue(picks.reviewer);
    const d = optByValue(picks.developer);
    startTeam({
      manager: { provider: m.provider, model: m.model },
      techLead: { provider: t.provider, model: t.model },
      reviewer: { provider: r.provider, model: r.model },
      developer: { provider: d.provider, model: d.model },
    });
  };

  return (
    <div className="mb-4 rounded-lg border border-indigo-200 bg-indigo-50/60 dark:border-indigo-700/40 dark:bg-indigo-900/15">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-indigo-200 dark:border-indigo-700/40 px-4 py-2.5">
        <div>
          <div className="text-sm font-semibold text-indigo-900 dark:text-indigo-200">
            Team setup
          </div>
          <p className="mt-0.5 text-xs text-indigo-800/80 dark:text-indigo-300/80">
            Pick an agent per role, then click Start team. Nothing runs
            until you do.
          </p>
        </div>
        <button
          onClick={handleStart}
          className="inline-flex items-center justify-center gap-1.5 rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white shadow-sm transition hover:bg-indigo-700 dark:bg-indigo-500 dark:hover:bg-indigo-400"
        >
          <Play className="h-3.5 w-3.5" />
          Start team
        </button>
      </div>
      <div className="divide-y divide-indigo-200/70 dark:divide-indigo-700/30">
        {ROLES.map((r) => (
          <div
            key={r.key}
            className="flex flex-col gap-1.5 px-4 py-2.5 sm:flex-row sm:items-center sm:gap-3"
          >
            <div className="w-32 flex-shrink-0">
              <div className="text-sm font-medium text-gray-800 dark:text-gray-100">
                {r.label}
              </div>
              <div
                className="text-xs text-gray-500 dark:text-gray-400"
                title={r.blurb}
              >
                {r.blurb}
              </div>
            </div>
            <select
              value={picks[r.key]}
              onChange={(e) =>
                setPicks((prev) => ({ ...prev, [r.key]: e.target.value }))
              }
              className="rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-2 py-0.5 text-xs font-mono text-gray-700 dark:text-gray-300 focus:outline-none focus:ring-1 focus:ring-indigo-400"
              title="Agent frontend × API backend"
            >
              {AGENT_OPTS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
            <span className="ml-auto inline-flex items-center rounded-full bg-gray-200 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-gray-600 dark:bg-gray-700 dark:text-gray-300">
              Not created
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
