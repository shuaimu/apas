"use client";

import dynamic from "next/dynamic";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  FolderOpen,
  History,
  LoaderCircle,
  MoreVertical,
  Pencil,
  Plus,
  RotateCcw,
  Settings2,
  Trash2,
  Send,
  SquareTerminal,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { AskUserQuestionCard } from "@/components/tools/AskUserQuestionCard";
import { ProjectJobsWorkspace } from "@/components/jobs/ProjectJobsWorkspace";
import { readSelectedPane, writeSelectedPane } from "@/lib/mobileSelectedPane";
import { isOmpQuestionInput, ompQuestionKey, parseOmpQuestionResult } from "@/lib/ompQuestions";
import {
  resolveMachineProjectTarget,
  type MachineProjectTarget,
} from "@/lib/machineProjectTarget";
import {
  refusedDeliveries,
  unconfirmedDeliveries,
} from "@/lib/terminalDelivery";
import { instancesNegotiated, selectSessionInstances } from "@/lib/projectInstances";
import { buildMachineChoices, sessionCanCloneJoin, type MachineChoice } from "@/lib/machineChoice";
import {
  cloneHereCopy,
  cloneInstanceName,
  describeTabHosts,
  forgetMachineTabsCopy,
  groupMachineChoices,
  hostList,
  machineChoiceDetail,
  projectHostLabel,
  projectStartHosts,
  sessionIsOwned,
  tabHostStateLabel,
  type ConfirmationCopy,
} from "@/lib/tabHosts";
import { AddPaneLaunches } from "@/components/tabs/AddPaneLaunches";
import { MobileProjectManageSheet } from "./MobileProjectManageSheet";
import { MobilePaneWorkSummarySheet } from "./MobilePaneWorkSummarySheet";
import { MobileTerminalKeyBar } from "./MobileTerminalKeyBar";
import {
  canonicalDeepseekModel,
  DEEPSEEK_FLASH_MODEL,
} from "@/lib/providerOptions";
import {
  paneKey,
  useStore,
  type Message,
  type PaneCleanupAction,
  type PaneConfig,
  type PaneKind,
  type Provider,
} from "@/lib/store";
import { paneIsAwaitingAnswerStatus } from "@/lib/paneStatus";

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
  if (element.closest("[hidden]")) return;
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

function isRoutineSystemUpdate(message: Message): boolean {
  // Provider result envelopes only carry end-of-turn bookkeeping such as
  // subtype, cost, and duration. They are useful to accounting/debug views,
  // but rendering one as "Session update" after every Claude turn adds no
  // conversational information on mobile.
  return message.role === "system" && message.outputType?.type === "system";
}

