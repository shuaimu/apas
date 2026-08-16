"use client";

import dynamic from "next/dynamic";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  History,
  LoaderCircle,
  Plus,
  Send,
  SquareTerminal,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { AskUserQuestionCard } from "@/components/tools/AskUserQuestionCard";
import { MobileProjectManageSheet } from "./MobileProjectManageSheet";
import { MobilePaneWorkSummarySheet } from "./MobilePaneWorkSummarySheet";
import {
  paneKey,
  useStore,
  type Message,
  type PaneConfig,
  type PaneKind,
  type Provider,
} from "@/lib/store";

const TerminalPane = dynamic(
  () => import("@/components/tabs/TerminalPane").then((module) => module.TerminalPane),
  {
    ssr: false,
    loading: () => <div className="flex-1 bg-[#0a0a0a] p-4 font-mono text-xs text-neutral-500">Loading terminal…</div>,
  },
);

const MAIN_PANE_ID = 0;
const INITIAL_ACTIVITY_LIMIT = 80;
const MOBILE_ACTIVITY_SCROLL_PREFIX = "apas_mobile_activity_scroll:";
const MOBILE_SELECTED_PANE_PREFIX = "apas_mobile_selected_pane:";
const BOTTOM_FOLLOW_THRESHOLD_PX = 72;

interface SavedActivityScroll {
  scrollTop: number;
  followNewest: boolean;
}

function activityScrollKey(sessionId: string, paneId: number): string {
  return `${MOBILE_ACTIVITY_SCROLL_PREFIX}${sessionId}:${paneId}`;
}

function readActivityScroll(sessionId: string, paneId: number): SavedActivityScroll | null {
  try {
    const raw = window.localStorage.getItem(activityScrollKey(sessionId, paneId));
    if (!raw) return null;
    const saved = JSON.parse(raw) as Partial<SavedActivityScroll>;
    if (typeof saved.scrollTop !== "number" || !Number.isFinite(saved.scrollTop) || saved.scrollTop < 0) return null;
    return { scrollTop: saved.scrollTop, followNewest: saved.followNewest === true };
  } catch {
    return null;
  }
}

function writeActivityScroll(sessionId: string, paneId: number, element: HTMLElement, followNewest: boolean) {
  try {
    window.localStorage.setItem(activityScrollKey(sessionId, paneId), JSON.stringify({
      scrollTop: element.scrollTop,
      followNewest,
    } satisfies SavedActivityScroll));
  } catch {
    // Scroll restoration is a convenience; private-mode storage failures
    // must never take down the conversation.
  }
}

function readSelectedPane(sessionId: string): number | null {
  try {
    const raw = window.localStorage.getItem(`${MOBILE_SELECTED_PANE_PREFIX}${sessionId}`);
    if (raw === null) return null;
    const value = Number(raw);
    return Number.isInteger(value) && value >= 0 ? value : null;
  } catch {
    return null;
  }
}

function writeSelectedPane(sessionId: string, paneId: number) {
  try {
    window.localStorage.setItem(`${MOBILE_SELECTED_PANE_PREFIX}${sessionId}`, String(paneId));
  } catch {
    // Keep navigation functional when browser storage is unavailable.
  }
}

interface ActivityItem {
  key: string;
  message: Message;
  paneId: number;
}

interface LaunchOption {
  key: string;
  kind: PaneKind;
  provider: Provider;
  model?: string;
  label: string;
}

export interface MobileSessionActivityProps {
  connected: boolean;
  onBack: () => void;
  onReconnect: () => void;
}

function displayProjectName(workingDir?: string, gitRemote?: string): string {
  return gitRemote?.split("/").pop()
    || workingDir?.replace(/\/$/, "").split("/").pop()
    || "Coding session";
}

function paneLabel(pane: PaneConfig): string {
  return pane.label?.trim() || `${pane.kind === "terminal" ? "Terminal" : "Pane"} ${pane.pane_id}`;
}

function eventLabel(message: Message): string | null {
  const output = message.outputType;
  if (message.role === "user") return null;
  if (!output || output.type === "text" || output.type === "code") return message.role === "system" ? "system" : null;
  if (output.type === "approval_request") return "approval";
  if (output.type === "tool_use") return output.tool === "AskUserQuestion" ? "question" : "tool";
  if (output.type === "tool_result") return output.success ? "tool result" : "error";
  return output.type.replaceAll("_", " ");
}

