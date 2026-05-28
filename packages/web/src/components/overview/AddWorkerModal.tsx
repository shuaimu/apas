"use client";

import { useState } from "react";
import { useStore, type PlanReviewMode } from "@/lib/store";
import { ROLE_TEMPLATES, TEMPLATE_COLOR_CLASSES, type RoleTemplate } from "@/lib/roleTemplates";
import { X } from "lucide-react";

interface AddWorkerModalProps {
  open: boolean;
  onClose: () => void;
}

type ProviderChoice = "claude" | "codex" | "opencode" | "cursor-agent";

const PROVIDERS: Array<{ id: ProviderChoice; label: string; hint: string }> = [
  { id: "claude", label: "Claude", hint: "Anthropic — default" },
  { id: "codex", label: "Codex", hint: "OpenAI Codex CLI" },
  { id: "opencode", label: "OpenCode", hint: "open-source coding agent" },
  { id: "cursor-agent", label: "Cursor", hint: "Cursor background agent" },
];

export function AddWorkerModal({ open, onClose }: AddWorkerModalProps) {
  const addPane = useStore((s) => s.addPane);
  const showToast = useStore((s) => s.showToast);

  const [template, setTemplate] = useState<RoleTemplate | null>(null);
  const [provider, setProvider] = useState<ProviderChoice>("claude");
  const [label, setLabel] = useState("");
  const [isolated, setIsolated] = useState(false);
  const [role, setRole] = useState("");
  const [goal, setGoal] = useState("");
  const [backstory, setBackstory] = useState("");
  const [mode, setMode] = useState<PlanReviewMode>("never");
  const [error, setError] = useState<string | null>(null);

  if (!open) return null;

  const applyTemplate = (t: RoleTemplate) => {
    setTemplate(t);
    setRole(t.role);
    setGoal(t.goal);
    setBackstory(t.backstory);
    setMode(t.planReviewMode);
    if (!label.trim()) setLabel(t.label);
    // Developer is the only template that should default to an isolated
    // worktree — the others mostly read or coordinate.
    if (t.id === "developer") setIsolated(true);
  };

  const reset = () => {
    setTemplate(null);
    setProvider("claude");
    setLabel("");
    setIsolated(false);
    setRole("");
    setGoal("");
    setBackstory("");
    setMode("never");
    setError(null);
  };

  const handleSubmit = () => {
    const cleanLabel = label.trim() || (template ? template.label : undefined);
    const result = addPane(
      provider,
      "interactive",
      cleanLabel,
      undefined,
      undefined,
      isolated || undefined,
      {
        role: role.trim() || undefined,
        goal: goal.trim() || undefined,
        backstory: backstory.trim() || undefined,
        planReviewMode: mode,
      },
      // managed: true — this pane joins the team queue and the Tech
      // Lead can delegate to it. The TabBar `+` path passes managed
      // as false (or omits it) so those panes stay as side chats.
      true,
    );
    if (result.success) {
      showToast(
        template
          ? `Worker added — ${template.label}${isolated ? " (isolated)" : ""}.`
          : "Worker added.",
        "success",
      );
      reset();
      onClose();
    } else {
      setError(result.error || "Failed to add worker");
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
      onClick={() => {
        reset();
        onClose();
      }}
    >
      <div
        className="flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-lg border border-zinc-700 bg-zinc-900 text-zinc-100 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-zinc-700 p-4">
          <h3 className="text-lg font-semibold">Add worker</h3>
          <button
            type="button"
            onClick={() => {
              reset();
              onClose();
            }}
            className="rounded p-1 hover:bg-zinc-800"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex flex-col gap-4 overflow-y-auto p-4">
          <div>
            <p className="mb-2 text-[11px] uppercase tracking-wide text-zinc-500">
              Template
            </p>
            <div className="flex flex-wrap gap-2">
              {ROLE_TEMPLATES.map((t) => (
                <button
                  key={t.id}
                  type="button"
                  onClick={() => applyTemplate(t)}
                  className={`rounded border px-2 py-1 text-xs font-medium transition-colors ${TEMPLATE_COLOR_CLASSES[t.color]} ${
                    template?.id === t.id ? "ring-2 ring-offset-1 ring-offset-zinc-900" : ""
                  }`}
                >
                  <span className="mr-1">{t.glyph}</span>
                  {t.label}
                </button>
              ))}
              <button
                type="button"
                onClick={reset}
                className="rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs font-medium text-zinc-300 transition-colors hover:bg-zinc-700"
              >
                ✕ No template
              </button>
            </div>
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label className="flex flex-col gap-1 text-xs">
              <span className="text-zinc-300">Provider</span>
              <select
                value={provider}
                onChange={(e) => setProvider(e.target.value as ProviderChoice)}
                className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-zinc-100"
              >
                {PROVIDERS.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label} — {p.hint}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex flex-col gap-1 text-xs">
              <span className="text-zinc-300">Label</span>
              <input
                type="text"
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder={template ? template.label : "Worker 1"}
                className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 font-mono text-zinc-100"
              />
            </label>
          </div>

          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={isolated}
              onChange={(e) => setIsolated(e.target.checked)}
              className="rounded border-zinc-700"
            />
            <span className="text-zinc-300">
              Isolated git worktree
              <span className="ml-1 text-zinc-500">
                (separate branch under .apas-worktrees/pane-&lt;id&gt;)
              </span>
            </span>
          </label>

          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">
              Role <span className="text-zinc-500">(short noun)</span>
            </span>
            <input
              type="text"
              value={role}
              onChange={(e) => setRole(e.target.value)}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 font-mono text-zinc-100"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">
              Goal <span className="text-zinc-500">(this worker&apos;s responsibility / scope)</span>
            </span>
            <textarea
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              rows={3}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 font-mono text-zinc-100 resize-y"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">
              Backstory <span className="text-zinc-500">(conventions, constraints, scope)</span>
            </span>
            <textarea
              value={backstory}
              onChange={(e) => setBackstory(e.target.value)}
              rows={6}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 font-mono text-zinc-100 resize-y"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">
              Plan review <span className="text-zinc-500">(gate tools behind approval)</span>
            </span>
            <select
              value={mode}
              onChange={(e) => setMode(e.target.value as PlanReviewMode)}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-zinc-100"
            >
              <option value="never">Never (default — agent runs tools freely)</option>
              <option value="risky_only">Risky only (Write / Edit / Bash / Task gated)</option>
              <option value="always">Always (every tool gated except AskUserQuestion)</option>
            </select>
          </label>

          {error && (
            <div className="rounded border border-red-700 bg-red-900/30 px-3 py-2 text-xs text-red-200">
              {error}
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 border-t border-zinc-700 p-4">
          <button
            type="button"
            onClick={() => {
              reset();
              onClose();
            }}
            className="rounded border border-zinc-600 bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleSubmit}
            className="rounded border border-emerald-700 bg-emerald-700 px-3 py-1.5 text-sm font-medium hover:bg-emerald-600"
          >
            Add worker
          </button>
        </div>
      </div>
    </div>
  );
}
