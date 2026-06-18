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
import {
  Check,
  X,
  Loader2,
  Plus,
  ChevronDown,
  ChevronRight,
  Search,
} from "lucide-react";
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
    /^pr:\s+(\d+)\s+(https:\/\/github\.com\/([^/\s]+)\/([^/\s]+)\/pull\/(\d+))(?:\s+\([^)]*\))?\s*$/,
  );
  if (!m) return null;
  const pane = Number.parseInt(m[1], 10);
  const num = Number.parseInt(m[5], 10);
  if (!Number.isFinite(pane) || !Number.isFinite(num)) return null;
  return { pane, url: m[2], owner: m[3], repo: m[4], num };
}

/// localStorage key for the user's preferred fold state. Global (not
/// per-project) so the user only sets it once. Default = expanded.
const COLLAPSED_KEY = "apas_team_todo_collapsed";

export function TeamTodoPanel() {
  const sessionId = useStore((s) => s.sessionId);
  const state = useStore((s) =>
    s.sessionId ? s.teamTodoStates.get(s.sessionId) ?? null : null,
  );
  const fetchTeamTodo = useStore((s) => s.fetchTeamTodo);
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return window.localStorage.getItem(COLLAPSED_KEY) === "1";
  });
  const [searchQuery, setSearchQuery] = useState("");
  const normalizedSearchQuery = searchQuery.trim().toLowerCase();
  const visibleGlobals = useMemo(() => {
    if (!state) return [];
    if (!normalizedSearchQuery) return state.globals;
    return state.globals.filter((g) =>
      globalMatchesSearch(g, normalizedSearchQuery),
    );
  }, [state, normalizedSearchQuery]);
  const toggleCollapsed = () => {
    setCollapsed((c) => {
      const next = !c;
      if (typeof window !== "undefined") {
        window.localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0");
      }
      return next;
    });
  };

  useEffect(() => {
    if (!sessionId) return;
    fetchTeamTodo();
    const id = setInterval(fetchTeamTodo, POLL_MS);
    return () => clearInterval(id);
  }, [sessionId, fetchTeamTodo]);

  if (!sessionId) return null;

  const empty = !state || (state.globals.length === 0 && state.workers.length === 0);

  // Counts for the collapsed header summary.
  const activeCount = state
    ? state.globals.filter(
        (g) =>
          g.status !== "rejected" &&
          g.status !== "done" &&
          g.status !== "withdrawn",
      ).length
    : 0;
  const doneCount = state
    ? state.globals.filter((g) => g.status === "done").length
    : 0;
  const totalSubtasks = state
    ? state.workers.reduce((acc, w) => acc + w.subtasks.length, 0)
    : 0;

  return (
    <section className="mb-6 rounded border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-900">
      <div className="mb-2 flex items-center justify-between gap-2">
        <button
          type="button"
          onClick={toggleCollapsed}
          className="flex items-center gap-1.5 text-sm font-semibold text-gray-900 hover:text-indigo-600 dark:text-gray-100 dark:hover:text-indigo-400"
          title={collapsed ? "Expand Team TODO" : "Collapse Team TODO"}
        >
          {collapsed ? (
            <ChevronRight className="h-4 w-4" />
          ) : (
            <ChevronDown className="h-4 w-4" />
          )}
          Team TODO
          {collapsed && (
            <span className="ml-1 text-xs font-normal text-gray-500 dark:text-gray-400">
              {activeCount} active · {doneCount} done
              {totalSubtasks > 0 ? ` · ${totalSubtasks} subtasks` : ""}
            </span>
          )}
        </button>
        {!collapsed && <AddTodoControl />}
      </div>

      {collapsed ? null : (
        <>

      <AgentStatusRow
        techLeadCursor={state?.tech_lead_cursor ?? null}
        reviewerCursor={state?.reviewer_cursor ?? null}
      />

      {state && state.globals.length > 0 && (
        <TeamTodoSearchControl
          value={searchQuery}
          onChange={setSearchQuery}
          visibleCount={visibleGlobals.length}
          totalCount={state.globals.length}
        />
      )}

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
      ) : !empty && state && state.globals.length > 0 && visibleGlobals.length === 0 ? (
        <p className="mb-3 rounded border border-dashed border-gray-300 px-3 py-2 text-xs text-gray-500 dark:border-gray-700 dark:text-gray-400">
          No TODOs match "{searchQuery.trim()}".
        </p>
      ) : !empty && state ? (
        (() => {
          const active = visibleGlobals.filter(
            (g) =>
              g.status !== "rejected" &&
              g.status !== "done" &&
              g.status !== "withdrawn",
          );
          const done = visibleGlobals.filter((g) => g.status === "done");
          const rejected = visibleGlobals.filter((g) => g.status === "rejected");
          const withdrawn = visibleGlobals.filter((g) => g.status === "withdrawn");
          const activeGroups = groupActiveGlobals(active);
          return (
            <>
              {activeGroups.map((group) => (
                <ActiveGlobalGroup
                  key={group.key}
                  label={group.label}
                  entries={group.entries}
                  workers={state.workers}
                />
              ))}
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
              {withdrawn.length > 0 && (
                <CollapsedFolder
                  label="Withdrawn (by Tech Lead)"
                  entries={withdrawn}
                  variant="withdrawn"
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
        </>
      )}
    </section>
  );
}

function TeamTodoSearchControl({
  value,
  onChange,
  visibleCount,
  totalCount,
}: {
  value: string;
  onChange: (value: string) => void;
  visibleCount: number;
  totalCount: number;
}) {
  const active = value.trim().length > 0;
  return (
    <div className="mb-3 flex flex-wrap items-center gap-2">
      <div className="relative min-w-0 flex-1 sm:max-w-sm">
        <Search
          className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-400"
          aria-hidden="true"
        />
        <input
          type="search"
          aria-label="Search Team TODOs"
          placeholder="Search TODOs"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="h-8 w-full rounded border border-gray-300 bg-white px-7 py-1 text-xs text-gray-900 placeholder:text-gray-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
        />
        {value && (
          <button
            type="button"
            aria-label="Clear Team TODO search"
            onClick={() => onChange("")}
            className="absolute right-1.5 top-1/2 flex h-5 w-5 -translate-y-1/2 items-center justify-center rounded text-gray-400 hover:bg-gray-100 hover:text-gray-700 dark:hover:bg-gray-700 dark:hover:text-gray-200"
          >
            <X className="h-3 w-3" />
          </button>
        )}
      </div>
      <span className="text-[11px] text-gray-500 dark:text-gray-400">
        {active ? `${visibleCount} of ${totalCount}` : `${totalCount}`} TODOs
      </span>
    </div>
  );
}

function globalMatchesSearch(
  global: TeamTodoGlobal,
  normalizedQuery: string,
): boolean {
  const fields = [
    global.id,
    global.title,
    global.body,
    global.status,
    global.status.replace(/_/g, " "),
    global.origin,
  ];
  for (const pr of global.prs ?? []) {
    const parsed = parsePrLine(`pr: ${pr.pane_id} ${pr.url}`);
    fields.push(pr.url, String(pr.pane_id));
    if (parsed) {
      fields.push(String(parsed.num), `#${parsed.num}`, `pr #${parsed.num}`);
    }
  }
  return fields.some((field) =>
    field.toLowerCase().includes(normalizedQuery),
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
  const status = activityStatus(lastActivity, now);
  return (
    <span title={cursor ? `cursor: ${cursor}` : "no cursor (agent hasn't iterated)"}>
      <strong className="text-gray-700 dark:text-gray-200">{label}</strong>{" "}
      <ActivityIndicator status={status} /> {relative(lastActivity, now)}
      {cursor && (
        <span className="ml-1 text-gray-400 dark:text-gray-500">
          · cursor {relative(parseTs(cursor), now)}
        </span>
      )}
    </span>
  );
}

type ActivityStatus = "active" | "recent" | "stale" | "unknown";

function activityStatus(ts: number | null, now: number): ActivityStatus {
  if (ts == null) return "unknown";
  const ageMs = now - ts;
  if (ageMs < 5 * 60_000) return "active";
  if (ageMs < 30 * 60_000) return "recent";
  return "stale";
}

function ActivityIndicator({ status }: { status: ActivityStatus }) {
  const { label, tone, dot } = (() => {
    switch (status) {
      case "active":
        return {
          label: "active",
          tone: "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300",
          dot: "bg-emerald-500",
        };
      case "recent":
        return {
          label: "recent",
          tone: "bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300",
          dot: "bg-amber-500",
        };
      case "stale":
        return {
          label: "stale",
          tone: "bg-rose-100 text-rose-800 dark:bg-rose-950/40 dark:text-rose-300",
          dot: "bg-rose-500",
        };
      case "unknown":
        return {
          label: "unknown",
          tone: "bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-300",
          dot: "bg-gray-400",
        };
    }
  })();
  return (
    <span
      aria-label={`Agent status: ${label}`}
      data-agent-status={status}
      className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium ${tone}`}
    >
      <span className={`h-1.5 w-1.5 rounded-full ${dot}`} aria-hidden="true" />
      {label}
    </span>
  );
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

const ACTIVE_GROUP_ORDER = [
  { key: "pr_open", label: "PR open" },
  { key: "under_review", label: "Under review" },
  { key: "in_progress", label: "In progress" },
  { key: "approved", label: "Approved" },
  { key: "proposed", label: "Proposed" },
  { key: "other", label: "Other active" },
] as const;

type ActiveGroupKey = (typeof ACTIVE_GROUP_ORDER)[number]["key"];

function groupActiveGlobals(globals: TeamTodoGlobal[]): Array<{
  key: ActiveGroupKey;
  label: string;
  entries: TeamTodoGlobal[];
}> {
  const byStatus = new Map<ActiveGroupKey, TeamTodoGlobal[]>(
    ACTIVE_GROUP_ORDER.map((group) => [group.key, []]),
  );
  for (const global of globals) {
    const key = ACTIVE_GROUP_ORDER.some((group) => group.key === global.status)
      ? (global.status as ActiveGroupKey)
      : "other";
    byStatus.get(key)?.push(global);
  }
  return ACTIVE_GROUP_ORDER.map((group) => ({
    ...group,
    entries: byStatus.get(group.key) ?? [],
  })).filter((group) => group.entries.length > 0);
}

function ActiveGlobalGroup({
  label,
  entries,
  workers,
}: {
  label: string;
  entries: TeamTodoGlobal[];
  workers: TeamTodoWorker[];
}) {
  return (
    <section className="mb-3">
      <h3 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
        {label} ({entries.length})
      </h3>
      <ul className="space-y-2">
        {entries.map((g) => (
          <GlobalRow key={g.id} g={g} workers={workers} />
        ))}
      </ul>
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

/// Done + Rejected + Withdrawn TODOs are all "no longer actionable"
/// history. Each gets folded under a single expandable summary at the
/// bottom of the active list, with each entry as a compact one-liner
/// — no body, no Approve/Reject buttons. Done entries also surface
/// their PR links (typically the merged PR) so the user can click
/// through to the diff. The `variant` knob tints the title accordingly.
function CollapsedFolder({
  label,
  entries,
  variant,
}: {
  label: string;
  entries: TeamTodoGlobal[];
  variant: "done" | "rejected" | "withdrawn";
}) {
  const titleClass =
    variant === "done"
      ? "truncate text-xs text-gray-600 dark:text-gray-400"
      : variant === "withdrawn"
      ? "truncate text-xs italic text-gray-500 dark:text-gray-500"
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

function GlobalRow({
  g,
  workers,
}: {
  g: TeamTodoGlobal;
  workers: TeamTodoWorker[];
}) {
  const approveTodo = useStore((s) => s.approveTodo);
  const rejectTodo = useStore((s) => s.rejectTodo);

  const isProposed = g.status === "proposed";
  const waitingForPrPanes =
    g.status === "under_review" && (g.prs?.length ?? 0) === 0
      ? workers
          .filter((w) =>
            w.subtasks.some((s) => s.parent === g.id && s.status === "approved"),
          )
          .map((w) => w.pane_id)
      : [];

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
          {waitingForPrPanes.length > 0 && (
            <p
              data-testid="waiting-for-pr-hint"
              className="mt-1 text-[11px] font-medium text-yellow-700 dark:text-yellow-300"
            >
              Reviewer approved; waiting for {formatPaneList(waitingForPrPanes)} to open PR
            </p>
          )}
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

function formatPaneList(panes: number[]): string {
  return panes.length === 1 ? `pane ${panes[0]}` : `panes ${panes.join(", ")}`;
}

/// `loading` while the GitHub fetch is in flight; `done` when the
/// Global is already status:done so we skip the fetch entirely (the
/// PR landed and re-polling merged PRs forever is wasteful).
type PrFetchState =
  | { kind: "loading" }
  | { kind: "open"; readiness?: PrReadiness }
  | { kind: "merged" }
  | { kind: "closed" }
  | { kind: "error" }
  | { kind: "done" };

type PrReadiness =
  | "review_requested"
  | "changes_requested"
  | "checks_pending"
  | "checks_failing"
  | "merge_conflict"
  | "ready";

interface PullRequestJson {
  state?: string;
  merged?: boolean;
  draft?: boolean;
  mergeable?: boolean | null;
  mergeable_state?: string | null;
  requested_reviewers?: unknown[];
  statuses_url?: string;
}

interface CommitStatusJson {
  state?: string;
  context?: string;
}

type CommitStatusState = "success" | "pending" | "failure" | "error";

interface PullReviewJson {
  state?: string;
  submitted_at?: string | null;
}

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
    const load = async () => {
      try {
        const pull = (await fetchJson(url)) as PullRequestJson;
        let statusState: CommitStatusState | undefined;
        let reviewState: "APPROVED" | "CHANGES_REQUESTED" | undefined;
        if (pull.state === "open" && pull.statuses_url) {
          try {
            statusState = commitStatusState(await fetchJson(pull.statuses_url));
          } catch {
            statusState = undefined;
          }
        }
        if (pull.state === "open") {
          try {
            reviewState = latestReviewState(
              await fetchJson(`${url}/reviews`),
            );
          } catch {
            reviewState = undefined;
          }
        }
        if (!cancelled) {
          setState(prFetchStateFromPull(pull, statusState, reviewState));
        }
      } catch {
        if (!cancelled) setState({ kind: "error" });
      }
    };
    load();
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
      <PrReadinessBadge readiness={state.kind === "open" ? state.readiness : undefined} />
    </span>
  );
}

async function fetchJson(url: string): Promise<unknown> {
  const r = await fetch(url);
  if (r.status === 403 || r.status === 429) {
    throw new Error("rate-limit");
  }
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

function prFetchStateFromPull(
  pull: PullRequestJson | null,
  statusState: CommitStatusState | undefined,
  reviewState: "APPROVED" | "CHANGES_REQUESTED" | undefined,
): PrFetchState {
  if (!pull) return { kind: "error" };
  if (pull.merged === true) return { kind: "merged" };
  if (pull.state === "closed") return { kind: "closed" };
  if (pull.state !== "open") return { kind: "error" };
  return { kind: "open", readiness: prReadiness(pull, statusState, reviewState) };
}

function commitStatusState(statusPayload: unknown): CommitStatusState | undefined {
  if (!Array.isArray(statusPayload)) {
    const state = (statusPayload as CommitStatusJson | null)?.state;
    return normalizeCommitStatusState(state);
  }
  const latestByContext = new Map<string, CommitStatusState>();
  statusPayload.forEach((entry, index) => {
    const status = entry as CommitStatusJson | null;
    const state = normalizeCommitStatusState(status?.state);
    if (!state) return;
    const context = status?.context ?? `status-${index}`;
    if (!latestByContext.has(context)) latestByContext.set(context, state);
  });
  const states = Array.from(latestByContext.values());
  if (states.length === 0) return undefined;
  if (states.some((state) => state === "failure" || state === "error")) {
    return "failure";
  }
  if (states.some((state) => state === "pending")) return "pending";
  return "success";
}

function normalizeCommitStatusState(
  state: string | undefined,
): CommitStatusState | undefined {
  const normalized = state?.toLowerCase();
  if (
    normalized === "success" ||
    normalized === "pending" ||
    normalized === "failure" ||
    normalized === "error"
  ) {
    return normalized;
  }
  return undefined;
}

function latestReviewState(
  reviewsPayload: unknown,
): "APPROVED" | "CHANGES_REQUESTED" | undefined {
  if (!Array.isArray(reviewsPayload)) return undefined;
  return [...reviewsPayload]
    .sort((a, b) => {
      const aTs = Date.parse((a as PullReviewJson).submitted_at ?? "");
      const bTs = Date.parse((b as PullReviewJson).submitted_at ?? "");
      return (Number.isFinite(bTs) ? bTs : 0) - (Number.isFinite(aTs) ? aTs : 0);
    })
    .map((review) => ((review as PullReviewJson).state ?? "").toUpperCase())
    .find(
      (state): state is "APPROVED" | "CHANGES_REQUESTED" =>
        state === "APPROVED" || state === "CHANGES_REQUESTED",
    );
}

function prReadiness(
  pull: PullRequestJson,
  checkState: CommitStatusState | undefined,
  reviewState: "APPROVED" | "CHANGES_REQUESTED" | undefined,
): PrReadiness | undefined {
  const mergeState = (pull.mergeable_state ?? "").toString().toLowerCase();

  if (reviewState === "CHANGES_REQUESTED") return "changes_requested";
  if (checkState === "failure" || checkState === "error") return "checks_failing";
  if (checkState === "pending") return "checks_pending";
  if (pull.mergeable === false || mergeState === "dirty") return "merge_conflict";
  if (
    pull.draft === true ||
    (pull.requested_reviewers?.length ?? 0) > 0
  ) {
    return "review_requested";
  }
  if (
    reviewState === "APPROVED" &&
    (checkState === "success" || checkState === undefined) &&
    mergeState !== "dirty"
  ) {
    return "ready";
  }
  return undefined;
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

function PrReadinessBadge({ readiness }: { readiness?: PrReadiness }) {
  if (!readiness) return null;
  const { label, tone, title } = (() => {
    switch (readiness) {
      case "review_requested":
        return {
          label: "review requested",
          title: "This PR is still waiting on review.",
          tone: "bg-blue-100 text-blue-800 dark:bg-blue-950/40 dark:text-blue-300",
        };
      case "changes_requested":
        return {
          label: "changes requested",
          title: "A reviewer requested changes on this PR.",
          tone: "bg-rose-100 text-rose-800 dark:bg-rose-950/40 dark:text-rose-300",
        };
      case "checks_pending":
        return {
          label: "checks pending",
          title: "GitHub commit statuses are still pending.",
          tone: "bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300",
        };
      case "checks_failing":
        return {
          label: "checks failing",
          title: "GitHub commit statuses are failing.",
          tone: "bg-rose-100 text-rose-800 dark:bg-rose-950/40 dark:text-rose-300",
        };
      case "merge_conflict":
        return {
          label: "merge conflict",
          title: "GitHub reports the PR is not currently mergeable.",
          tone: "bg-orange-100 text-orange-800 dark:bg-orange-950/40 dark:text-orange-300",
        };
      case "ready":
        return {
          label: "ready",
          title: "Review, checks, and mergeability look ready from GitHub data.",
          tone: "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300",
        };
    }
  })();
  return (
    <span
      data-testid="pr-readiness-badge"
      data-pr-readiness={readiness}
      title={title}
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
      case "withdrawn":
        return "bg-gray-200 text-gray-600 italic dark:bg-gray-800 dark:text-gray-400";
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