function eventTitle(message: Message): string {
  const output = message.outputType;
  if (output?.type === "approval_request") return `${output.tool} requires permission`;
  if (output?.type === "tool_use") return output.tool === "AskUserQuestion" ? "Agent question" : `Using ${output.tool}`;
  if (output?.type === "tool_result") return `${output.tool} ${output.success ? "finished" : "failed"}`;
  if (message.role === "user") return message.content.trim() || "Instruction sent";
  if (message.role === "system") return "Session update";
  return message.content.trim().split("\n")[0] || "Agent activity";
}

function eventDetail(message: Message): string {
  const output = message.outputType;
  if (output?.type === "tool_use") {
    try {
      return JSON.stringify(output.input, null, 2);
    } catch {
      return String(output.input);
    }
  }
  if (output?.type === "approval_request") return output.description;
  return message.content;
}

function eventTone(message: Message, answeredQuestions: Map<string, Record<string, string>>): string {
  const output = message.outputType;
  const unansweredQuestion = output?.type === "tool_use"
    && output.tool === "AskUserQuestion"
    && Boolean(output.toolUseId)
    && !answeredQuestions.has(output.toolUseId!);
  if (output?.type === "approval_request" || unansweredQuestion) {
    return "border-amber-400 dark:border-amber-700";
  }
  if (output?.type === "error" || (output?.type === "tool_result" && !output.success)) {
    return "border-red-400 dark:border-red-800";
  }
  if (message.role === "user") {
    return "border-[#c8c1ff] dark:border-[#665cc7]";
  }
  return "border-[#dedee7] dark:border-[#383842]";
}

function eventSurface(message: Message): string {
  return message.role === "user"
    ? "ml-10 bg-[#eeecff] shadow-none dark:bg-[#292452]"
    : "bg-white shadow-sm dark:bg-[#1b1b21]";
}

function parseLaunchProfile(key: string): LaunchOption | null {
  const [rawKind, frontend, backend, ...modelParts] = key.split(":");
  if (rawKind !== "agent" && rawKind !== "terminal") return null;
  const provider = (backend === "deepseek" ? "deepseek" : frontend) as Provider;
  if (!["claude", "codex", "deepseek", "opencode", "cursor-agent"].includes(provider)) return null;
  const rawModel = modelParts.join(":");
  const model = rawModel && rawModel !== "default" ? rawModel : undefined;
  const providerLabel = provider === "cursor-agent"
    ? "Cursor"
    : provider === "opencode"
      ? "OpenCode"
      : provider.charAt(0).toUpperCase() + provider.slice(1);
  return {
    key,
    kind: rawKind,
    provider,
    model,
    label: `${providerLabel}${rawKind === "terminal" ? " terminal" : " agent"}${model ? ` · ${model}` : ""}`,
  };
}

