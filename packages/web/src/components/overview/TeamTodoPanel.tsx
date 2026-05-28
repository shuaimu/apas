"use client";

/**
 * Tech-Lead-driven workflow — overview panel for `team-todo.md`.
 * See docs/todo-driven-workflow.md for the protocol.
 *
 * Top section: Global TODOs with status badges. Items in `proposed`
 * state get inline Approve / Reject buttons. Below: per-worker
 * sections with their subtasks and statuses.
 *
 * Polls `fetchTeamTodo()` every 10s and on mount so the view stays
 * fresh without waiting for the Tech Lead's next iteration to push.
 */
import { useEffect, useState } from "react";
import { Check, X, Loader2, Plus } from "lucide-react";
import {
  TeamTodoGlobal,
  TeamTodoSubtask,
  TeamTodoWorker,
  useStore,
} from "@/lib/store";

const POLL_MS = 10_000;

export function TeamTodoPanel() {
  const sessionId = useStore((s) => s.sessionId);
  const state = useStore((s) => s.teamTodoState);
  const fetchTeamTodo = useStore((s) => s.fetchTeamTodo);

  useEffect(() => {
    if (!sessionId) return;
    fetchTeamTodo();
    const id = setInterval(fetchTeamTodo, POLL_MS);
    return () => clearInterval(id);
  }, [sessionId, fetchTeamTodo]);

  if (!sessionId) return null;

  const empty = !state || (state.globals.length === 0 && state.workers.length === 0);
  return (
    <section className="mb-6 rounded border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-900">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
          Team TODO
        </h3>
        <AddTodoControl />
      </div>

      {empty && (
        <p className="text-xs text-gray-500 dark:text-gray-400">
          No TODOs yet. Click + Add TODO above, or ask the Manager to add one,
          or wait for the Tech Lead to propose one based on the project goal.
        </p>
      )}

      {!empty && state && state.globals.length === 0 ? (
        <p className="mb-3 text-xs text-gray-500 dark:text-gray-400">
          No global TODOs yet.
        </p>
      ) : !empty && state ? (
        <ul className="mb-3 space-y-2">
          {state.globals.map((g) => (
            <GlobalRow key={g.id} g={g} />
          ))}
        </ul>
      ) : null}

      {!empty && state && state.workers.length > 0 && (
        <div className="border-t border-gray-200 pt-3 dark:border-gray-700">
          <p className="mb-1 text-[11px] font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">
            Worker queues
          </p>
          <ul className="space-y-3">
            {state.workers.map((w) => (
              <WorkerRow key={w.pane_id} w={w} />
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

function AddTodoControl() {
  const addTodo = useStore((s) => s.addTodo);
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");

  const submit = () => {
    addTodo(title, body);
    setTitle("");
    setBody("");
    setOpen(false);
  };

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="flex items-center gap-1 rounded border border-blue-500 bg-blue-600 px-2 py-0.5 text-[11px] font-medium text-white hover:bg-blue-500"
      >
        <Plus className="h-3 w-3" /> Add TODO
      </button>
    );
  }

  return (
    <div className="flex w-full max-w-md flex-col gap-1">
      <input
        autoFocus
        type="text"
        placeholder="Short title — what should the team do?"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit();
          if (e.key === "Escape") setOpen(false);
        }}
        className="w-full rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
      />
      <textarea
        placeholder="Optional body — context, acceptance criteria, …"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        rows={3}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit();
          if (e.key === "Escape") setOpen(false);
        }}
        className="w-full rounded border border-gray-300 px-2 py-1 text-xs dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
      />
      <div className="flex justify-end gap-1">
        <button
          type="button"
          onClick={() => setOpen(false)}
          className="rounded border border-gray-300 px-2 py-0.5 text-[11px] text-gray-600 hover:bg-gray-100 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={submit}
          disabled={!title.trim()}
          className="rounded border border-blue-500 bg-blue-600 px-2 py-0.5 text-[11px] text-white hover:bg-blue-500 disabled:opacity-40"
          title="⌘↩ / Ctrl+↩ to submit"
        >
          Add
        </button>
      </div>
    </div>
  );
}

function GlobalRow({ g }: { g: TeamTodoGlobal }) {
  const approveTodo = useStore((s) => s.approveTodo);
  const rejectTodo = useStore((s) => s.rejectTodo);

  const isProposed = g.status === "proposed";

  return (
    <li className="rounded border border-gray-200 bg-gray-50 p-2 dark:border-gray-700 dark:bg-gray-800/40">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <code className="text-[10px] text-gray-500 dark:text-gray-400">{g.id}</code>
            <StatusBadge status={g.status} />
            <OriginBadge origin={g.origin} />
            {g.prs?.map((pr, i) => (
              <a
                key={i}
                href={pr.url}
                target="_blank"
                rel="noopener noreferrer"
                className="text-[11px] text-blue-600 hover:underline dark:text-blue-400"
                title={pr.url}
              >
                PR{pr.pane_id ? ` (pane ${pr.pane_id})` : ""} ↗
              </a>
            ))}
          </div>
          <p className="mt-0.5 text-sm font-medium text-gray-900 dark:text-gray-100">
            {g.title}
          </p>
          {g.body.trim() && (
            <p className="mt-1 whitespace-pre-wrap text-xs text-gray-600 dark:text-gray-300">
              {g.body}
            </p>
          )}
        </div>
        {isProposed && (
          <div className="flex shrink-0 flex-col gap-1">
            <button
              type="button"
              onClick={() => approveTodo(g.id)}
              title="Approve — Tech Lead will start dispatching subtasks"
              className="flex items-center gap-1 rounded border border-emerald-500 bg-emerald-600 px-2 py-0.5 text-[11px] font-medium text-white hover:bg-emerald-500"
            >
              <Check className="h-3 w-3" /> Approve
            </button>
            <button
              type="button"
              onClick={() => rejectTodo(g.id)}
              title="Reject"
              className="flex items-center gap-1 rounded border border-rose-500 bg-rose-600 px-2 py-0.5 text-[11px] font-medium text-white hover:bg-rose-500"
            >
              <X className="h-3 w-3" /> Reject
            </button>
          </div>
        )}
      </div>
    </li>
  );
}

function WorkerRow({ w }: { w: TeamTodoWorker }) {
  return (
    <li>
      <p className="text-xs font-medium text-gray-700 dark:text-gray-200">
        pane {w.pane_id}
        {w.role_hint ? (
          <span className="ml-1 text-gray-500 dark:text-gray-400">
            — {w.role_hint}
          </span>
        ) : null}
      </p>
      {w.subtasks.length === 0 ? (
        <p className="ml-2 text-[11px] italic text-gray-400 dark:text-gray-500">
          (no subtasks)
        </p>
      ) : (
        <ul className="ml-2 mt-1 space-y-1">
          {w.subtasks.map((s) => (
            <SubtaskRow key={s.id} s={s} />
          ))}
        </ul>
      )}
    </li>
  );
}

function SubtaskRow({ s }: { s: TeamTodoSubtask }) {
  const inFlight = s.status === "in_progress" || s.status === "revising";
  return (
    <li className="flex items-start gap-2">
      {inFlight ? (
        <Loader2 className="mt-0.5 h-3 w-3 animate-spin text-blue-500" />
      ) : (
        <SubStatusDot status={s.status} />
      )}
      <div className="min-w-0 flex-1">
        <p className="text-xs text-gray-800 dark:text-gray-200">
          {s.title}{" "}
          <span className="text-[10px] text-gray-400 dark:text-gray-500">
            ({s.status} · {s.parent})
          </span>
        </p>
      </div>
    </li>
  );
}

function StatusBadge({ status }: { status: string }) {
  const tone = (() => {
    switch (status) {
      case "proposed":
        return "bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300";
      case "approved":
        return "bg-blue-100 text-blue-800 dark:bg-blue-950/40 dark:text-blue-300";
      case "in_progress":
        return "bg-violet-100 text-violet-800 dark:bg-violet-950/40 dark:text-violet-300";
      case "under_review":
        return "bg-yellow-100 text-yellow-800 dark:bg-yellow-950/40 dark:text-yellow-300";
      case "pr_open":
        return "bg-indigo-100 text-indigo-800 dark:bg-indigo-950/40 dark:text-indigo-300";
      case "done":
        return "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300";
      case "rejected":
        return "bg-rose-100 text-rose-800 dark:bg-rose-950/40 dark:text-rose-300";
      default:
        return "bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300";
    }
  })();
  return (
    <span className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${tone}`}>
      {status}
    </span>
  );
}

function OriginBadge({ origin }: { origin: string }) {
  return (
    <span className="rounded border border-gray-300 px-1.5 py-0.5 text-[10px] text-gray-600 dark:border-gray-600 dark:text-gray-400">
      {origin}
    </span>
  );
}

function SubStatusDot({ status }: { status: string }) {
  const color = (() => {
    switch (status) {
      case "pending":
        return "bg-gray-400";
      case "done":
        return "bg-emerald-500";
      case "reviewing":
        return "bg-yellow-500";
      case "approved":
        return "bg-emerald-700";
      default:
        return "bg-gray-300";
    }
  })();
  return <span className={`mt-1.5 h-2 w-2 rounded-full ${color}`} />;
}
