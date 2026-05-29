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
import { useEffect, useMemo, useState } from "react";
import { Check, X, Loader2, Plus } from "lucide-react";
import {
  TeamTodoGlobal,
  TeamTodoSubtask,
  TeamTodoWorker,
  paneKey,
  useStore,
} from "@/lib/store";

const POLL_MS = 10_000;

/// Parsed form of a `pr: <pane> <url>` line in team-todo.md. `null`
/// when the line is malformed (not a GitHub PR URL we can poll).
export interface ParsedPrLine {
  pane: number;
  url: string;
  owner: string;
  repo: string;
  num: number;
}

/// Pull `pr: <pane> <url>` apart into the components needed to drive
/// the GitHub Pulls API call. Returns null on any malformed input so
/// callers can ignore the line without crashing.
export function parsePrLine(line: string): ParsedPrLine | null {
  const m = line.match(
    /^pr:\s+(\d+)\s+(https:\/\/github\.com\/([^/\s]+)\/([^/\s]+)\/pull\/(\d+))\s*$/,
  );
  if (!m) return null;
  const pane = Number.parseInt(m[1], 10);
  const num = Number.parseInt(m[5], 10);
  if (!Number.isFinite(pane) || !Number.isFinite(num)) return null;
  return { pane, url: m[2], owner: m[3], repo: m[4], num };
}

export function TeamTodoPanel() {
  const sessionId = useStore((s) => s.sessionId);
  const state = useStore((s) =>
    s.sessionId ? s.teamTodoStates.get(s.sessionId) ?? null : null,
  );
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

      <AgentStatusRow
        techLeadCursor={state?.tech_lead_cursor ?? null}
        reviewerCursor={state?.reviewer_cursor ?? null}
      />

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
        (() => {
          const active = state.globals.filter(
            (g) => g.status !== "rejected" && g.status !== "done",
          );
          const done = state.globals.filter((g) => g.status === "done");
          const rejected = state.globals.filter((g) => g.status === "rejected");
          return (
            <>
              {active.length > 0 && (
                <ul className="mb-3 space-y-2">
                  {active.map((g) => (
                    <GlobalRow key={g.id} g={g} />
                  ))}
                </ul>
              )}
              {done.length > 0 && (
                <CollapsedFolder
                  label="Done"
                  entries={done}
                  variant="done"
                />
              )}
              {rejected.length > 0 && (
                <CollapsedFolder
                  label="Rejected"
                  entries={rejected}
                  variant="rejected"
                />
              )}
            </>
          );
        })()
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

/// Bar showing Tech Lead + Reviewer status. "Last active" derives from
/// the most-recent message in each pane (via paneMessages); "cursor"
/// is the timestamp piped through from the cursor file by the CLI.
/// Both numbers tick once a minute via the `now` state below so the
/// "Xm ago" labels stay accurate without per-second re-renders.
function AgentStatusRow({
  techLeadCursor,
  reviewerCursor,
}: {
  techLeadCursor: string | null;
  reviewerCursor: string | null;
}) {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const paneMessages = useStore((s) => s.paneMessages);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);

  const techLead = useMemo(
    () =>
      paneConfigs.find((p) =>
        (p.role ?? "").toLowerCase().includes("tech lead"),
      ),
    [paneConfigs],
  );
  const reviewer = useMemo(
    () =>
      paneConfigs.find(
        (p) =>
          (p.role ?? "").toLowerCase().includes("reviewer") &&
          !(p.role ?? "").toLowerCase().includes("tech lead"),
      ),
    [paneConfigs],
  );

  const techLeadLast = lastActivityTs(techLead?.pane_id, paneMessages);
  const reviewerLast = lastActivityTs(reviewer?.pane_id, paneMessages);

  return (
    <div className="mb-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-gray-500 dark:text-gray-400">
      <AgentLine
        label="Tech Lead"
        present={!!techLead}
        lastActivity={techLeadLast}
        cursor={techLeadCursor}
        now={now}
      />
      <AgentLine
        label="Reviewer"
        present={!!reviewer}
        lastActivity={reviewerLast}
        cursor={reviewerCursor}
        now={now}
      />
    </div>
  );
}