function MobileEventCard({
  item,
  answeredQuestions,
  onApprove,
  onReject,
}: {
  item: ActivityItem;
  answeredQuestions: Map<string, Record<string, string>>;
  onApprove: (toolCallId: string) => void;
  onReject: (toolCallId: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const { message } = item;
  const output = message.outputType;
  const detail = eventDetail(message);
  const label = eventLabel(message);
  const expandable = detail.length > 180 || detail.includes("\n") || output?.type === "tool_use";

  return (
    <article data-message-role={message.role} className={`rounded-2xl border p-3.5 ${eventSurface(message)} ${eventTone(message, answeredQuestions)}`}>
      <button
        type="button"
        aria-expanded={expandable ? expanded : undefined}
        onClick={() => expandable && setExpanded((value) => !value)}
        className="w-full min-w-0 text-left"
      >
        {label && (
          <div className="mb-2.5 flex items-center">
            <span className="shrink-0 rounded-full bg-[#efeff5] px-2.5 py-1 text-[0.68rem] font-extrabold uppercase tracking-wide text-[#686873] dark:bg-[#25252d] dark:text-[#aaaab6]">
              {label}
            </span>
          </div>
        )}
        <div data-message-line className="flex items-start gap-2">
          <p className={`min-w-0 flex-1 whitespace-pre-wrap break-words text-sm leading-5 ${message.role === "user" ? "font-medium" : "font-semibold"} ${expanded ? "" : "line-clamp-3"}`}>
            {eventTitle(message)}
            <time dateTime={message.timestamp.toISOString()} className="ml-2 inline-block whitespace-nowrap text-[0.68rem] font-normal leading-none text-[#686873] dark:text-[#aaaab6]">
              {message.timestamp.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
            </time>
          </p>
          {expandable && (expanded ? <ChevronUp className="mt-0.5 h-4 w-4 shrink-0 text-[#686873]" /> : <ChevronDown className="mt-0.5 h-4 w-4 shrink-0 text-[#686873]" />)}
        </div>
      </button>

      {expanded && message.role !== "user" && output?.type !== "approval_request" && !(output?.type === "tool_use" && output.tool === "AskUserQuestion") && (
        <pre className="mt-3 box-border w-full min-w-0 max-w-full overflow-auto whitespace-pre-wrap break-words rounded-xl bg-[#efeff5] p-3 text-xs font-normal leading-5 text-[#45454f] dark:bg-[#111115] dark:text-[#c7c7d1]">{detail}</pre>
      )}

      {output?.type === "approval_request" && (
        <div className="mt-3 border-t border-amber-200 pt-3 dark:border-amber-900">
          <p className="mb-3 text-sm text-[#686873] dark:text-[#aaaab6]">{output.description}</p>
          <div className="flex gap-2">
            <button type="button" onClick={() => onReject(output.toolCallId)} className="flex-1 rounded-xl border border-red-300 px-3 py-2 text-sm font-bold text-red-600 dark:border-red-900 dark:text-red-300">Reject</button>
            <button type="button" onClick={() => onApprove(output.toolCallId)} className="flex-1 rounded-xl bg-emerald-600 px-3 py-2 text-sm font-bold text-white">Approve</button>
          </div>
        </div>
      )}
      {output?.type === "tool_use" && output.tool === "AskUserQuestion" && (
        <AskUserQuestionCard toolUseId={output.toolUseId} input={output.input} />
      )}
    </article>
  );
}

export function MobileSessionActivity({ connected, onBack, onReconnect }: MobileSessionActivityProps) {
  const sessionId = useStore((state) => state.sessionId);
  const sessions = useStore((state) => state.sessions);
  const paneConfigs = useStore((state) => state.paneConfigs);
  const messages = useStore((state) => state.messages);
  const paneMessages = useStore((state) => state.paneMessages);
  const paneStatuses = useStore((state) => state.paneStatuses);
  const paneHasMore = useStore((state) => state.paneHasMore);
  const isAttached = useStore((state) => state.isAttached);
  const answeredQuestions = useStore((state) => state.answeredQuestions);
  const planReviewPending = useStore((state) => state.planReviewPending);
  const projectPolicies = useStore((state) => state.projectPolicies);
  const loadSessionActivity = useStore((state) => state.loadSessionActivity);
  const loadPaneMessagesIfNeeded = useStore((state) => state.loadPaneMessagesIfNeeded);
  const loadMoreMessages = useStore((state) => state.loadMoreMessages);
  const sendMessageToPane = useStore((state) => state.sendMessageToPane);
  const sendTerminalConversationMessage = useStore((state) => state.sendTerminalConversationMessage);
  const approve = useStore((state) => state.approve);
  const reject = useStore((state) => state.reject);
  const answerPlanReview = useStore((state) => state.answerPlanReview);
  const addPane = useStore((state) => state.addPane);
  const summarySupported = useStore((state) => state.negotiatedCapabilities.has("pane_work_summary_v1"));

  const [selectedPaneId, setSelectedPaneId] = useState<number | null>(null);
  const [followUp, setFollowUp] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);
  const [activityLimit, setActivityLimit] = useState(INITIAL_ACTIVITY_LIMIT);
  const [terminalPaneId, setTerminalPaneId] = useState<number | null>(null);
  const [newPaneOpen, setNewPaneOpen] = useState(false);
  const [summaryOpen, setSummaryOpen] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const activityScrollRef = useRef<HTMLDivElement>(null);
  const restoredScrollContextRef = useRef<string | null>(null);
  const restoredScrollElementRef = useRef<HTMLDivElement | null>(null);
  const selectedPaneSessionRef = useRef<string | null>(null);
  const followNewestRef = useRef(true);
  const saveScrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const session = sessions.find((item) => item.id === sessionId);
  const selectedPane = paneConfigs.find((pane) => pane.pane_id === selectedPaneId);
  const selectedStatus = selectedPaneId === null ? null : paneStatuses[paneKey(selectedPaneId)] || null;

  useEffect(() => {
    if (sessionId) loadSessionActivity(sessionId);
  }, [loadSessionActivity, sessionId]);

  useEffect(() => {
    if (!sessionId) return;
    const sameSession = selectedPaneSessionRef.current === sessionId;
    if (sameSession && selectedPaneId !== null && paneConfigs.some((pane) => pane.pane_id === selectedPaneId)) return;

    const rememberedPaneId = readSelectedPane(sessionId);
    const remembered = paneConfigs.find((pane) => pane.pane_id === rememberedPaneId);
    const preferred = remembered ?? paneConfigs.find((pane) => pane.kind !== "terminal") ?? paneConfigs[0];
    const nextPaneId = preferred?.pane_id ?? (messages.length > 0 ? MAIN_PANE_ID : null);
    selectedPaneSessionRef.current = sessionId;
    if (nextPaneId !== null) writeSelectedPane(sessionId, nextPaneId);
    if (selectedPaneId !== nextPaneId) setSelectedPaneId(nextPaneId);
  }, [messages.length, paneConfigs, selectedPaneId, sessionId]);

  useEffect(() => {
    if (selectedPaneId !== null) loadPaneMessagesIfNeeded(selectedPaneId);
  }, [loadPaneMessagesIfNeeded, selectedPaneId]);

  const activity = useMemo(() => {
    const items: ActivityItem[] = messages.map((message) => ({ key: `0:${message.id}`, message, paneId: MAIN_PANE_ID }));
    for (const [rawPaneId, paneActivity] of Object.entries(paneMessages)) {
      const paneId = Number(rawPaneId);
      for (const message of paneActivity) items.push({ key: `${paneId}:${message.id}`, message, paneId });
    }
    items.sort((left, right) => left.message.timestamp.getTime() - right.message.timestamp.getTime());
    return items;
  }, [messages, paneMessages]);
  const selectedActivity = useMemo(
    () => selectedPaneId === null ? [] : activity.filter((item) => item.paneId === selectedPaneId),
    [activity, selectedPaneId],
  );
  const selectedPlanReviews = useMemo(
    () => selectedPaneId === null ? [] : planReviewPending.filter((plan) => plan.paneId === selectedPaneId),
    [planReviewPending, selectedPaneId],
  );
  const visibleActivity = selectedActivity.slice(Math.max(0, selectedActivity.length - activityLimit));
  const firstVisibleActivityKey = visibleActivity[0]?.key;
  const lastVisibleActivityKey = visibleActivity.at(-1)?.key;

  const persistActivityScroll = useCallback(() => {
    if (!sessionId || selectedPaneId === null || !activityScrollRef.current) return;
    writeActivityScroll(sessionId, selectedPaneId, activityScrollRef.current, followNewestRef.current);
  }, [selectedPaneId, sessionId]);

  const handleActivityScroll = useCallback(() => {
    const element = activityScrollRef.current;
    if (!element) return;
    const distanceFromBottom = element.scrollHeight - element.clientHeight - element.scrollTop;
    followNewestRef.current = distanceFromBottom <= BOTTOM_FOLLOW_THRESHOLD_PX;
    if (saveScrollTimerRef.current) clearTimeout(saveScrollTimerRef.current);
    saveScrollTimerRef.current = setTimeout(() => {
      saveScrollTimerRef.current = null;
      persistActivityScroll();
    }, 150);
  }, [persistActivityScroll]);

  // Restore before paint so reopening a project never flashes at the oldest
  // loaded message. A conversation with no saved position starts at newest.
  useLayoutEffect(() => {
    const element = activityScrollRef.current;
    if (!sessionId || selectedPaneId === null || !element || visibleActivity.length === 0) return;

    const scrollContext = `${sessionId}:${selectedPaneId}`;

    if (
      restoredScrollContextRef.current !== scrollContext
      || restoredScrollElementRef.current !== element
    ) {
      const saved = readActivityScroll(sessionId, selectedPaneId);
      restoredScrollContextRef.current = scrollContext;
      restoredScrollElementRef.current = element;
      followNewestRef.current = saved?.followNewest ?? true;
      const bottom = Math.max(0, element.scrollHeight - element.clientHeight);
      element.scrollTop = followNewestRef.current
        ? bottom
        : Math.min(saved?.scrollTop ?? bottom, bottom);
      return;
    }

    if (followNewestRef.current) {
      element.scrollTop = Math.max(0, element.scrollHeight - element.clientHeight);
    }
  }, [firstVisibleActivityKey, lastVisibleActivityKey, selectedPaneId, sessionId, terminalPaneId, visibleActivity.length]);

  useEffect(() => {
    const element = activityScrollRef.current;
    if (!sessionId || selectedPaneId === null || !element) return;
    const save = () => writeActivityScroll(sessionId, selectedPaneId, element, followNewestRef.current);
    window.addEventListener("pagehide", save);
    return () => {
      window.removeEventListener("pagehide", save);
      if (saveScrollTimerRef.current) {
        clearTimeout(saveScrollTimerRef.current);
        saveScrollTimerRef.current = null;
      }
      save();
    };
  }, [selectedPaneId, sessionId]);

  const launchOptions = useMemo(() => {
    if (!sessionId) return [];
    return (projectPolicies[sessionId]?.allowedLaunchProfiles ?? [])
      .flatMap((key) => {
        const parsed = parseLaunchProfile(key);
        return parsed?.kind === "terminal" ? [parsed] : [];
      });
  }, [projectPolicies, sessionId]);

  const selectPane = (paneId: number) => {
    persistActivityScroll();
    if (sessionId) writeSelectedPane(sessionId, paneId);
    setActivityLimit(INITIAL_ACTIVITY_LIMIT);
    setSelectedPaneId(paneId);
    loadPaneMessagesIfNeeded(paneId);
  };

  const sendFollowUp = () => {
    if (selectedPaneId === null || !followUp.trim()) return;
    const sessionIsRunning = session?.isActive ?? isAttached;
    if (!connected || !sessionIsRunning) {
      setActionError("Reconnect to the active session before sending a message.");
      return;
    }
    if (selectedPane?.kind === "terminal") {
      const result = sendTerminalConversationMessage(selectedPaneId, followUp);
      if (!result.success) {
        setActionError(result.error || "The conversation message could not be sent.");
        return;
      }
      setFollowUp("");
      setActionError(null);
      return;
    }
    const result = sendMessageToPane(followUp.trim(), selectedPaneId);
    if (!result.success) {
      setActionError(result.error || "The instruction could not be sent.");
      return;
    }
    setFollowUp("");
    setActionError(null);
  };

  const createPane = (option: LaunchOption) => {
    const ordinal = paneConfigs.length + 1;
    const result = addPane(
      option.provider,
      "interactive",
      `${option.label} ${ordinal}`,
      undefined,
      option.model,
      false,
      undefined,
      false,
      option.kind,
    );
    if (!result.success) {
      setActionError(result.error || "The pane could not be created.");
      setNewPaneOpen(false);
      return;
    }
    setActionError(null);
    setNewPaneOpen(false);
  };

  if (terminalPaneId !== null) {
    return (
      <section aria-label="Mobile terminal" className="flex h-full min-h-0 flex-col bg-[#0a0a0a] text-white">
        <div className="flex shrink-0 items-center justify-between border-b border-neutral-800 px-2 py-1.5">
          <button type="button" onClick={() => setTerminalPaneId(null)} className="flex items-center gap-2 rounded-lg px-2 py-2 text-sm font-bold hover:bg-neutral-800">
            <ArrowLeft className="h-5 w-5" /> Conversation
          </button>
          <span className="truncate px-2 text-xs text-neutral-400">{selectedPane ? paneLabel(selectedPane) : `Pane ${terminalPaneId}`}</span>
        </div>
        <div className="flex min-h-0 flex-1 flex-col"><TerminalPane paneId={terminalPaneId} /></div>
      </section>
    );
  }

  if (!sessionId || !session) {
    return (
      <section className="flex h-full flex-col items-center justify-center bg-[#f7f7fa] p-6 text-center dark:bg-[#111115]">
        <h1 className="text-xl font-extrabold">Session unavailable</h1>
        <p className="mt-2 text-sm text-[#686873] dark:text-[#aaaab6]">It may have been deleted or your project access may have changed.</p>
        <button type="button" onClick={onBack} className="mt-4 rounded-xl bg-[#6d5efc] px-4 py-2.5 text-sm font-bold text-white">Back to sessions</button>
      </section>
    );
  }

  const projectName = displayProjectName(session.workingDir, session.gitRemote);
  const selectedIsTerminal = selectedPane?.kind === "terminal";
  const selectedIsBot = selectedPane?.mode === "deadloop";
  const sessionIsRunning = session.isActive ?? isAttached;
  const canCompose = selectedPaneId !== null && !selectedIsBot;
  const canSend = canCompose && connected && sessionIsRunning;

  return (
    <section aria-label="Mobile session activity" className="flex h-full min-h-0 w-full flex-col overflow-hidden bg-[#f7f7fa] text-[#18181b] dark:bg-[#111115] dark:text-[#f7f7fa]">
      {!connected && (
        <button type="button" onClick={onReconnect} className="flex shrink-0 items-center justify-center gap-2 border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs font-bold text-amber-800 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          <WifiOff className="h-4 w-4" /> Offline · tap to reconnect
        </button>
      )}

      {/* Switching panes is the thing done constantly here, so it sits in the
          top row; the project's identity is read once and takes the small line
          beneath. Account left entirely — it navigates away from the session,
          and the home screen has it. */}
      <div className="shrink-0 border-b border-[#dedee7] px-4 pt-3 pb-2.5 dark:border-[#383842]">
        <div className="flex items-center gap-2">
          <button type="button" aria-label="Back to coding sessions" onClick={onBack} className="-ml-2 shrink-0 rounded-lg p-2 text-[#686873] hover:bg-[#efeff5] dark:text-[#aaaab6] dark:hover:bg-[#25252d]"><ArrowLeft className="h-5 w-5" /></button>
          <div className="no-scrollbar flex min-w-0 flex-1 items-center gap-2 overflow-x-auto">
            {paneConfigs.length > 0 ? paneConfigs.map((pane) => {
              const selected = pane.pane_id === selectedPaneId;
              return (
                <button key={pane.pane_id} type="button" aria-pressed={selected} onClick={() => selectPane(pane.pane_id)} className={`shrink-0 rounded-full border px-3 py-1.5 text-xs font-bold ${selected ? "border-[#6d5efc] text-[#6d5efc]" : "border-[#dedee7] text-[#686873] dark:border-[#383842] dark:text-[#aaaab6]"}`}>
                  {paneLabel(pane)}{paneStatuses[paneKey(pane.pane_id)] ? " · working" : ""}
                </button>
              );
            }) : (
              <span className="shrink-0 text-xs text-[#686873] dark:text-[#aaaab6]">Waiting for panes…</span>
            )}
            <button type="button" aria-label="Create pane" onClick={() => setNewPaneOpen(true)} className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[#6d5efc] text-white"><Plus className="h-4 w-4" /></button>
          </div>
          <button type="button" onClick={() => setManageOpen(true)} className="min-h-9 shrink-0 px-1 text-sm font-bold text-[#6d5efc]">Manage</button>
        </div>

        <div className="mt-1.5 flex items-center gap-2">
          {/* Still the screen's heading, just no longer the largest thing on
              it — losing the h1 would leave the screen without one. */}
          {/* The heading is the project alone; the target rides beside it, so
              the screen keeps a heading that names one thing. */}
          <div className="flex min-w-0 flex-1 items-baseline gap-1.5">
            <h1 className="shrink-0 max-w-[55%] truncate text-sm font-bold">{projectName}</h1>
            <span className="min-w-0 flex-1 truncate text-xs text-[#686873] dark:text-[#aaaab6]">
              {session.hostname || session.workingDir || "Unknown target"}
            </span>
          </div>
          <span className={`shrink-0 rounded-full px-2.5 py-1 text-[0.68rem] font-extrabold ${session.isActive ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300" : "bg-[#efeff5] text-[#686873] dark:bg-[#25252d] dark:text-[#aaaab6]"}`}>
            {session.isActive ? "Active" : session.status}
          </span>
        </div>
      </div>

      {actionError && <div className="mx-4 mt-3 flex shrink-0 items-center gap-2 rounded-xl border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300"><AlertTriangle className="h-4 w-4 shrink-0" /> {actionError}</div>}


      <div ref={activityScrollRef} role="log" aria-label="Conversation activity" onScroll={handleActivityScroll} className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        {selectedPlanReviews.length > 0 && (
          <div className="mb-3 space-y-2">
            {selectedPlanReviews.map((plan) => (
              <div key={plan.toolUseId} className="rounded-2xl border border-amber-400 bg-white p-3.5 dark:border-amber-700 dark:bg-[#1b1b21]">
                <span className="rounded-full bg-amber-100 px-2.5 py-1 text-[0.68rem] font-extrabold uppercase text-amber-800 dark:bg-amber-950/60 dark:text-amber-200">plan review</span>
                <p className="mt-2 text-sm font-semibold">Pane {plan.paneId} wants to use {plan.toolName}.</p>
                <div className="mt-3 flex gap-2"><button type="button" onClick={() => answerPlanReview(plan.toolUseId, false)} className="flex-1 rounded-xl border border-red-300 py-2 text-sm font-bold text-red-600 dark:border-red-900 dark:text-red-300">Deny</button><button type="button" onClick={() => answerPlanReview(plan.toolUseId, true)} className="flex-1 rounded-xl bg-emerald-600 py-2 text-sm font-bold text-white">Approve</button></div>
              </div>
            ))}
          </div>
        )}

        {selectedActivity.length > visibleActivity.length && (
          <button type="button" onClick={() => setActivityLimit((limit) => limit + INITIAL_ACTIVITY_LIMIT)} className="mb-3 w-full rounded-xl border border-[#dedee7] bg-white px-3 py-2 text-xs font-bold dark:border-[#383842] dark:bg-[#1b1b21]">
            Show earlier loaded activity
          </button>
        )}
        {selectedPaneId !== null && paneHasMore[paneKey(selectedPaneId)] && (
          <button type="button" onClick={() => loadMoreMessages(selectedPaneId)} className="mb-3 w-full rounded-xl border border-[#dedee7] bg-white px-3 py-2 text-xs font-bold dark:border-[#383842] dark:bg-[#1b1b21]">
            Load older activity for {selectedPane ? paneLabel(selectedPane) : `pane ${selectedPaneId}`}
          </button>
        )}

        {visibleActivity.length > 0 ? (
          <div className="space-y-2.5">
            {visibleActivity.map((item) => <MobileEventCard key={item.key} item={item} answeredQuestions={answeredQuestions} onApprove={approve} onReject={reject} />)}
          </div>
        ) : (
          <div className="flex min-h-60 flex-col items-center justify-center text-center">
            <div className="rounded-2xl bg-[#efeff5] p-3 dark:bg-[#25252d]">{connected ? <Wifi className="h-6 w-6 text-[#686873]" /> : <WifiOff className="h-6 w-6 text-[#686873]" />}</div>
            <h2 className="mt-3 text-lg font-extrabold">{connected ? "No activity yet" : "No cached activity"}</h2>
            <p className="mt-1 text-sm text-[#686873] dark:text-[#aaaab6]">{connected ? "Instructions and agent activity for this pane will appear here." : "Reconnect to retrieve this pane's activity."}</p>
          </div>
        )}
      </div>

      <div className="shrink-0 border-t border-[#dedee7] bg-[#f7f7fa] p-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] dark:border-[#383842] dark:bg-[#111115]">
        {connected && selectedStatus && <div role="status" aria-live="polite" className="mb-1.5 flex w-fit max-w-full items-center gap-1.5 rounded-full bg-[#eeecff] px-2.5 py-1 text-xs font-bold text-[#5b4de0] dark:bg-[#292452] dark:text-[#c8c1ff]"><LoaderCircle aria-hidden="true" className="h-3.5 w-3.5 shrink-0 animate-spin" /><span className="truncate">{selectedStatus}</span></div>}
        {!connected && canCompose && <p className="mb-1.5 text-[11px] text-amber-700 dark:text-amber-300">You can keep drafting while offline. Reconnect to send.</p>}
        {connected && !sessionIsRunning && canCompose && <p className="mb-1.5 text-[11px] text-amber-700 dark:text-amber-300">You can draft a message, but this project must be running before it can be sent.</p>}
        <div className="flex items-end gap-2">
          <textarea value={followUp} onChange={(event) => setFollowUp(event.target.value)} rows={2} placeholder={selectedIsBot ? "Stop the bot before steering it" : selectedIsTerminal ? "Message this terminal conversation" : "Steer this session and pane"} disabled={!canCompose} className="min-h-[2.75rem] flex-1 resize-none rounded-xl border border-[#dedee7] bg-white px-3 py-2 text-sm outline-none focus:border-[#6d5efc] disabled:opacity-50 dark:border-[#383842] dark:bg-[#1b1b21]" />
          {/* Occasional, and about the selected pane rather than the
              conversation — so they sit here instead of costing a row above it. */}
          <button type="button" aria-label="Open raw terminal" disabled={!selectedIsTerminal} onClick={() => {
            if (selectedPaneId === null) return;
            persistActivityScroll();
            setTerminalPaneId(selectedPaneId);
          }} className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-[#dedee7] bg-white text-[#686873] disabled:opacity-40 dark:border-[#383842] dark:bg-[#1b1b21] dark:text-[#aaaab6]"><SquareTerminal className="h-5 w-5" /></button>
          {summarySupported && (
            <button type="button" aria-label="Open work summary" disabled={selectedPaneId === null} onClick={() => setSummaryOpen(true)} className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-[#dedee7] bg-white text-[#686873] disabled:opacity-40 dark:border-[#383842] dark:bg-[#1b1b21] dark:text-[#aaaab6]"><History className="h-5 w-5" /></button>
          )}
          <button type="button" aria-label={selectedIsTerminal ? "Send conversation message" : "Send follow-up"} disabled={!canSend || !followUp.trim()} onClick={sendFollowUp} className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-[#6d5efc] text-white disabled:opacity-40"><Send className="h-5 w-5" /></button>
        </div>
      </div>

      {newPaneOpen && (
        <div className="fixed inset-0 z-[95] flex items-end bg-black/45" onClick={() => setNewPaneOpen(false)}>
          <div role="dialog" aria-modal="true" aria-label="Create pane" onClick={(event) => event.stopPropagation()} className="max-h-[82dvh] w-full overflow-y-auto rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]">
            <div className="flex items-center justify-between"><div><h2 className="text-xl font-extrabold">Create pane</h2><p className="mt-1 text-sm text-[#686873] dark:text-[#aaaab6]">Choose a profile allowed by this project's cluster policy.</p></div><button type="button" aria-label="Close create pane" onClick={() => setNewPaneOpen(false)} className="rounded-lg p-2 hover:bg-[#efeff5] dark:hover:bg-[#25252d]"><X className="h-5 w-5" /></button></div>
            <div className="mt-4 space-y-2">
              {launchOptions.map((option) => <button key={option.key} type="button" onClick={() => createPane(option)} className="flex w-full items-center justify-between rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left dark:border-[#383842] dark:bg-[#1b1b21]"><span><span className="block font-bold">{option.label}</span><span className="mt-0.5 block text-xs text-[#686873] dark:text-[#aaaab6]">{option.kind === "terminal" ? "Interactive TUI" : "Coding activity pane"}</span></span><Plus className="h-5 w-5 text-[#6d5efc]" /></button>)}
              {launchOptions.length === 0 && <div className="rounded-2xl border border-[#dedee7] bg-white p-4 text-sm text-[#686873] dark:border-[#383842] dark:bg-[#1b1b21] dark:text-[#aaaab6]">Waiting for the project's launch policy, or no profiles are currently allowed.</div>}
            </div>
          </div>
        </div>
      )}

      {manageOpen && <MobileProjectManageSheet onClose={() => setManageOpen(false)} />}

      {summaryOpen && selectedPaneId !== null && (
        <MobilePaneWorkSummarySheet
          connected={connected}
          sessionId={sessionId}
          paneId={selectedPaneId}
          panes={paneConfigs}
          onSelectPane={selectPane}
          onClose={() => setSummaryOpen(false)}
        />
      )}
    </section>
  );
}