function eventTitle(message: Message): string {
  const output = message.outputType;
  if (output?.type === "approval_request") return `${output.tool} requires permission`;
  if (output?.type === "tool_use") return output.tool === "AskUserQuestion" ? "Agent question" : `Using ${output.tool}`;
  if (output?.type === "tool_result") return `${output.tool} ${output.success ? "finished" : "failed"}`;
  if (output?.type === "error") return message.content.trim().split("\n")[0] || "Session error";
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

function eventTone(message: Message, answeredQuestions: Map<string, Record<string, string>>, nativeResolved: boolean): string {
  const output = message.outputType;
  const unansweredQuestion = output?.type === "tool_use"
    && output.tool === "AskUserQuestion"
    && Boolean(output.toolUseId)
    && !nativeResolved
    && !answeredQuestions.has(output.toolUseId!);
  if (output?.type === "approval_request" || unansweredQuestion) {
    return "border-amber-400 dark:border-amber-700";
  }
  if (output?.type === "error" || (output?.type === "tool_result" && !output.success)) {
    return "border-red-400 dark:border-red-800";
  }
  if (message.role === "user") {
    return message.deliveryError
      ? "border-red-400 dark:border-red-800"
      : "border-[#c8c1ff] dark:border-[#665cc7]";
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
  // Terminal panes always run the frontend binary (DeepSeek has no TUI);
  // the deepseek backend is carried as the pane model instead. Agent panes
  // keep the legacy direct Deepseek provider mapping.
  const provider = (
    backend === "deepseek" && rawKind !== "terminal" ? "deepseek" : frontend
  ) as Provider;
  if (!["claude", "codex", "deepseek", "opencode", "pi", "omp", "cursor-agent"].includes(provider)) return null;
  const rawModel = modelParts.join(":");
  const model = rawModel && rawModel !== "default" ? rawModel : undefined;
  const deepseekModel = canonicalDeepseekModel(model);
  const providerLabel = deepseekModel === DEEPSEEK_FLASH_MODEL
    ? "DeepSeek 4.1 Flash"
    : provider === "cursor-agent"
        ? "Cursor"
        : provider === "opencode"
          ? "OpenCode"
          : provider === "pi"
            ? "Pi"
          : provider === "omp"
            ? "OMP"
            : provider.charAt(0).toUpperCase() + provider.slice(1);
  return {
    key,
    kind: rawKind,
    provider,
    model,
    label: `${providerLabel}${rawKind === "terminal" ? " terminal" : " agent"}`,
  };
}

function MobileEventCard({
  item,
  answeredQuestions,
  onApprove,
  onReject,
  disabledReason,
}: {
  item: ActivityItem;
  answeredQuestions: Map<string, Record<string, string>>;
  onApprove: (toolCallId: string) => void;
  onReject: (toolCallId: string) => void;
  /** Why the card's pane cannot take answers now: its machine is not running
   *  the project, or this viewer may not use it. The server would only
   *  refuse an answer, so its decisions are disabled with the reason. */
  disabledReason?: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const { message } = item;
  const output = message.outputType;
  const nativeState = useStore((state) => message.sessionId && message.paneId !== undefined &&
    output?.type === "tool_use" && output.toolUseId && isOmpQuestionInput(output.input)
    ? state.ompQuestions.get(ompQuestionKey(message.sessionId, message.paneId, output.toolUseId)) : undefined);
  const detail = eventDetail(message);
  const label = eventLabel(message);
  const expandable = detail.length > 180 || detail.includes("\n") || output?.type === "tool_use";

  return (
    <article data-message-role={message.role} className={`rounded-2xl border p-3.5 ${eventSurface(message)} ${eventTone(message, answeredQuestions, !!nativeState?.result)}`}>
      <button
        type="button"
        aria-expanded={expandable ? expanded : undefined}
        onClick={() => expandable && setExpanded((value) => !value)}
        className="w-full min-w-0 text-left"
      >
        {label && (
          <div className="mb-2.5 flex items-center">
            <span className="shrink-0 rounded-full bg-[#efeff5] px-2.5 py-1 text-xs font-extrabold uppercase tracking-wide text-[#686873] dark:bg-[#25252d] dark:text-[#aaaab6]">
              {label}
            </span>
          </div>
        )}
        <div data-message-line className="flex items-start gap-2">
          <p className={`min-w-0 flex-1 whitespace-pre-wrap break-words text-base leading-6 ${message.role === "user" ? "font-medium" : "font-semibold"} ${expanded ? "" : "line-clamp-3"}`}>
            {eventTitle(message)}
            <time dateTime={message.timestamp.toISOString()} className="ml-2 inline-block whitespace-nowrap text-xs font-normal leading-none text-[#686873] dark:text-[#aaaab6]">
              {message.timestamp.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
            </time>
          </p>
          {expandable && (expanded ? <ChevronUp className="mt-0.5 h-4 w-4 shrink-0 text-[#686873]" /> : <ChevronDown className="mt-0.5 h-4 w-4 shrink-0 text-[#686873]" />)}
        </div>
      </button>

      {message.role === "user" && message.deliveryError && (
        // The server refused this send: say so on the send itself, which
        // keeps its text to recover.
        <p role="status" className="mt-2 flex items-start gap-1.5 text-sm text-red-700 dark:text-red-300">
          <AlertTriangle aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />
          <span className="min-w-0 break-words">Not delivered: {message.deliveryError}</span>
        </p>
      )}

      {expanded && message.role !== "user" && output?.type !== "approval_request" && !(output?.type === "tool_use" && output.tool === "AskUserQuestion") && (
        <pre className="mt-3 box-border w-full min-w-0 max-w-full overflow-auto whitespace-pre-wrap break-words rounded-xl bg-[#efeff5] p-3 text-sm font-normal leading-6 text-[#45454f] dark:bg-[#111115] dark:text-[#c7c7d1]">{detail}</pre>
      )}

      {output?.type === "approval_request" && (
        <div className="mt-3 border-t border-amber-200 pt-3 dark:border-amber-900">
          <p className="mb-3 text-sm text-[#686873] dark:text-[#aaaab6]">{output.description}</p>
          {disabledReason && <p role="status" className="mb-3 text-sm text-amber-800 dark:text-amber-200">{disabledReason}</p>}
          <div className="flex gap-2">
            <button type="button" disabled={Boolean(disabledReason)} title={disabledReason} onClick={() => onReject(output.toolCallId)} className="flex-1 rounded-xl border border-red-300 px-3 py-2 text-sm font-bold text-red-600 disabled:opacity-40 dark:border-red-900 dark:text-red-300">Reject</button>
            <button type="button" disabled={Boolean(disabledReason)} title={disabledReason} onClick={() => onApprove(output.toolCallId)} className="flex-1 rounded-xl bg-emerald-600 px-3 py-2 text-sm font-bold text-white disabled:opacity-40">Approve</button>
          </div>
        </div>
      )}
      {output?.type === "tool_use" && output.tool === "AskUserQuestion" && (
        <AskUserQuestionCard key={JSON.stringify([message.sessionId, message.paneId, output.toolUseId])} toolUseId={output.toolUseId} input={output.input} sessionId={message.sessionId} paneId={message.paneId} disabledReason={disabledReason} />
      )}
    </article>
  );
}

export function MobileSessionActivity({ connected, onBack, onReconnect }: MobileSessionActivityProps) {
  const sessionId = useStore((state) => state.sessionId);
  const accountId = useStore((state) => state.userId);
  const sessions = useStore((state) => state.sessions);
  const paneConfigs = useStore((state) => state.paneConfigs);
  const removePane = useStore((state) => state.removePane);
  const rebootPane = useStore((state) => state.rebootPane);
  const updatePaneLabel = useStore((state) => state.updatePaneLabel);
  const messages = useStore((state) => state.messages);
  const paneMessages = useStore((state) => state.paneMessages);
  const paneStatuses = useStore((state) => state.paneStatuses);
  const paneHasMore = useStore((state) => state.paneHasMore);
  const isAttached = useStore((state) => state.isAttached);
  const answeredQuestions = useStore((state) => state.answeredQuestions);
  const planReviewPending = useStore((state) => state.planReviewPending);
  const projectPolicies = useStore((state) => state.projectPolicies);
  const machines = useStore((state) => state.machines);
  const loadSessionActivity = useStore((state) => state.loadSessionActivity);
  const loadPaneMessagesIfNeeded = useStore((state) => state.loadPaneMessagesIfNeeded);
  const loadMoreMessages = useStore((state) => state.loadMoreMessages);
  const sendMessageToPane = useStore((state) => state.sendMessageToPane);
  const sendTerminalConversationMessage = useStore((state) => state.sendTerminalConversationMessage);
  const approve = useStore((state) => state.approve);
  const reject = useStore((state) => state.reject);
  const answerPlanReview = useStore((state) => state.answerPlanReview);
  const addPane = useStore((state) => state.addPane);
  const cloneAndAddPane = useStore((state) => state.cloneAndAddPane);
  const dismissAddPaneRequest = useStore((state) => state.dismissAddPaneRequest);
  const forgetMachinePanes = useStore((state) => state.forgetMachinePanes);
  const addPaneRequests = useStore((state) => state.addPaneRequests);
  const instances = useStore((state) => selectSessionInstances(state, state.sessionId));
  const instancesSupported = useStore(instancesNegotiated);
  const showToast = useStore((state) => state.showToast);
  const startMachineProjectCli = useStore((state) => state.startMachineProjectCli);
  const summarySupported = useStore((state) => state.negotiatedCapabilities.has("pane_work_summary_v1"));

  const [selectedPaneId, setSelectedPaneId] = useState<number | null>(null);
  const [followUp, setFollowUp] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);
  const [activityLimit, setActivityLimit] = useState(INITIAL_ACTIVITY_LIMIT);
  const [terminalPaneId, setTerminalPaneId] = useState<number | null>(null);
  const [mountedTerminal, setMountedTerminal] = useState<{ sessionId: string; paneId: number } | null>(null);
  const [newPaneOpen, setNewPaneOpen] = useState(false);
  const [interactiveCreation, setInteractiveCreation] = useState(false);
  const [summaryOpen, setSummaryOpen] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState<PaneConfig | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [closeTarget, setCloseTarget] = useState<PaneConfig | null>(null);
  const [rebootTarget, setRebootTarget] = useState<PaneConfig | null>(null);
  const [openProjectTarget, setOpenProjectTarget] = useState<MachineProjectTarget | null>(null);
  // The new-agent sheet's machine step, and the two confirmations that act on
  // a machine rather than a pane.
  const [machineKey, setMachineKey] = useState<string | null>(null);
  const [cloneTarget, setCloneTarget] = useState<{ choice: MachineChoice; option: LaunchOption; copy: ConfirmationCopy } | null>(null);
  const [forgetTarget, setForgetTarget] = useState<{ machineId: string; hostname: string; copy: ConfirmationCopy } | null>(null);
  // Messages written into the pty that the provider has not recorded back.
  const pendingDeliveries = useStore((state) => state.pendingTerminalDeliveries);
  const dismissTerminalDelivery = useStore((state) => state.dismissTerminalDelivery);
  const [deliveryNow, setDeliveryNow] = useState(() => Date.now());
  const activityScrollRef = useRef<HTMLDivElement>(null);
  const restoredScrollContextRef = useRef<string | null>(null);
  const restoredScrollElementRef = useRef<HTMLDivElement | null>(null);
  const selectedPaneSessionRef = useRef<string | null>(null);
  const followNewestRef = useRef(true);
  const saveScrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const session = sessions.find((item) => item.id === sessionId);
  const machineProjectTarget = useMemo(
    () => resolveMachineProjectTarget(session, machines),
    [machines, session],
  );
  const selectedPane = paneConfigs.find((pane) => pane.pane_id === selectedPaneId);
  const selectedStatus = selectedPaneId === null ? null : paneStatuses[paneKey(selectedPaneId)] || null;
  const selectedIsAwaitingAnswer = paneIsAwaitingAnswerStatus(selectedStatus);
  const sessionIsRunning = session?.isActive ?? isAttached;
  const renameTargetExists = renameTarget !== null
    && paneConfigs.some((pane) => pane.pane_id === renameTarget.pane_id);

  // Where each pane runs, and why one cannot be used: its machine is not
  // running the project, or this viewer may not use that machine. The same
  // rules the desktop tab bar applies.
  const tabHosts = useMemo(() => describeTabHosts({
    panes: paneConfigs,
    instances,
    machines,
    instancesSupported,
    projectIds: [session?.projectId, sessionId].filter((id): id is string => Boolean(id)),
    isOwner: sessionIsOwned(session),
  }), [instances, instancesSupported, machines, paneConfigs, session, sessionId]);
  const selectedHost = selectedPaneId === null ? undefined : tabHosts.tabs.get(selectedPaneId);
  const selectedRefusal = selectedHost?.refusal;
  const machineChoices = useMemo(() => buildMachineChoices({
    projectId: session?.projectId ?? sessionId ?? "",
    instances,
    machines,
    instancesSupported,
    canClone: sessionCanCloneJoin(session),
    selectedPane: selectedPane ?? null,
    fallbackMachineId: machineProjectTarget?.machineId,
  }), [instances, instancesSupported, machineProjectTarget?.machineId, machines, selectedPane, session, sessionId]);
  // The user's pick, else the default (the selected pane's machine), derived
  // so a default that arrives while the sheet is open still counts. A pick
  // that disappears while the sheet is open (an instance detaches) is no pick.
  const selectedMachineKey = machineKey ?? machineChoices.defaultChoice?.key ?? null;
  const selectedMachine = machineChoices.showChoice
    ? machineChoices.choices.find((choice) => choice.key === selectedMachineKey && !choice.disabledReason)
    : undefined;
  const needsMachine = machineChoices.showChoice && !selectedMachine;
  const launches = useMemo(
    () => Object.values(addPaneRequests)
      .filter((request) => request.sessionId === sessionId)
      .sort((a, b) => a.startedAt - b.startedAt),
    [addPaneRequests, sessionId],
  );

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

  useEffect(() => {
    if (!renameTarget || renameTargetExists) return;
    setRenameTarget(null);
    setRenameDraft("");
  }, [renameTarget, renameTargetExists]);

  const activity = useMemo(() => {
    const items: ActivityItem[] = messages
      .filter((message) => !isRoutineSystemUpdate(message) && !(message.outputType?.type === "tool_result" && parseOmpQuestionResult(message.content)))
      .map((message) => ({ key: `0:${message.id}`, message, paneId: MAIN_PANE_ID }));
    for (const [rawPaneId, paneActivity] of Object.entries(paneMessages)) {
      const paneId = Number(rawPaneId);
      for (const message of paneActivity) {
        if (!isRoutineSystemUpdate(message) && !(message.outputType?.type === "tool_result" && parseOmpQuestionResult(message.content))) {
          items.push({ key: `${paneId}:${message.id}`, message, paneId });
        }
      }
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

  const selectedDeliveries = useMemo(
    () => pendingDeliveries.filter((entry) => entry.sessionId === sessionId && entry.paneId === selectedPaneId),
    [pendingDeliveries, sessionId, selectedPaneId],
  );
  // A refused send's outcome is known; only the others wait on the transcript.
  const awaitingTranscript = pendingDeliveries.some((entry) => !entry.error);
  // Nothing else re-renders when the grace period simply elapses.
  useEffect(() => {
    if (!awaitingTranscript) return;
    const timer = window.setInterval(() => setDeliveryNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [awaitingTranscript]);
  const unconfirmed = useMemo(
    () => unconfirmedDeliveries(selectedDeliveries, deliveryNow),
    [selectedDeliveries, deliveryNow],
  );
  const refused = useMemo(() => refusedDeliveries(selectedDeliveries), [selectedDeliveries]);

  const firstVisibleActivityKey = visibleActivity[0]?.key;
  const lastVisibleActivityKey = visibleActivity.at(-1)?.key;

  const persistActivityScroll = useCallback(() => {
    if (!sessionId || selectedPaneId === null || !activityScrollRef.current) return;
    writeActivityScroll(sessionId, selectedPaneId, activityScrollRef.current, followNewestRef.current);
  }, [selectedPaneId, sessionId]);

  const openRawTerminal = useCallback(() => {
    if (selectedPaneId === null || !sessionId) return;
    persistActivityScroll();
    setMountedTerminal({ sessionId, paneId: selectedPaneId });
    setTerminalPaneId(selectedPaneId);
  }, [persistActivityScroll, selectedPaneId, sessionId]);

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
    if (!sessionId || selectedPaneId === null || !element || element.closest("[hidden]") || visibleActivity.length === 0) return;

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

  const closePane = (pane: PaneConfig, cleanup?: PaneCleanupAction) => {
    removePane(pane.pane_id, cleanup);
    setCloseTarget(null);
    if (pane.pane_id === selectedPaneId) {
      // Otherwise the screen keeps steering a pane that is going away.
      const remaining = paneConfigs.filter((candidate) => candidate.pane_id !== pane.pane_id);
      setSelectedPaneId(remaining.length > 0 ? remaining[0].pane_id : null);
    }
  };

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
    if (selectedRefusal) {
      setActionError(selectedRefusal);
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
    // The machine step's pick, else the default (the selected pane's
    // machine). A hidden step's default is a running instance, and is still
    // sent so the pane cannot land elsewhere; a default that would start the
    // project is never hidden (`buildMachineChoices`).
    const target = selectedMachine ?? machineChoices.defaultChoice;
    if (target?.kind === "clone") {
      // Cloning onto a machine is confirmed on its own sheet first.
      const gitRemote = session?.gitRemote;
      setNewPaneOpen(false);
      if (!gitRemote) {
        setActionError("This project has no repository to clone.");
        return;
      }
      setCloneTarget({ choice: target, option, copy: cloneHereCopy(target.hostname, gitRemote) });
      return;
    }
    if (!target && machineChoices.showChoice) {
      setActionError("Choose where the new agent runs.");
      return;
    }
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
      target?.machineId,
    );
    if (!result.success) {
      setActionError(result.error || "The pane could not be created.");
      setNewPaneOpen(false);
      return;
    }
    setActionError(null);
    setNewPaneOpen(false);
  };

  const confirmClone = () => {
    if (!cloneTarget) return;
    const { choice, option } = cloneTarget;
    setCloneTarget(null);
    const gitRemote = session?.gitRemote;
    if (!choice.machineId || !gitRemote) {
      setActionError("This project has no repository to clone.");
      return;
    }
    const result = cloneAndAddPane(
      {
        machineId: choice.machineId,
        gitRemote,
        instanceName: cloneInstanceName(gitRemote),
        cloneUrl: session?.gitRemoteUrl,
      },
      {
        provider: option.provider,
        mode: "interactive",
        label: `${option.label} ${paneConfigs.length + 1}`,
        model: option.model,
        isolatedWorktree: false,
        managed: false,
        kind: option.kind,
      },
    );
    setActionError(result.success ? null : result.error || "The clone could not be requested.");
  };

  // Start on <host> starts that machine only, as the Machines page's Start
  // does; the project's other machines are untouched.
  const startOnMachine = (machineId: string, projectId: string, hostname: string) => {
    startMachineProjectCli(machineId, projectId);
    setActionError(null);
    showToast(`Starting this project on ${hostname}…`, "info");
  };

  const confirmForget = () => {
    if (!forgetTarget || !sessionId) return;
    forgetMachinePanes(sessionId, forgetTarget.machineId);
    setForgetTarget(null);
  };

  const dismissRename = () => {
    setRenameTarget(null);
    setRenameDraft("");
  };

  const saveRename = () => {
    const trimmed = renameDraft.trim();
    if (
      !renameTarget
      || !renameTargetExists
      || !connected
      || !sessionIsRunning
      || !trimmed
    ) {
      if (!renameTargetExists) dismissRename();
      return;
    }
    updatePaneLabel(renameTarget.pane_id, trimmed);
    dismissRename();
  };


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
  const canOpenProject = connected
    && !sessionIsRunning
    && machineProjectTarget !== null
    && !machineProjectTarget.isRunning;
  const canCompose = selectedPaneId !== null && !selectedIsBot;
  const canSend = canCompose && connected && sessionIsRunning && !selectedRefusal;
  const hostLabel = projectHostLabel(instances, session.hostname);
  const stoppedActions = selectedHost?.stopped;
  const projectId = (session.projectId !== session.id ? session.projectId : undefined)
    ?? machineProjectTarget?.projectId ?? session.projectId;
  const projectContext = projectId ? {
    id: projectId,
    name: projectName,
    machineId: machineProjectTarget?.projectId === projectId ? machineProjectTarget?.machineId : undefined,
  } : undefined;
  const retainedTerminal = mountedTerminal?.sessionId === sessionId
    && paneConfigs.some((pane) => pane.pane_id === mountedTerminal.paneId && pane.kind === "terminal")
    ? mountedTerminal : null;
  const terminalIsOpen = terminalPaneId !== null && retainedTerminal?.paneId === terminalPaneId;
  // The retained terminal's own pane, which need not be the selected one.
  // Its screen still reads, but keys and sizes would only be refused.
  const retainedRefusal = retainedTerminal
    ? tabHosts.tabs.get(retainedTerminal.paneId)?.refusal
    : undefined;
  // The machines the Open sheet's request starts; empty means only the one it
  // names, which is all an older server ever starts.
  const openProjectStartHosts = openProjectTarget && instancesSupported
    ? projectStartHosts({ panes: paneConfigs, machines, projectId: openProjectTarget.projectId })
    : [];

  return (
    <ProjectJobsWorkspace key={`${accountId}:${projectId ?? sessionId}`} project={projectContext} paneSelectionRequested={false}>
    {(workspace) => (
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
      <div hidden={terminalIsOpen && !workspace.active} className={terminalIsOpen && !workspace.active ? "hidden" : "shrink-0 border-b border-[#dedee7] px-4 pt-3 pb-2.5 dark:border-[#383842]"}>
        <div className="flex items-center gap-2" onClickCapture={persistActivityScroll}>
          <button type="button" aria-label="Back to coding sessions" onClick={onBack} className="-ml-2 flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-[#686873] hover:bg-[#efeff5] dark:text-[#aaaab6] dark:hover:bg-[#25252d]"><ArrowLeft className="h-5 w-5" /></button>
          <div className="no-scrollbar flex min-w-0 flex-1 items-center gap-2 overflow-x-auto">
            {workspace.overviewTab}
            {paneConfigs.length > 0 ? paneConfigs.map((pane) => {
              const selected = pane.pane_id === selectedPaneId && !workspace.active;
              const paneStatus = paneStatuses[paneKey(pane.pane_id)];
              const statusSuffix = paneIsAwaitingAnswerStatus(paneStatus)
                ? " · pending answer"
                : paneStatus
                  ? " · working"
                  : "";
              // The pane's machine, once the project spans machines or the
              // pane cannot be used there.
              const host = tabHosts.tabs.get(pane.pane_id);
              const hostname = host?.badge ? host.hostname : undefined;
              const hostState = host && host.state !== "running" ? tabHostStateLabel(host.state) : undefined;
              // A chip naming a machine puts it on a second line, and is never
              // wider than the row: on one line a phone's row (about 200px)
              // could not hold even "Claude 2 · zoo-006 (stopped)", and a long
              // name pushed the machine's state out of view. Each line gives
              // way on its own; the state and the pane's status stay whole.
              // Flex items drop edge spaces, hence whitespace-pre; the name
              // read out is the whole text, which a truncated chip no longer
              // shows.
              const chipName = hostname
                ? `${paneLabel(pane)} · ${hostname}${hostState ? ` (${hostState})` : ""}${statusSuffix}`
                : undefined;
              return (
                <button key={pane.pane_id} type="button" aria-pressed={selected} aria-label={chipName} title={host?.refusal} onClick={() => { workspace.selectInteractive(); selectPane(pane.pane_id); }} className={`min-h-11 shrink-0 rounded-full border px-3 py-1.5 text-xs font-bold ${hostname ? "flex max-w-full flex-col items-center justify-center leading-tight" : ""} ${selected ? "border-[#6d5efc] text-[#6d5efc]" : "border-[#dedee7] text-[#686873] dark:border-[#383842] dark:text-[#aaaab6]"} ${host?.refusal && !selected ? "opacity-60" : ""}`}>
                  {hostname ? (
                    <>
                      <span className="max-w-full truncate">{paneLabel(pane)}</span>
                      <span className="flex max-w-full text-[11px] font-normal">
                        <span className="min-w-0 truncate">{hostname}</span>
                        {hostState && <span className="shrink-0 whitespace-pre">{` · ${hostState}`}</span>}
                        {statusSuffix && <span className="shrink-0 whitespace-pre">{statusSuffix}</span>}
                      </span>
                    </>
                  ) : (
                    <>
                      {paneLabel(pane)}
                      {statusSuffix}
                    </>
                  )}
                </button>
              );
            }) : (
              <span className="shrink-0 text-xs text-[#686873] dark:text-[#aaaab6]">Waiting for panes…</span>
            )}
            {workspace.tabs}
          </div>
          <button type="button" aria-label="Add agent" onClick={() => { setInteractiveCreation(false); setMachineKey(null); setNewPaneOpen(true); }} className="inline-flex h-11 w-11 shrink-0 items-center justify-center rounded-full bg-[#6d5efc] text-white"><Plus className="h-4 w-4" /></button>
          {!workspace.active && <button type="button" aria-label="More actions" onClick={() => setMoreOpen(true)} className="-mr-2 flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-[#686873] hover:bg-[#efeff5] dark:text-[#aaaab6] dark:hover:bg-[#25252d]"><MoreVertical className="h-5 w-5" /></button>}
        </div>

        <div className="mt-1.5 flex items-center gap-2">
          {/* Still the screen's heading, just no longer the largest thing on
              it — losing the h1 would leave the screen without one. */}
          {/* The heading is the project alone; the target rides beside it, so
              the screen keeps a heading that names one thing. */}
          <div className="flex min-w-0 flex-1 items-baseline gap-1.5">
            <h1 className="shrink-0 max-w-[55%] truncate text-sm font-bold">{projectName}</h1>
            <span className="min-w-0 flex-1 truncate text-xs text-[#686873] dark:text-[#aaaab6]" title={hostLabel.title}>
              {hostLabel.text || session.workingDir || "Unknown target"}
            </span>
          </div>
          <span className={`shrink-0 rounded-full px-2.5 py-1 text-[0.68rem] font-extrabold ${session.isActive ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300" : "bg-[#efeff5] text-[#686873] dark:bg-[#25252d] dark:text-[#aaaab6]"}`}>
            {session.isActive ? "Active" : session.status}
          </span>
        </div>
      </div>

      {actionError && <div className="mx-4 mt-3 flex shrink-0 items-center gap-2 rounded-xl border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300"><AlertTriangle className="h-4 w-4 shrink-0" /> {actionError}</div>}

      {/* The selected pane belongs to a machine that is not running the
          project, or that this viewer may not use. Its history still reads;
          say which machine it needs and offer it. */}
      {selectedRefusal && !workspace.active && !terminalIsOpen && (
        <div role="status" className="mx-4 mt-3 shrink-0 rounded-xl border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          <p className="flex items-start gap-2"><AlertTriangle aria-hidden="true" className="mt-px h-4 w-4 shrink-0" /> {selectedRefusal}</p>
          {(stoppedActions?.canStart || stoppedActions?.canForget) && (
            <div className="mt-2 flex flex-wrap gap-2">
              {stoppedActions.canStart && (
                <button
                  type="button"
                  disabled={!connected || Boolean(stoppedActions.startDisabledReason) || !stoppedActions.projectId}
                  title={stoppedActions.startDisabledReason}
                  onClick={() => {
                    if (stoppedActions.projectId) startOnMachine(stoppedActions.machineId, stoppedActions.projectId, stoppedActions.hostname);
                  }}
                  className="min-h-11 rounded-xl bg-[#6d5efc] px-3 py-2 text-xs font-bold text-white disabled:opacity-40"
                >
                  Start on {stoppedActions.hostname}
                </button>
              )}
              {stoppedActions.canForget && (
                <button
                  type="button"
                  disabled={!connected}
                  onClick={() => setForgetTarget({
                    machineId: stoppedActions.machineId,
                    hostname: stoppedActions.hostname,
                    copy: forgetMachineTabsCopy(stoppedActions.hostname),
                  })}
                  className="min-h-11 rounded-xl border border-amber-300 px-3 py-2 text-xs font-bold disabled:opacity-40 dark:border-amber-800"
                >
                  Forget {stoppedActions.hostname}&apos;s tabs
                </button>
              )}
            </div>
          )}
          {stoppedActions?.canStart && stoppedActions.startDisabledReason && (
            <p className="mt-1.5">{stoppedActions.startDisabledReason}</p>
          )}
        </div>
      )}

      <AddPaneLaunches launches={launches} onDismiss={dismissAddPaneRequest} variant="mobile" />

      {workspace.content}
      {retainedTerminal && (
        <section hidden={!terminalIsOpen || workspace.active} aria-label="Mobile terminal" className={terminalIsOpen && !workspace.active ? "flex flex-1 min-h-0 flex-col bg-[#0a0a0a] text-white" : "hidden"}>
          <div className="flex shrink-0 items-center justify-between border-b border-neutral-800 px-2 py-1.5">
            <button type="button" onClick={() => setTerminalPaneId(null)} className="flex min-h-11 items-center gap-2 rounded-lg px-2 py-2 text-sm font-bold hover:bg-neutral-800"><ArrowLeft className="h-5 w-5" /> Conversation</button>
            <span className="truncate px-2 text-xs text-neutral-400">{selectedPane ? paneLabel(selectedPane) : `Pane ${retainedTerminal.paneId}`}</span>
          </div>
          {/* The session's refusal banner is hidden while the terminal is
              open, so the terminal says why it takes no input. */}
          {retainedRefusal && (
            <p role="status" className="flex shrink-0 items-start gap-2 border-b border-neutral-800 px-3 py-2 text-xs text-amber-300">
              <AlertTriangle aria-hidden="true" className="mt-px h-4 w-4 shrink-0" /> {retainedRefusal}
            </p>
          )}
          <div className="flex min-h-0 flex-1 flex-col"><TerminalPane key={`${sessionId}:${retainedTerminal.paneId}`} paneId={retainedTerminal.paneId} visible={terminalIsOpen && !workspace.active} inputDisabledReason={retainedRefusal} /></div>
          <MobileTerminalKeyBar paneId={retainedTerminal.paneId} connected={connected} disabledReason={retainedRefusal} />
        </section>
      )}
      <div hidden={workspace.active || terminalIsOpen} className={workspace.active || terminalIsOpen ? "hidden" : "flex min-h-0 flex-1 flex-col"}>

      <div ref={activityScrollRef} role="log" aria-label="Conversation activity" onScroll={handleActivityScroll} className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        {selectedPlanReviews.length > 0 && (
          <div className="mb-3 space-y-2">
            {selectedPlanReviews.map((plan) => {
              // Answering optimistically drops the review, while the server
              // refuses it for a machine that is not running the project.
              const planRefusal = tabHosts.tabs.get(plan.paneId)?.refusal;
              return (
                <div key={plan.toolUseId} className="rounded-2xl border border-amber-400 bg-white p-3.5 dark:border-amber-700 dark:bg-[#1b1b21]">
                  <span className="rounded-full bg-amber-100 px-2.5 py-1 text-[0.68rem] font-extrabold uppercase text-amber-800 dark:bg-amber-950/60 dark:text-amber-200">plan review</span>
                  <p className="mt-2 text-sm font-semibold">Pane {plan.paneId} wants to use {plan.toolName}.</p>
                  {planRefusal && <p role="status" className="mt-2 text-sm text-amber-800 dark:text-amber-200">{planRefusal}</p>}
                  <div className="mt-3 flex gap-2"><button type="button" disabled={Boolean(planRefusal)} title={planRefusal} onClick={() => answerPlanReview(plan.toolUseId, false)} className="flex-1 rounded-xl border border-red-300 py-2 text-sm font-bold text-red-600 disabled:opacity-40 dark:border-red-900 dark:text-red-300">Deny</button><button type="button" disabled={Boolean(planRefusal)} title={planRefusal} onClick={() => answerPlanReview(plan.toolUseId, true)} className="flex-1 rounded-xl bg-emerald-600 py-2 text-sm font-bold text-white disabled:opacity-40">Approve</button></div>
                </div>
              );
            })}
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
            {visibleActivity.map((item) => <MobileEventCard key={item.key} item={item} answeredQuestions={answeredQuestions} onApprove={approve} onReject={reject} disabledReason={tabHosts.tabs.get(item.paneId)?.refusal} />)}
          </div>
        ) : (
          <div className="flex min-h-60 flex-col items-center justify-center px-5 text-center">
            <div className="rounded-2xl bg-[#efeff5] p-3 dark:bg-[#25252d]">{connected ? <Wifi className="h-6 w-6 text-[#686873]" /> : <WifiOff className="h-6 w-6 text-[#686873]" />}</div>
            {connected && selectedIsTerminal ? (
              <>
                {/* A terminal pane records every turn, so none at all means no
                    conversation has started — it is very likely still at a
                    startup prompt, where typed prose does something else
                    entirely. Saying "No activity yet" made that look like a
                    healthy, quiet agent. */}
                <h2 className="mt-3 text-lg font-extrabold">This agent hasn&apos;t started a conversation yet</h2>
                <p className="mt-1 max-w-sm text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">
                  It may still be starting up, or waiting at a prompt of its own — asking whether to
                  resume a previous session, for example. Messages typed here go to it as
                  keystrokes, so they will not do what you mean until it is ready.
                </p>
                <button
                  type="button"
                  aria-label="Open raw terminal to finish setting up this agent"
                  onClick={openRawTerminal}
                  className="mt-4 inline-flex items-center gap-2 rounded-xl bg-[#6d5efc] px-4 py-2.5 text-sm font-bold text-white"
                >
                  <SquareTerminal className="h-4 w-4" /> Open raw terminal
                </button>
              </>
            ) : (
              <>
                <h2 className="mt-3 text-lg font-extrabold">{connected ? "No activity yet" : "No cached activity"}</h2>
                <p className="mt-1 text-sm text-[#686873] dark:text-[#aaaab6]">{connected ? "Instructions and agent activity for this pane will appear here." : "Reconnect to retrieve this pane's activity."}</p>
              </>
            )}
          </div>
        )}
      </div>

      <div className="shrink-0 border-t border-[#dedee7] bg-[#f7f7fa] p-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] dark:border-[#383842] dark:bg-[#111115]">
        {refused.map((entry) => (
          // The server refused it, so it is neither unconfirmed nor waiting:
          // show why, and keep its text to restore. It is never resent.
          <div key={entry.id} role="status" aria-live="polite" className="mb-1.5 flex items-start gap-2 rounded-xl border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-800 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200">
            <AlertTriangle aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />
            <span className="min-w-0 flex-1">
              Your message was not delivered: {entry.error}
              {!followUp && <button type="button" onClick={() => {
                setFollowUp(entry.text);
                dismissTerminalDelivery(entry.id);
              }} className="ml-2 underline font-bold">Restore draft</button>}
              <button type="button" onClick={() => dismissTerminalDelivery(entry.id)} className="ml-2 underline font-bold">Dismiss</button>
            </span>
          </div>
        ))}
        {selectedDeliveries.length > refused.length && unconfirmed.length === 0 && (
          <p role="status" className="mb-1.5 text-xs text-[#686873] dark:text-[#aaaab6]">Sent to terminal. Waiting for the agent to record it.</p>
        )}
        {unconfirmed.length > 0 && (
          <div role="status" aria-live="polite" className="mb-1.5 flex items-start gap-2 rounded-xl border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
            <AlertTriangle aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />
            <span className="min-w-0 flex-1">
              {unconfirmed.length === 1
                ? "No confirmation that the agent recorded your last message."
                : `No confirmation that the agent recorded ${unconfirmed.length} of your messages.`}
              {" "}
              A terminal dialog may have consumed it, or recording may be delayed. Check the terminal before sending again.
              <button type="button" aria-label="Open raw terminal to check this message" onClick={openRawTerminal} className="ml-1 underline font-bold">
                Open raw terminal
              </button>
              {!followUp && <button type="button" onClick={() => {
                const entry = unconfirmed[0];
                setFollowUp(entry.text);
                dismissTerminalDelivery(entry.id);
              }} className="ml-2 underline font-bold">Restore draft</button>}
              <button type="button" onClick={() => unconfirmed.forEach((entry) => dismissTerminalDelivery(entry.id))} className="ml-2 underline font-bold">Dismiss</button>
            </span>
          </div>
        )}
        {connected && selectedStatus && <div role="status" aria-live="polite" className={`mb-1.5 flex w-fit max-w-full items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-bold ${selectedIsAwaitingAnswer ? "bg-amber-100 text-amber-800 dark:bg-amber-950/60 dark:text-amber-200" : "bg-[#eeecff] text-[#5b4de0] dark:bg-[#292452] dark:text-[#c8c1ff]"}`}>{selectedIsAwaitingAnswer ? <AlertTriangle aria-hidden="true" className="h-3.5 w-3.5 shrink-0" /> : <LoaderCircle aria-hidden="true" className="h-3.5 w-3.5 shrink-0 animate-spin" />}<span className="truncate">{selectedStatus}</span></div>}
        {!connected && canCompose && <p className="mb-1.5 text-[11px] text-amber-700 dark:text-amber-300">You can keep drafting while offline. Reconnect to send.</p>}
        {connected && !sessionIsRunning && canCompose && <p className="mb-1.5 text-[11px] text-amber-700 dark:text-amber-300">You can draft a message, but this project must be running before it can be sent.</p>}
        {connected && sessionIsRunning && canCompose && selectedRefusal && <p className="mb-1.5 text-[11px] text-amber-700 dark:text-amber-300">You can draft a message, but it cannot be sent: {selectedRefusal}</p>}
        <div className="flex items-end gap-2">
          <textarea value={followUp} onChange={(event) => setFollowUp(event.target.value)} rows={2} placeholder={selectedIsBot ? "Stop the bot before steering it" : selectedIsTerminal ? "Message this terminal conversation" : "Steer this session and pane"} disabled={!canCompose} className="min-h-[2.75rem] flex-1 resize-none rounded-xl border border-[#dedee7] bg-white px-3 py-2 text-sm outline-none focus:border-[#6d5efc] disabled:opacity-50 dark:border-[#383842] dark:bg-[#1b1b21]" />
          <button type="button" aria-label={selectedIsTerminal ? "Send conversation message" : "Send follow-up"} disabled={!canSend || !followUp.trim()} onClick={sendFollowUp} className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-[#6d5efc] text-white disabled:opacity-40"><Send className="h-5 w-5" /></button>
        </div>
      </div>
      </div>

      {newPaneOpen && (
        <div className="fixed inset-0 z-[95] flex items-end bg-black/45" onClick={() => setNewPaneOpen(false)}>
          <div role="dialog" aria-modal="true" aria-label="Add agent" onClick={(event) => event.stopPropagation()} className="max-h-[82dvh] w-full overflow-y-auto rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]">
            <div className="flex items-center justify-between"><div><h2 className="text-xl font-extrabold">{interactiveCreation ? "Interactive agent" : "Add agent"}</h2><p className="mt-1 text-sm text-[#686873] dark:text-[#aaaab6]">{interactiveCreation ? "Choose a profile allowed by this project's cluster policy." : "Choose interactive or bounded autonomous work."}</p></div><button type="button" aria-label="Close add agent" onClick={() => setNewPaneOpen(false)} className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg hover:bg-[#efeff5] dark:hover:bg-[#25252d]"><X className="h-5 w-5" /></button></div>
            {!interactiveCreation ? <div className="mt-4 space-y-2">
              <button type="button" onClick={() => setInteractiveCreation(true)} className="min-h-11 w-full rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left dark:border-[#383842] dark:bg-[#1b1b21]"><span className="block font-bold">Interactive agent</span><span className="mt-1 block text-sm">Work directly with a provider in a terminal.</span></button>
              <button type="button" disabled={!workspace.addAutonomous} onClick={() => { persistActivityScroll(); setNewPaneOpen(false); workspace.addAutonomous?.(); }} className="min-h-11 w-full rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left disabled:opacity-50 dark:border-[#383842] dark:bg-[#1b1b21]"><span className="block font-bold">Autonomous agent</span><span className="mt-1 block text-sm">{workspace.unavailableReason || "Review a bounded task, worker readiness and limits."}</span></button>
            </div> : <>
            <button type="button" onClick={() => setInteractiveCreation(false)} className="mt-2 min-h-11 text-sm font-bold text-[#6d5efc]">Back to agent types</button>
            {/* Where the agent runs, when there is a real choice: the same
                choices, groups and reasons as the desktop new-tab menu. */}
            {machineChoices.showChoice && (
              <fieldset className="mt-3">
                <legend className="text-sm font-bold">Run on</legend>
                {groupMachineChoices(machineChoices.choices).map((group) => (
                  <div key={group.key} role="group" aria-label={group.label} className="mt-2 space-y-2">
                    <p aria-hidden="true" className="text-[0.68rem] font-extrabold uppercase tracking-wide text-[#686873] dark:text-[#aaaab6]">{group.label}</p>
                    {group.choices.map((choice) => (
                      <label key={choice.key} title={choice.disabledReason} className={`flex min-h-11 items-start gap-3 rounded-2xl border p-3 ${selectedMachineKey === choice.key && !choice.disabledReason ? "border-[#6d5efc]" : "border-[#dedee7] dark:border-[#383842]"} bg-white dark:bg-[#1b1b21] ${choice.disabledReason ? "opacity-50" : ""}`}>
                        <input
                          type="radio"
                          name="mobile-new-agent-machine"
                          value={choice.key}
                          checked={selectedMachineKey === choice.key}
                          disabled={Boolean(choice.disabledReason)}
                          onChange={() => setMachineKey(choice.key)}
                          className="mt-1 h-4 w-4 shrink-0 accent-[#6d5efc]"
                        />
                        <span className="min-w-0">
                          <span className="block truncate font-bold">{choice.hostname}</span>
                          <span className="mt-0.5 block text-xs text-[#686873] dark:text-[#aaaab6]">{machineChoiceDetail(choice)}</span>
                        </span>
                      </label>
                    ))}
                  </div>
                ))}
              </fieldset>
            )}
            {needsMachine && <p className="mt-3 text-xs text-[#686873] dark:text-[#aaaab6]">Choose where the agent runs first.</p>}
            <div className="mt-4 space-y-2">
              {launchOptions.map((option) => <button key={option.key} type="button" disabled={needsMachine} onClick={() => { workspace.selectInteractive(); createPane(option); }} className="flex w-full items-center justify-between rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left disabled:opacity-40 dark:border-[#383842] dark:bg-[#1b1b21]"><span><span className="block font-bold">{option.label}</span><span className="mt-0.5 block text-xs text-[#686873] dark:text-[#aaaab6]">{option.kind === "terminal" ? "Interactive TUI" : "Coding activity pane"}</span></span><Plus className="h-5 w-5 text-[#6d5efc]" /></button>)}
              {launchOptions.length === 0 && <div className="rounded-2xl border border-[#dedee7] bg-white p-4 text-sm text-[#686873] dark:border-[#383842] dark:bg-[#1b1b21] dark:text-[#aaaab6]">Waiting for the project's launch policy, or no profiles are currently allowed.</div>}
            </div>
            </>}
          </div>
        </div>
      )}

      {moreOpen && (
        <div className="fixed inset-0 z-[95] flex items-end bg-black/45" onClick={() => setMoreOpen(false)}>
          <div role="dialog" aria-modal="true" aria-label="Pane actions" onClick={(event) => event.stopPropagation()} className="w-full rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]">
            <div className="flex items-center justify-between">
              <h2 className="text-lg font-extrabold">{selectedPane ? paneLabel(selectedPane) : "No pane selected"}</h2>
              <button type="button" aria-label="Close actions" onClick={() => setMoreOpen(false)} className="rounded-lg p-2 hover:bg-[#efeff5] dark:hover:bg-[#25252d]"><X className="h-5 w-5" /></button>
            </div>
            <div className="mt-3 space-y-2">
              {!sessionIsRunning && (
                <button
                  type="button"
                  aria-label="Open project"
                  disabled={!canOpenProject}
                  title={!connected
                    ? "Reconnect before opening this project"
                    : !machineProjectTarget
                      ? "No unambiguous machine was found for this project"
                      : machineProjectTarget.isRunning
                        ? "This project is already starting"
                        : "Open this project"}
                  onClick={() => {
                    if (!canOpenProject || !machineProjectTarget) return;
                    setMoreOpen(false);
                    setOpenProjectTarget(machineProjectTarget);
                  }}
                  className="flex w-full items-center gap-3 rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left font-bold disabled:opacity-40 dark:border-[#383842] dark:bg-[#1b1b21]"
                >
                  <FolderOpen className="h-5 w-5 shrink-0 text-[#6d5efc]" /> Open project
                </button>
              )}
              <button type="button" aria-label="Open raw terminal" disabled={!selectedIsTerminal} onClick={() => {
                if (selectedPaneId === null) return;
                setMoreOpen(false);
                openRawTerminal();
              }} className="flex w-full items-center gap-3 rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left font-bold disabled:opacity-40 dark:border-[#383842] dark:bg-[#1b1b21]"><SquareTerminal className="h-5 w-5 shrink-0 text-[#686873] dark:text-[#aaaab6]" /> Raw terminal</button>
              {summarySupported && (
                <button type="button" aria-label="Open work summary" disabled={selectedPaneId === null} onClick={() => { setMoreOpen(false); setSummaryOpen(true); }} className="flex w-full items-center gap-3 rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left font-bold disabled:opacity-40 dark:border-[#383842] dark:bg-[#1b1b21]"><History className="h-5 w-5 shrink-0 text-[#686873] dark:text-[#aaaab6]" /> Work summary</button>
              )}
              <button
                type="button"
                aria-label="Rename conversation"
                disabled={!selectedPane || !connected || !sessionIsRunning || Boolean(selectedRefusal)}
                title={!selectedPane
                  ? "Select a pane to rename its conversation"
                  : !connected
                    ? "Reconnect before renaming this conversation"
                    : !sessionIsRunning
                      ? "Open this project before renaming its conversation"
                      : selectedRefusal ?? "Rename this conversation"}
                onClick={() => {
                  if (!selectedPane || !connected || !sessionIsRunning || selectedRefusal) return;
                  setMoreOpen(false);
                  setRenameTarget(selectedPane);
                  setRenameDraft(paneLabel(selectedPane));
                }}
                className="flex w-full items-center gap-3 rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left font-bold disabled:opacity-40 dark:border-[#383842] dark:bg-[#1b1b21]"
              >
                <Pencil className="h-5 w-5 shrink-0 text-[#686873] dark:text-[#aaaab6]" /> Rename conversation
              </button>
              <button type="button" aria-label="Reboot this pane" disabled={!selectedPane || Boolean(selectedRefusal)} title={selectedRefusal} onClick={() => { setMoreOpen(false); setRebootTarget(selectedPane ?? null); }} className="flex w-full items-center gap-3 rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left font-bold disabled:opacity-40 dark:border-[#383842] dark:bg-[#1b1b21]"><RotateCcw className="h-5 w-5 shrink-0 text-[#686873] dark:text-[#aaaab6]" /> Reboot this pane</button>
              <button type="button" aria-label="Close this pane" disabled={!selectedPane || Boolean(selectedRefusal)} title={selectedRefusal} onClick={() => { setMoreOpen(false); setCloseTarget(selectedPane ?? null); }} className="flex w-full items-center gap-3 rounded-2xl border border-red-200 bg-white p-3.5 text-left font-bold text-red-700 disabled:opacity-40 dark:border-red-900 dark:bg-[#1b1b21] dark:text-red-300"><Trash2 className="h-5 w-5 shrink-0" /> Close this pane</button>
              {/* Different scope from the four above, and the only place it can
                  live: project management is per-project, so there is no
                  session-independent screen to move it to. */}
              <button type="button" aria-label="Manage project" onClick={() => { setMoreOpen(false); setManageOpen(true); }} className="mt-1 flex w-full items-center gap-3 rounded-2xl border border-[#dedee7] bg-white p-3.5 text-left font-bold dark:border-[#383842] dark:bg-[#1b1b21]"><Settings2 className="h-5 w-5 shrink-0 text-[#686873] dark:text-[#aaaab6]" /> Manage project</button>
            </div>
          </div>
        </div>
      )}

      {renameTarget && (
        <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={dismissRename}>
          <form
            role="dialog"
            aria-modal="true"
            aria-label={`Rename ${paneLabel(renameTarget)}`}
            onClick={(event) => event.stopPropagation()}
            onSubmit={(event) => {
              event.preventDefault();
              saveRename();
            }}
            className="w-full rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]"
          >
            <div className="flex items-center justify-between">
              <div>
                <h2 className="text-lg font-extrabold">Rename conversation</h2>
                <p className="mt-1 text-sm text-[#686873] dark:text-[#aaaab6]">This changes the selected pane&apos;s name everywhere.</p>
              </div>
              <button type="button" aria-label="Close rename conversation" onClick={dismissRename} className="rounded-lg p-2 hover:bg-[#efeff5] dark:hover:bg-[#25252d]"><X className="h-5 w-5" /></button>
            </div>
            <label htmlFor="mobile-conversation-name" className="mt-4 block text-sm font-bold">Conversation name</label>
            <input
              id="mobile-conversation-name"
              value={renameDraft}
              onChange={(event) => setRenameDraft(event.target.value)}
              autoFocus
              onFocus={(event) => event.currentTarget.select()}
              className="mt-2 w-full rounded-xl border border-[#dedee7] bg-white px-3 py-2.5 text-base outline-none focus:border-[#6d5efc] dark:border-[#383842] dark:bg-[#1b1b21]"
            />
            <div className="mt-4 flex gap-2.5">
              <button type="button" onClick={dismissRename} className="flex-1 rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]">Cancel</button>
              <button
                type="submit"
                aria-label="Save conversation name"
                disabled={!renameDraft.trim() || !connected || !sessionIsRunning || !renameTargetExists}
                className="flex-1 rounded-xl bg-[#6d5efc] px-4 py-2.5 text-sm font-bold text-white disabled:opacity-40"
              >
                Save
              </button>
            </div>
          </form>
        </div>
      )}

      {openProjectTarget && (
        <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={() => setOpenProjectTarget(null)}>
          <div role="dialog" aria-modal="true" aria-label={`Open ${projectName}`} onClick={(event) => event.stopPropagation()} className="w-full rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]">
            <h2 className="text-lg font-extrabold">Open {projectName}?</h2>
            <p className="mt-1.5 text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">
              {/* Name the machines the server will start: every online one
                  that has the project's tabs, else the one the request names.
                  Not the session's recorded host, which may be a machine with
                  no tabs that was the last to stop. An older server starts
                  only the named machine. */}
              {`Its APAS process starts on ${hostList(openProjectStartHosts) || openProjectTarget.hostname || session.hostname || "the selected host"}, and its saved panes become available again.`}
            </p>
            <div className="mt-4 flex gap-2.5">
              <button type="button" onClick={() => setOpenProjectTarget(null)} className="flex-1 rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]">Cancel</button>
              <button type="button" onClick={() => {
                startMachineProjectCli(
                  openProjectTarget.machineId,
                  openProjectTarget.projectId,
                  { allInstances: true },
                );
                setOpenProjectTarget(null);
                setActionError(null);
              }} className="flex-1 rounded-xl bg-[#6d5efc] px-4 py-2.5 text-sm font-bold text-white">Open project</button>
            </div>
          </div>
        </div>
      )}

      {cloneTarget && (
        <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={() => setCloneTarget(null)}>
          <div role="dialog" aria-modal="true" aria-label={cloneTarget.copy.title} onClick={(event) => event.stopPropagation()} className="w-full rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]">
            <h2 className="text-lg font-extrabold">{cloneTarget.copy.title}</h2>
            {cloneTarget.copy.body.map((paragraph) => (
              <p key={paragraph} className="mt-1.5 break-words text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">{paragraph}</p>
            ))}
            <div className="mt-4 flex gap-2.5">
              <button type="button" onClick={() => setCloneTarget(null)} className="flex-1 rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]">Cancel</button>
              <button type="button" disabled={!connected} onClick={confirmClone} className="flex-1 rounded-xl bg-[#6d5efc] px-4 py-2.5 text-sm font-bold text-white disabled:opacity-40">Clone and start</button>
            </div>
          </div>
        </div>
      )}

      {forgetTarget && (
        <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={() => setForgetTarget(null)}>
          <div role="dialog" aria-modal="true" aria-label={forgetTarget.copy.title} onClick={(event) => event.stopPropagation()} className="w-full rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]">
            <h2 className="text-lg font-extrabold">{forgetTarget.copy.title}</h2>
            {forgetTarget.copy.body.map((paragraph) => (
              <p key={paragraph} className="mt-1.5 text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">{paragraph}</p>
            ))}
            <div className="mt-4 flex gap-2.5">
              <button type="button" onClick={() => setForgetTarget(null)} className="flex-1 rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]">Cancel</button>
              <button type="button" disabled={!connected} onClick={confirmForget} className="flex-1 rounded-xl bg-red-600 px-4 py-2.5 text-sm font-bold text-white disabled:opacity-40">Forget tabs</button>
            </div>
          </div>
        </div>
      )}

      {rebootTarget && (
        <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={() => setRebootTarget(null)}>
          <div role="dialog" aria-modal="true" aria-label={`Reboot ${paneLabel(rebootTarget)}`} onClick={(event) => event.stopPropagation()} className="w-full rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]">
            <h2 className="text-lg font-extrabold">Reboot {paneLabel(rebootTarget)}?</h2>
            {/* The reassurance is the point, and it is what the desktop
                promises too: the respawn is on the same session, so the agent
                comes back with its conversation rather than a blank one. */}
            <p className="mt-1.5 text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">
              The running process is killed and respawned on the same session, so the agent resumes
              with its prior conversation.
            </p>
            <div className="mt-4 flex gap-2.5">
              <button type="button" onClick={() => setRebootTarget(null)} className="flex-1 rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]">Cancel</button>
              <button type="button" onClick={() => { rebootPane(rebootTarget.pane_id); setRebootTarget(null); }} className="flex-1 rounded-xl bg-[#6d5efc] px-4 py-2.5 text-sm font-bold text-white">Reboot pane</button>
            </div>
          </div>
        </div>
      )}

      {closeTarget && (
        <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={() => setCloseTarget(null)}>
          <div role="dialog" aria-modal="true" aria-label={`Close ${paneLabel(closeTarget)}`} onClick={(event) => event.stopPropagation()} className="w-full rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]">
            <h2 className="text-lg font-extrabold">Close {paneLabel(closeTarget)}?</h2>
            <p className="mt-1.5 text-sm leading-5 text-[#686873] dark:text-[#aaaab6]">
              {closeTarget.worktree_path
                ? "This pane has its own worktree. Choose what happens to that work."
                : "The agent in this pane is stopped and the pane is removed."}
            </p>
            {closeTarget.worktree_path ? (
              <div className="mt-4 space-y-2">
                {([
                  ["merge_and_remove", "Merge into the branch, then remove"],
                  ["leave_as_branch", "Keep the branch, remove the pane"],
                  ["discard", "Discard the work"],
                ] as const).map(([action, label]) => (
                  <button key={action} type="button" onClick={() => closePane(closeTarget, action)} className={`w-full rounded-2xl border p-3.5 text-left font-bold ${action === "discard" ? "border-red-200 text-red-700 dark:border-red-900 dark:text-red-300" : "border-[#dedee7] dark:border-[#383842]"} bg-white dark:bg-[#1b1b21]`}>{label}</button>
                ))}
                <button type="button" onClick={() => setCloseTarget(null)} className="w-full rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]">Cancel</button>
              </div>
            ) : (
              <div className="mt-4 flex gap-2.5">
                <button type="button" onClick={() => setCloseTarget(null)} className="flex-1 rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]">Cancel</button>
                <button type="button" onClick={() => closePane(closeTarget)} className="flex-1 rounded-xl bg-red-600 px-4 py-2.5 text-sm font-bold text-white">Close pane</button>
              </div>
            )}
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
    )}
    </ProjectJobsWorkspace>
  );
}