function AgentLine({
  label,
  present,
  lastActivity,
  cursor,
  now,
}: {
  label: string;
  present: boolean;
  lastActivity: number | null;
  cursor: string | null;
  now: number;
}) {
  if (!present) {
    return (
      <span>
        <strong className="text-gray-700 dark:text-gray-200">{label}</strong>:{" "}
        <span className="text-gray-400">not running</span>
      </span>
    );
  }
  const dot = activityDot(lastActivity, now);
  return (
    <span title={cursor ? `cursor: ${cursor}` : "no cursor (agent hasn't iterated)"}>
      <strong className="text-gray-700 dark:text-gray-200">{label}</strong>{" "}
      {dot} {relative(lastActivity, now)}
      {cursor && (
        <span className="ml-1 text-gray-400 dark:text-gray-500">
          · cursor {relative(parseTs(cursor), now)}
        </span>
      )}
    </span>
  );
}

function activityDot(ts: number | null, now: number): string {
  if (ts == null) return "○";
  const ageMs = now - ts;
  if (ageMs < 5 * 60_000) return "🟢";   // active in last 5 min
  if (ageMs < 30 * 60_000) return "🟡";  // idle but recent
  return "🔴";                            // stale
}

function relative(ts: number | null, now: number): string {
  if (ts == null) return "—";
  const ageMs = Math.max(0, now - ts);
  const s = Math.round(ageMs / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

function parseTs(s: string): number | null {
  const n = Date.parse(s);
  return Number.isFinite(n) ? n : null;
}

function lastActivityTs(
  paneId: number | undefined,
  paneMessages: Record<string, { timestamp: Date }[]>,
): number | null {
  if (paneId == null) return null;
  const msgs = paneMessages[paneKey(paneId)];
  if (!msgs || msgs.length === 0) return null;
  return msgs[msgs.length - 1].timestamp.getTime();
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

/// Done + Rejected TODOs are both "no longer actionable" history.
/// Each gets folded under a single expandable summary at the bottom
/// of the active list, with each entry as a compact one-liner — no
/// body, no Approve/Reject buttons. Done entries also surface their
/// PR links (typically the merged PR) so the user can click through
/// to the diff. The `variant` knob tints the title accordingly.
function CollapsedFolder({
  label,
  entries,
  variant,
}: {
  label: string;
  entries: TeamTodoGlobal[];
  variant: "done" | "rejected";
}) {
  const titleClass =
    variant === "done"
      ? "truncate text-xs text-gray-600 dark:text-gray-400"
      : "truncate text-xs text-gray-700 line-through decoration-gray-400 dark:text-gray-300";
  return (
    <details className="mb-3 rounded border border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-800/40">
      <summary className="cursor-pointer select-none px-2 py-1 text-[11px] text-gray-600 hover:text-gray-900 dark:text-gray-400 dark:hover:text-gray-100">
        {label} ({entries.length})
      </summary>
      <ul className="divide-y divide-gray-200 dark:divide-gray-700">
        {entries.map((g) => (
          <li key={g.id} className="flex items-baseline gap-2 px-2 py-1">
            <code className="text-[10px] text-gray-500 dark:text-gray-400">
              {g.id}
            </code>
            <span className={titleClass}>{g.title}</span>
            {variant === "done" && g.prs?.length > 0 && (
              <span className="ml-1 flex shrink-0 items-baseline gap-1">
                {g.prs.map((pr, i) => (
                  <a
                    key={i}
                    href={pr.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-[10px] text-blue-600 hover:underline dark:text-blue-400"
                    title={pr.url}
                  >
                    PR{pr.pane_id ? ` (pane ${pr.pane_id})` : ""} ↗
                  </a>
                ))}
              </span>
            )}
            <span className="ml-auto shrink-0 text-[10px] text-gray-500 dark:text-gray-400">
              {g.origin}
            </span>
          </li>
        ))}
      </ul>
    </details>
  );
}

function GlobalRow({ g }: { g: TeamTodoGlobal }) {
  const approveTodo = useStore((s) => s.approveTodo);
  const rejectTodo = useStore((s) => s.rejectTodo);

  const isProposed = g.status === "proposed";

  /// Dedupe by URL — a Global can list the same PR twice (e.g. when
  /// multiple workers contributed) and we only want one badge per URL.
  /// Drop entries whose URL doesn't parse as a real GitHub PR.
  const uniquePrs = useMemo(() => {
    const seen = new Set<string>();
    const out: { pane_id: number; parsed: ParsedPrLine }[] = [];
    for (const pr of g.prs ?? []) {
      if (seen.has(pr.url)) continue;
      const parsed = parsePrLine(`pr: ${pr.pane_id} ${pr.url}`);
      if (!parsed) continue;
      seen.add(pr.url);
      out.push({ pane_id: pr.pane_id, parsed });
    }
    return out;
  }, [g.prs]);

  return (
    <li className="rounded border border-gray-200 bg-gray-50 p-2 dark:border-gray-700 dark:bg-gray-800/40">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <code className="text-[10px] text-gray-500 dark:text-gray-400">{g.id}</code>
            <StatusBadge status={g.status} />
            <OriginBadge origin={g.origin} />
            {uniquePrs.map(({ pane_id, parsed }) => (
              <PrLink
                key={parsed.url}
                paneId={pane_id}
                parsed={parsed}
                globalStatus={g.status}
              />
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

/// `loading` while the GitHub fetch is in flight; `done` when the
/// Global is already status:done so we skip the fetch entirely (the
/// PR landed and re-polling merged PRs forever is wasteful).
type PrFetchState =
  | { kind: "loading" }
  | { kind: "open" }
  | { kind: "merged" }
  | { kind: "closed" }
  | { kind: "error" }
  | { kind: "done" };

function PrLink({
  paneId,
  parsed,
  globalStatus,
}: {
  paneId: number;
  parsed: ParsedPrLine;
  globalStatus: string;
}) {
  const skipFetch = globalStatus === "done";
  const [state, setState] = useState<PrFetchState>(
    skipFetch ? { kind: "done" } : { kind: "loading" },
  );

  useEffect(() => {
    if (skipFetch) return;
    let cancelled = false;
    const url = `https://api.github.com/repos/${parsed.owner}/${parsed.repo}/pulls/${parsed.num}`;
    fetch(url)
      .then(async (r) => {
        if (r.status === 403 || r.status === 429) {
          throw new Error("rate-limit");
        }
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((j) => {
        if (cancelled) return;
        if (j && j.merged === true) setState({ kind: "merged" });
        else if (j && j.state === "open") setState({ kind: "open" });
        else if (j && j.state === "closed") setState({ kind: "closed" });
        else setState({ kind: "error" });
      })
      .catch(() => {
        if (!cancelled) setState({ kind: "error" });
      });
    return () => {
      cancelled = true;
    };
  }, [skipFetch, parsed.owner, parsed.repo, parsed.num]);

  return (
    <span className="inline-flex items-center gap-1">
      <a
        href={parsed.url}
        target="_blank"
        rel="noopener noreferrer"
        className="text-[11px] text-blue-600 hover:underline dark:text-blue-400"
        title={parsed.url}
      >
        PR #{parsed.num}{paneId ? ` (pane ${paneId})` : ""} ↗
      </a>
      <PrStateBadge state={state} />
    </span>
  );
}

function PrStateBadge({ state }: { state: PrFetchState }) {
  const { label, tone } = (() => {
    switch (state.kind) {
      case "loading":
        return {
          label: "…",
          tone: "bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400",
        };
      case "open":
        return {
          label: "OPEN",
          tone: "bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300",
        };
      case "merged":
        return {
          label: "MERGED",
          tone: "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300",
        };
      case "closed":
        return {
          label: "CLOSED",
          tone: "bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300",
        };
      case "error":
        return {
          label: "—",
          tone: "bg-rose-100 text-rose-800 dark:bg-rose-950/40 dark:text-rose-300",
        };
      case "done":
        return {
          label: "MERGED",
          tone: "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300",
        };
    }
  })();
  return (
    <span
      data-testid="pr-state-badge"
      data-pr-state={state.kind}
      className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${tone}`}
    >
      {label}
    </span>
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
