"use client";

import { useCallback, useId, useRef, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Bot, ChevronDown, LoaderCircle, RefreshCw, Repeat, Trash2 } from "lucide-react";
import { paneKey } from "@/lib/store";
import type { CliLifecycleInventory, CliLifecycleStatus, PaneConfig, PaneKind } from "@/lib/store";
import { useIsLaunchProfileAllowed } from "@/lib/tabTypes";
import { paneIsAwaitingAnswerStatus, paneIsWorkingStatus } from "@/lib/paneStatus";
import { providerOfferOn, type MachineChoice, type MachineChoiceModel, type ProviderOffer } from "@/lib/machineChoice";
import {
  groupMachineChoices,
  machineChoiceDetail,
  tabHostDescription,
  tabHostStateLabel,
  type RebootTarget,
  type TabHost,
  type TabHostState,
} from "@/lib/tabHosts";
import {
  DEEPSEEK_FLASH_MODEL,
  isDeepseekModel,
  isRetiredPiProvider,
  isRetiredProviderModel,
  PI_RETIRED_MESSAGE,
} from "@/lib/providerOptions";
import { autopilotMarkerLabel, type AutopilotListStatus } from "@/lib/autopilot";
import { withLinesBeforeLast, type ProjectCloseAction, type ProjectOpenAction } from "@/lib/projectOpenState";
import { useConfirmDialog } from "@/components/cluster/ConfirmDialog";
import { DELETE_PROJECT_LABEL } from "@/components/project/DeleteProjectDialog";

/**
 * The project's own controls in the tab bar: opening and closing it as a
 * whole, and deleting it. The actions and their confirmation texts come from
 * `buildProjectOpenModel`; a control is shown only when its action or
 * handler is present. There is no control for one machine alone.
 */
export interface ProjectControls {
  /** The project's state, "Open" or "Closed". Absent from a server that
   *  records no state: nothing is then shown beside the buttons. */
  openState?: "Open" | "Closed";
  /** The account that set the recorded state, where the server said. */
  recordedBy?: string;
  /** "Open" or "Open again". */
  openAction?: ProjectOpenAction;
  /** "Close" or "Close again". */
  closeAction?: ProjectCloseAction;
  /** Called once, after the person confirmed. */
  onOpen: () => void;
  onClose: () => void;
  /** Added to the Close confirmation, before its last line: which tabs are
   *  on autopilot and what becomes of their autopilot. */
  closeNote?: string;
  /** Present for the project's owner only: opens the Delete project dialog.
   *  Closing never calls it. */
  onDelete?: () => void;
}

interface TabBarProps {
  tabs: PaneConfig[];
  activeTabId: number | null;
  onSelectTab: (paneId: number) => void;
  onCloseTab: (paneId: number) => void;
  /** `machine` is the machine picked in the new-tab menu's machine step;
   *  absent when the step was not shown, which means the default machine. */
  onAddTab: (
    provider?: string,
    model?: string,
    isolatedWorktree?: boolean,
    kind?: PaneKind,
    machine?: MachineChoice,
  ) => void;
  jobTabs?: ReactNode;
  overviewTab?: ReactNode;
  onRenameTab?: (paneId: number, newLabel: string) => void;
  onReorderTabs?: (orderedIds: number[]) => void;
  /** Open, Close and Delete for the project as a whole. Absent: none of
   *  them is shown. */
  projectControls?: ProjectControls;
  /** Reboot one instance's CLI; absent machine = the primary's. */
  onRebootCli?: (machineId?: string) => void;
  lifecycleInventory?: CliLifecycleInventory;
  lifecycleStatus?: CliLifecycleStatus;
  /** One entry per attached instance, primary first. With more than one,
   *  the CLI menu offers a reboot per machine, each confirmed against that
   *  machine's own inventory. */
  rebootTargets?: RebootTarget[];
  showRebootButton?: boolean;
  /** Where each tab runs (see `describeTabHosts`). Without an entry a tab
   *  renders as it always has. */
  tabHosts?: Map<number, TabHost>;
  /** Forget one stopped machine's tabs; the handler confirms first. */
  onForgetMachine?: (machineId: string, hostname: string) => void;
  /** The new-tab machine step. Shown only when `showChoice`; otherwise the
   *  menu is exactly what it was with one machine. */
  machineChoices?: MachineChoiceModel;
  /** The tabs with an autopilot to show. One that is on, or paused, marks
   *  its tab; a tab without an entry renders as it always has. */
  autopilotTabs?: Map<number, AutopilotListStatus>;
  paneStatuses: Record<string, string | null>;
  pausedPanes: number[];
  /** Mobile-only controls merged into the tab-bar row so the top header can
   *  be hidden on phones. Rendered at the start (leading) and end (trailing)
   *  of the bar; callers gate their own visibility (e.g. md:hidden). */
  leading?: ReactNode;
  trailing?: ReactNode;
}

function rebootConfirmation(
  inventory?: CliLifecycleInventory,
  target?: { hostname: string; otherHosts: string[] },
): string {
  const on = target ? ` on ${target.hostname}` : "";
  // A daemon runs every project on its machine, so a reboot restarts all of
  // them there; the project's other machines are not touched.
  const scope = target
    ? [
        "",
        `Every project running on ${target.hostname} restarts with it.${target.otherHosts.length > 0
          ? ` The instance${target.otherHosts.length === 1 ? "" : "s"} on ${target.otherHosts.join(", ")} keep${target.otherHosts.length === 1 ? "s" : ""} running.`
          : ""}`,
      ]
    : [];
  if (!inventory) {
    return [
      `Reboot this older project CLI${on}?`,
      "",
      "All panes may restart. Upgrade the CLI on the project host to see preservation details and use transport-only reconnect.",
      ...scope,
    ].join("\n");
  }
  const adoptable = inventory.panes.filter((pane) => pane.mode === "live_adoptable").length;
  const restarting = inventory.panes.filter(
    (pane) => pane.mode === "restart_required_on_cli_reboot",
  ).length;
  const structured = inventory.panes.filter(
    (pane) => pane.mode === "structured_pane_may_resume",
  ).length;
  return [
    `Reboot the project CLI${on}?`,
    "",
    `${adoptable} terminal pane${adoptable === 1 ? "" : "s"} will stay live and be reattached.`,
    `${restarting} terminal pane${restarting === 1 ? "" : "s"} will restart and resume where supported.`,
    `${structured} structured pane${structured === 1 ? "" : "s"} may restart and resume.`,
    ...scope,
  ].join("\n");
}

function lifecycleInFlight(status?: CliLifecycleStatus): boolean {
  return Boolean(status && !["succeeded", "failed", "timed_out"].includes(status.phase));
}

/** Whether a lifecycle operation is for this reboot target. One that names
 *  no machine went to the primary. */
function lifecycleTargets(status: CliLifecycleStatus, target: RebootTarget): boolean {
  return status.machineId ? status.machineId === target.machineId : target.primary;
}

function lifecyclePhaseLabel(status?: CliLifecycleStatus, hostname?: string): string | null {
  if (!status) return null;
  const label = status.message || `CLI reboot: ${status.phase.replaceAll("_", " ")}`;
  return hostname ? `${hostname}: ${label}` : label;
}

const HOST_BADGE_TONES: Record<TabHostState, { badge: string; dot: string }> = {
  running: {
    badge: "bg-gray-200 text-gray-600 dark:bg-gray-700 dark:text-gray-300",
    dot: "bg-green-500",
  },
  no_access: {
    badge: "bg-amber-100 text-amber-800 dark:bg-amber-950/60 dark:text-amber-200",
    dot: "bg-amber-500",
  },
  // Still running there, briefly unreachable: amber like a problem that
  // clears by itself, not grey like a machine someone has to start.
  disconnected: {
    badge: "bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400",
    dot: "bg-amber-400",
  },
  stopped: {
    badge: "bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400",
    dot: "bg-gray-400",
  },
  offline: {
    badge: "bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400",
    dot: "border border-gray-400 bg-transparent",
  },
};

function isDeepseekTab(provider: string, model?: string, label?: string): boolean {
  if (provider === "deepseek") return true;
  if (provider !== "claude") return false;
  if (isRetiredProviderModel(provider, model)) return false;
  if (isDeepseekModel(model)) return true;
  return typeof label === "string" && label.toLowerCase().includes("deepseek");
}

function ProviderIcon({
  provider,
  model,
  label,
  className = "w-3.5 h-3.5",
}: {
  provider: string;
  model?: string;
  label?: string;
  className?: string;
}) {
  if (provider === "codex") {
    // OpenAI logo — stylized hexagonal node ring
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="Codex">
        <path d="M22.282 9.821a5.985 5.985 0 0 0-.516-4.91 6.046 6.046 0 0 0-6.51-2.9A6.065 6.065 0 0 0 4.981 4.18a5.985 5.985 0 0 0-3.998 2.9 6.046 6.046 0 0 0 .743 7.097 5.98 5.98 0 0 0 .51 4.911 6.051 6.051 0 0 0 6.515 2.9A5.985 5.985 0 0 0 13.26 24a6.056 6.056 0 0 0 5.772-4.206 5.99 5.99 0 0 0 3.997-2.9 6.056 6.056 0 0 0-.747-7.073zM13.26 22.43a4.476 4.476 0 0 1-2.876-1.04l.141-.081 4.779-2.758a.795.795 0 0 0 .392-.681v-6.737l2.02 1.168a.071.071 0 0 1 .038.052v5.583a4.504 4.504 0 0 1-4.494 4.494zM3.6 18.304a4.47 4.47 0 0 1-.535-3.014l.142.085 4.783 2.759a.771.771 0 0 0 .78 0l5.843-3.369v2.332a.08.08 0 0 1-.033.062L9.74 19.95a4.5 4.5 0 0 1-6.14-1.646zM2.34 7.896a4.485 4.485 0 0 1 2.366-1.973V11.6a.766.766 0 0 0 .388.676l5.815 3.355-2.02 1.168a.076.076 0 0 1-.071 0l-4.83-2.786A4.504 4.504 0 0 1 2.34 7.872zm16.597 3.855l-5.833-3.387L15.119 7.2a.076.076 0 0 1 .071 0l4.83 2.791a4.494 4.494 0 0 1-.676 8.105v-6.678a.79.79 0 0 0-.407-.667zm2.01-3.023l-.141-.085-4.774-2.782a.776.776 0 0 0-.785 0L9.409 9.23V6.897a.066.066 0 0 1 .028-.061l4.83-2.787a4.5 4.5 0 0 1 6.68 4.66zm-12.64 4.135l-2.02-1.164a.08.08 0 0 1-.038-.057V6.075a4.5 4.5 0 0 1 7.375-3.453l-.142.08L8.704 5.46a.795.795 0 0 0-.393.681zm1.097-2.365l2.602-1.5 2.607 1.5v2.999l-2.597 1.5-2.607-1.5z" />
      </svg>
    );
  }
  if (isRetiredProviderModel(provider, model)) {
    // A saved Pi tab lands here too: Pi was retired, and its tab stays listed
    // until someone closes it.
    return (
      <span
        className={`${className} text-center font-bold`}
        aria-label={isRetiredPiProvider(provider) ? PI_RETIRED_MESSAGE : "Unsupported provider"}
      >
        !
      </span>
    );
  }
  if (isDeepseekTab(provider, model, label)) {
    // DeepSeek logo — stylized whale silhouette (abstracted as a "D" with fin)
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="DeepSeek">
        <path d="M5 7c4-3 9-3 13 0l2-2v6l-2-2c-3-3-8-3-11 0-1 1-1 3 0 4 2 2 5 2 7 0l2 2-2 2c-4 3-9 3-13 0-2-2-2-7 0-10z" />
      </svg>
    );
  }
  if (provider === "opencode") {
    // OpenCode logo — code brackets < />
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="OpenCode">
        <path d="M8 3L2 12l6 9 2-1.5L5.5 12 10 4.5 8 3zm8 0l-2 1.5L18.5 12 14 19.5 16 21l6-9-6-9z" />
      </svg>
    );
  }
  if (provider === "omp") {
    // OMP is Pi-derived; the doubled pi marks the fork without a new glyph.
    // Drawn inside the icon's own box, like the other providers' marks: as
    // plain text the two letters were wider than the 14px box and ran into
    // the tab's name. `textLength` fits them to the box at any icon size.
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" role="img" aria-label="OMP" data-provider-icon="omp">
        <text x="12" y="19" textAnchor="middle" fontSize="26" fontWeight="700" textLength="23" lengthAdjust="spacingAndGlyphs">
          ππ
        </text>
      </svg>
    );
  }
  if (provider === "cursor-agent") {
    // Cursor logo — upward arrow/cursor silhouette
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="Cursor">
        <path d="M3 2l9 20 3-8 8-3L3 2z" />
      </svg>
    );
  }
  // Claude / Anthropic logo — starburst (from docs.anthropic.com)
  return (
    <svg className={className} viewBox="1 0 50 50" fill="currentColor" aria-label="Claude">
      <path d="M23.1516 45.525L23.8096 42.611L24.5616 38.851L25.1726 35.843L25.7366 32.13L26.0656 30.908L26.0186 30.814L25.7836 30.861L22.9636 34.715L18.6866 40.496L15.3026 44.068L14.5036 44.397L13.0936 43.692L13.2346 42.376L14.0336 41.248L18.6866 35.279L21.5066 31.566L23.3396 29.451L23.2926 29.169H23.1986L10.7906 37.253L8.58158 37.535L7.59458 36.642L7.73558 35.185L8.20558 34.715L11.9186 32.13L21.1776 26.96L21.3186 26.49L21.1776 26.255H20.7076L19.1566 26.161L13.8926 26.02L9.33358 25.832L4.86858 25.597L3.74058 25.362L2.70658 23.952L2.80058 23.247L3.74058 22.636L5.10358 22.73L8.06458 22.965L12.5296 23.247L15.7726 23.435L20.5666 23.952H21.3186L21.4126 23.623L21.1776 23.435L20.9896 23.247L16.3366 20.145L11.3546 16.855L8.72258 14.928L7.31258 13.941L6.60758 13.048L6.32558 11.074L7.59458 9.664L9.33358 9.805L9.75658 9.899L11.4956 11.262L15.2086 14.129L20.0966 17.748L20.8016 18.312L21.1306 18.124V17.983L20.8016 17.466L18.1696 12.672L15.3496 7.784L14.0806 5.763L13.7516 4.541C13.6263 4.118 13.5636 3.648 13.5636 3.131L15.0206 1.157L15.8196 0.874997L17.7936 1.157L18.5926 1.862L19.8146 4.635L21.7416 9.006L24.7966 14.928L25.6896 16.714L26.1596 18.312L26.3476 18.829H26.6766V18.547L26.9116 15.163L27.3816 11.074L27.8516 5.81L27.9926 4.306L28.7446 2.52L30.2016 1.58L31.3296 2.097L32.2696 3.46L32.1286 4.306L31.6116 7.925L30.4836 13.612L29.7786 17.466H30.2016L30.6716 16.949L32.5986 14.411L35.8416 10.369L37.2516 8.771L38.9436 6.985L40.0246 6.139H42.0456L43.5026 8.348L42.8446 10.651L40.7766 13.283L39.0376 15.492L36.5466 18.829L35.0426 21.508L35.1836 21.696H35.5126L41.1056 20.474L44.1606 19.957L47.7326 19.346L49.3776 20.098L49.5656 20.85L48.9076 22.448L45.0536 23.388L40.5416 24.281L33.8206 25.879L33.7266 25.926L33.8206 26.067L36.8286 26.349L38.1446 26.443H41.3406L47.2626 26.866L48.8136 27.9L49.7066 29.122L49.5656 30.109L47.1686 31.284L43.9726 30.532L36.4526 28.746L33.9146 28.135H33.5386V28.323L35.7006 30.438L39.6016 33.963L44.5366 38.522L44.7716 39.65L44.1606 40.59L43.5026 40.496L39.1786 37.206L37.4866 35.749L33.7266 32.6H33.4916V32.929L34.3376 34.198L38.9436 41.107L39.1786 43.222L38.8496 43.88L37.6276 44.303L36.3586 44.068L33.6326 40.308L30.8596 36.031L28.6036 32.224L28.3686 32.412L27.0056 46.606L26.3946 47.311L24.9846 47.875L23.8096 46.982L23.1516 45.525Z" />
    </svg>
  );
}

export function TabBar({
  tabs,
  activeTabId,
  onSelectTab,
  onCloseTab,
  onAddTab,
  jobTabs,
  overviewTab,
  onRenameTab,
  onReorderTabs,
  projectControls,
  onRebootCli,
  lifecycleInventory,
  lifecycleStatus,
  rebootTargets,
  showRebootButton = false,
  tabHosts,
  onForgetMachine,
  machineChoices,
  autopilotTabs,
  paneStatuses,
  pausedPanes,
  leading,
  trailing,
}: TabBarProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [contextMenu, setContextMenu] = useState<{ paneId: number; x: number; y: number } | null>(null);
  const [renamingPaneId, setRenamingPaneId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const renameInputRef = useRef<HTMLInputElement>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const lifecycleMenuRef = useRef<HTMLDivElement>(null);
  const [lifecycleMenuOpen, setLifecycleMenuOpen] = useState(false);
  const { requestConfirm, confirmDialog } = useConfirmDialog();

  // Drag-and-drop state
  const [draggedPaneId, setDraggedPaneId] = useState<number | null>(null);
  const [dropTargetId, setDropTargetId] = useState<number | null>(null);
  const [dropSide, setDropSide] = useState<"left" | "right" | null>(null);

  // Scroll active tab into view when it changes
  useEffect(() => {
    if (activeTabId == null || !scrollRef.current) return;
    const activeEl = scrollRef.current.querySelector(`[data-tab-id="${activeTabId}"]`);
    if (activeEl) {
      activeEl.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
    }
  }, [activeTabId]);

  // Dismiss context menu on outside click
  useEffect(() => {
    if (!contextMenu) return;
    const handleClick = (e: MouseEvent) => {
      if (contextMenuRef.current && !contextMenuRef.current.contains(e.target as Node)) {
        setContextMenu(null);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [contextMenu]);

  useEffect(() => {
    if (!lifecycleMenuOpen) return;
    const handleClick = (event: MouseEvent) => {
      if (!lifecycleMenuRef.current?.contains(event.target as Node)) {
        setLifecycleMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [lifecycleMenuOpen]);

  // Auto-focus rename input
  useEffect(() => {
    if (renamingPaneId != null && renameInputRef.current) {
      renameInputRef.current.focus();
      renameInputRef.current.select();
    }
  }, [renamingPaneId]);

  const handleContextMenu = useCallback((e: React.MouseEvent, paneId: number) => {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ paneId, x: e.clientX, y: e.clientY });
  }, []);

  const handleStartRename = useCallback(() => {
    if (contextMenu == null) return;
    const paneId = contextMenu.paneId;
    const tab = tabs.find((t) => t.pane_id === paneId);
    const label = tab?.label ?? "";
    setRenameDraft(label);
    setRenamingPaneId(paneId);
    setContextMenu(null);
  }, [contextMenu, tabs]);

  const handleFinishRename = useCallback(() => {
    if (renamingPaneId == null) return;
    const trimmed = renameDraft.trim();
    if (trimmed && onRenameTab) {
      onRenameTab(renamingPaneId, trimmed);
    }
    setRenamingPaneId(null);
    setRenameDraft("");
  }, [renamingPaneId, renameDraft, onRenameTab]);

  const handleRenameKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleFinishRename();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setRenamingPaneId(null);
      setRenameDraft("");
    }
  }, [handleFinishRename]);

  // Drag handlers
  const handleDragStart = useCallback((e: React.DragEvent, paneId: number) => {
    setDraggedPaneId(paneId);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(paneId));
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent, paneId: number) => {
    if (draggedPaneId == null || draggedPaneId === paneId) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const midX = rect.left + rect.width / 2;
    setDropTargetId(paneId);
    setDropSide(e.clientX < midX ? "left" : "right");
  }, [draggedPaneId]);

  const handleDrop = useCallback((e: React.DragEvent, targetPaneId: number) => {
    e.preventDefault();
    if (draggedPaneId == null || draggedPaneId === targetPaneId || !onReorderTabs) {
      setDraggedPaneId(null);
      setDropTargetId(null);
      setDropSide(null);
      return;
    }
    const currentOrder = tabs.map((t) => t.pane_id);
    const fromIndex = currentOrder.indexOf(draggedPaneId);
    let toIndex = currentOrder.indexOf(targetPaneId);
    if (fromIndex === -1 || toIndex === -1) return;

    const newOrder = currentOrder.filter((id) => id !== draggedPaneId);
    toIndex = newOrder.indexOf(targetPaneId);
    const insertAt = dropSide === "right" ? toIndex + 1 : toIndex;
    newOrder.splice(insertAt, 0, draggedPaneId);

    onReorderTabs(newOrder);
    setDraggedPaneId(null);
    setDropTargetId(null);
    setDropSide(null);
  }, [draggedPaneId, dropSide, tabs, onReorderTabs]);

  const handleDragEnd = useCallback(() => {
    setDraggedPaneId(null);
    setDropTargetId(null);
    setDropSide(null);
  }, []);

  return (
    <div className="flex items-end border-b border-gray-200 dark:border-gray-700 bg-gray-100 dark:bg-gray-800/50 flex-shrink-0 min-h-[40px]">
      {leading}
      <div
        ref={scrollRef}
        className="flex-1 min-w-0 flex flex-nowrap items-end overflow-x-auto overflow-y-hidden no-scrollbar"
        style={{ scrollbarWidth: "none", msOverflowStyle: "none" }}
      >
        {overviewTab}
        {tabs.map((tab, index) => {
          const isActive = tab.pane_id === activeTabId;
          const isBot = tab.mode === "deadloop";
          const isUnsupported = isRetiredProviderModel(tab.provider, tab.model);
          const isPaused = pausedPanes.includes(tab.pane_id);
          const status = paneStatuses[paneKey(tab.pane_id)];
          const hasActivity = paneIsWorkingStatus(status);
          const isAwaitingAnswer = paneIsAwaitingAnswerStatus(status);
          const label = tab.label || `Tab ${index + 1}`;
          const isRenaming = renamingPaneId === tab.pane_id;
          const host = tabHosts?.get(tab.pane_id);
          // Why this tab's controls do not work: its machine is not running
          // the project, or this viewer may not use that machine.
          const refusal = host?.refusal;
          const hostTone = host ? HOST_BADGE_TONES[host.state] : undefined;
          const showsHostBadge = Boolean(host?.badge && host.hostname && hostTone);
          // On autopilot, or paused: marked on the tab. An ended one is not.
          const autopilot = autopilotTabs?.get(tab.pane_id);
          const autopilotLabel = autopilot ? autopilotMarkerLabel(autopilot.kind) : null;
          const spokenName = `${label}${isBot ? " (Bot)" : ""}${autopilotLabel ? `, ${autopilotLabel}` : ""}`;

          return (
            <button
              key={tab.pane_id}
              data-tab-id={tab.pane_id}
              draggable={!isRenaming}
              onDragStart={(e) => handleDragStart(e, tab.pane_id)}
              onDragOver={(e) => handleDragOver(e, tab.pane_id)}
              onDrop={(e) => handleDrop(e, tab.pane_id)}
              onDragEnd={handleDragEnd}
              onClick={() => { if (!isRenaming) onSelectTab(tab.pane_id); }}
              onContextMenu={(e) => handleContextMenu(e, tab.pane_id)}
              title={refusal}
              // Spelled out once a host badge shows: joined from the
              // visible spans, the label and host would run together. And
              // for a tab on autopilot, whose marker is only a glyph. A tab
              // with neither keeps the name its content gives it.
              aria-label={host?.badge && host.hostname
                ? `${spokenName} — ${tabHostDescription(host)}`
                : autopilotLabel ? spokenName : undefined}
              className={`group relative flex items-center gap-1.5 px-3 py-2 text-sm font-medium transition-colors flex-shrink-0 border-r border-gray-200 dark:border-gray-700 ${
                // Room for the host badge beside the name: within the plain
                // cap a stopped machine's badge left the name one letter.
                // A name at its 7rem cap still leaves the badge room for a
                // short host and its longest state word ("disconnected").
                showsHostBadge ? "max-w-[320px]" : "max-w-[200px]"
              } ${
                isActive
                  ? "bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100 border-b-2 border-b-blue-500 -mb-px"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-700/50 hover:text-gray-700 dark:hover:text-gray-300"
              } ${draggedPaneId === tab.pane_id ? "opacity-40" : refusal && !isActive ? "opacity-60" : ""}`}
              style={{ scrollSnapAlign: "start" }}
            >
              {/* Drop indicator */}
              {dropTargetId === tab.pane_id && dropSide === "left" && (
                <div className="absolute left-0 top-0 bottom-0 w-0.5 bg-blue-500 -translate-x-1/2 z-10" />
              )}
              {dropTargetId === tab.pane_id && dropSide === "right" && (
                <div className="absolute right-0 top-0 bottom-0 w-0.5 bg-blue-500 translate-x-1/2 z-10" />
              )}

              {/* Provider icon + status badge */}
              <span
                className={`relative inline-flex items-center justify-center flex-shrink-0 ${
                  tab.provider === "codex"
                    ? "text-green-500"
                    : tab.provider === "cursor-agent"
                      ? "text-sky-500"
                      : tab.provider === "opencode"
                        ? "text-orange-500"
                        : tab.provider === "omp"
                          ? "text-violet-500"
                          : isUnsupported
                            ? "text-red-500"
                            : "text-blue-500"
                }`}
                title={
                  tab.provider === "codex"
                    ? "Codex"
                    : tab.provider === "cursor-agent"
                      ? "Cursor"
                      : tab.provider === "opencode"
                        ? "OpenCode"
                        : tab.provider === "omp"
                          ? "OMP"
                          : isRetiredPiProvider(tab.provider)
                            ? PI_RETIRED_MESSAGE
                            : isUnsupported
                              ? "Unsupported provider"
                              : "Claude"
                }
              >
                <ProviderIcon provider={tab.provider} model={tab.model} label={tab.label} />
                {hasActivity && !isPaused && (
                  <span className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-blue-500 animate-pulse" />
                )}
                {isAwaitingAnswer && !isPaused && (
                  <span className="absolute -top-0.5 -right-0.5 h-1.5 w-1.5 rounded-full bg-amber-500" />
                )}
                {isPaused && (
                  <span className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-amber-500" />
                )}
              </span>

              {/* Label */}
              {isRenaming ? (
                <input
                  ref={renameInputRef}
                  value={renameDraft}
                  onChange={(e) => setRenameDraft(e.target.value)}
                  onBlur={handleFinishRename}
                  onKeyDown={handleRenameKeyDown}
                  onClick={(e) => e.stopPropagation()}
                  className="w-full min-w-[60px] max-w-[140px] px-1 py-0 text-sm bg-white dark:bg-gray-800 border border-blue-400 rounded outline-none"
                />
              ) : (
                // On mobile, only the active tab shows its label;
                // inactive tabs collapse to icon + activity dot so a
                // dozen panes still fit in one row. Desktop (sm:) keeps
                // the label visible on all tabs.
                // Beside a host badge the name keeps its width, up to a
                // cap, and the badge gives way instead.
                <span className={`truncate ${showsHostBadge ? "shrink-0 max-w-[7rem]" : ""} ${isActive ? "inline" : "hidden sm:inline"}`}>
                  {label}
                  {isBot && " (Bot)"}
                </span>
              )}

              {/* The tab repeats a prompt on its own. Not the worker's icon:
                  autopilot is not a worker. Dimmed while it is paused. */}
              {autopilot && autopilotLabel && (
                <span
                  data-testid="tab-autopilot-marker"
                  data-autopilot={autopilot.kind}
                  aria-hidden="true"
                  title={`Autopilot · ${autopilot.text}`}
                  className={`flex-shrink-0 ${autopilot.kind === "on" ? "text-indigo-500" : "text-amber-500 opacity-60"}`}
                >
                  <Repeat className="h-3 w-3" />
                </span>
              )}

              {/* Which machine runs the tab, once that is worth saying: the
                  roster spans machines, or this tab cannot be used. Like the
                  label, phones show it on the active tab only. */}
              {showsHostBadge && host?.hostname && hostTone && (
                <span
                  data-testid="tab-host-badge"
                  title={tabHostDescription(host)}
                  className={`${isActive ? "inline-flex" : "hidden sm:inline-flex"} ${host.state === "running" ? "max-w-[96px]" : "max-w-[136px]"} min-w-0 shrink items-center gap-1 rounded px-1 py-px text-[10px] font-normal leading-tight ${hostTone.badge}`}
                >
                  <span aria-hidden="true" className={`h-1.5 w-1.5 flex-shrink-0 rounded-full ${hostTone.dot}`} />
                  <span className="truncate">{host.hostname}</span>
                  {/* In words as well as colour; the host name gives way first. */}
                  {host.state !== "running" && (
                    <span className="flex-shrink-0">· {tabHostStateLabel(host.state)}</span>
                  )}
                </span>
              )}

              {/* Close button - visible on hover or when active.
                  Every tab can be closed, including the last one: a project
                  with no panes is a real state that the workspace explains
                  and the "+" here reverses. Hiding it on the last tab also
                  made any tab that reappeared alone impossible to dismiss.
                  The old exemption for managed Manager/Tech Lead tabs went
                  with team mode; nothing creates a managed pane now, so it
                  could only strand a pane from a legacy `.apas`.
                  A tab whose machine is not running the project cannot be
                  closed until it is: that machine's instance holds the tab. */}
              {(
                <span
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!refusal) onCloseTab(tab.pane_id);
                  }}
                  aria-disabled={refusal ? true : undefined}
                  className={`ml-1 p-0.5 rounded flex-shrink-0 ${
                    refusal ? "cursor-not-allowed" : "hover:bg-gray-300 dark:hover:bg-gray-600"
                  } ${
                    isActive
                      ? refusal ? "opacity-30" : "opacity-60 hover:opacity-100"
                      : refusal ? "opacity-0 group-hover:opacity-30" : "opacity-0 group-hover:opacity-60 hover:!opacity-100"
                  }`}
                  title={refusal ?? "Close tab"}
                >
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </span>
              )}
            </button>
          );
        })}
        {jobTabs}
      </div>

      {/* Right-click context menu */}
      {contextMenu && (() => {
        const menuHost = tabHosts?.get(contextMenu.paneId);
        const menuRefusal = menuHost?.refusal;
        const stopped = menuHost?.stopped;
        return (
          <div
            ref={contextMenuRef}
            className="fixed bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-lg py-1 z-50 min-w-[120px] max-w-[min(18rem,calc(100vw-24px))]"
            style={{ left: contextMenu.x, top: contextMenu.y }}
          >
            {menuRefusal && (
              <p className="px-3 py-1.5 text-xs text-gray-500 dark:text-gray-400">{menuRefusal}</p>
            )}
            {stopped?.canForget && onForgetMachine && (
              <button
                onClick={() => {
                  onForgetMachine(stopped.machineId, stopped.hostname);
                  setContextMenu(null);
                }}
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700"
              >
                Forget {stopped.hostname}&apos;s tabs…
              </button>
            )}
            {onRenameTab && (
              <button
                disabled={Boolean(menuRefusal)}
                title={menuRefusal}
                onClick={handleStartRename}
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 disabled:opacity-50 disabled:hover:bg-transparent"
              >
                Rename
              </button>
            )}
            <button
              disabled={Boolean(menuRefusal)}
              title={menuRefusal}
              onClick={() => {
                if (contextMenu) onCloseTab(contextMenu.paneId);
                setContextMenu(null);
              }}
              className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 text-red-600 dark:text-red-400 disabled:opacity-50 disabled:hover:bg-transparent"
            >
              Close
            </button>
          </div>
        );
      })()}

      {/* Global tab-bar actions */}
      <div className="flex items-center gap-1 pr-1">
        <AddTabButton
          onAddTab={onAddTab}
          machineChoices={machineChoices}
        />
        {/* The project as a whole: open or closed, on every machine that
            owns its tabs. Each action is confirmed in the page, with the
            machines named. Closing stops every agent in the project and
            deletes nothing; deleting is its own control and its own dialog. */}
        {projectControls?.openState && (
          <span
            title={projectControls.recordedBy
              ? `${projectControls.openState === "Open" ? "Opened" : "Closed"} by ${projectControls.recordedBy}`
              : undefined}
            className="inline-flex items-center gap-1 px-1 text-xs text-gray-600 dark:text-gray-300"
          >
            <span
              aria-hidden="true"
              className={`h-1.5 w-1.5 rounded-full ${projectControls.openState === "Open" ? "bg-green-500" : "bg-gray-400"}`}
            />
            <span className="sr-only">Project state: </span>
            <span data-testid="project-open-state">{projectControls.openState}</span>
          </span>
        )}
        {projectControls?.openAction && (() => {
          const action = projectControls.openAction;
          return (
            <button
              type="button"
              onClick={() => requestConfirm({
                title: action.confirmation.title,
                body: action.confirmation.body.map((line, index) => <p key={index}>{line}</p>),
                confirmLabel: action.label,
                tone: "primary",
                onConfirm: projectControls.onOpen,
              })}
              className="px-2.5 h-8 my-1 text-xs font-medium rounded transition-colors bg-blue-500 hover:bg-blue-600 text-white whitespace-nowrap"
              title={action.kind === "open_again" ? "Open this project again" : "Open this project"}
            >
              {action.label}
            </button>
          );
        })()}
        {projectControls?.closeAction && (() => {
          const action = projectControls.closeAction;
          const copy = withLinesBeforeLast(
            action.confirmation,
            projectControls.closeNote ? [projectControls.closeNote] : [],
          );
          return (
            <button
              type="button"
              onClick={() => requestConfirm({
                title: copy.title,
                body: copy.body.map((line, index) => <p key={index}>{line}</p>),
                confirmLabel: action.label,
                tone: "danger",
                onConfirm: projectControls.onClose,
              })}
              className="px-2.5 h-8 my-1 text-xs font-medium rounded border border-red-300 text-red-700 transition-colors hover:bg-red-50 dark:border-red-800 dark:text-red-300 dark:hover:bg-red-950/40 whitespace-nowrap"
              title="Close this project and shut down its agents"
            >
              {action.label}
            </button>
          );
        })()}
        {/* An icon here: with its words the project controls took a third
            of the bar, and three tabs already scrolled at 1440 px. The
            cluster panel and the access dialog keep the words. */}
        {projectControls?.onDelete && (
          <button
            type="button"
            onClick={projectControls.onDelete}
            aria-label={DELETE_PROJECT_LABEL}
            title={DELETE_PROJECT_LABEL}
            data-testid="delete-project-control"
            className="flex h-8 w-8 my-1 shrink-0 items-center justify-center rounded text-gray-500 transition-colors hover:bg-red-50 hover:text-red-700 dark:text-gray-400 dark:hover:bg-red-950/40 dark:hover:text-red-300"
          >
            <Trash2 className="h-4 w-4" aria-hidden="true" />
          </button>
        )}
        {showRebootButton && onRebootCli && (
          <div ref={lifecycleMenuRef} className="relative hidden md:block">
            <button
              onClick={() => setLifecycleMenuOpen((open) => !open)}
              className="flex items-center gap-1 px-2.5 h-8 my-1 text-xs font-medium rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 hover:bg-gray-50 dark:hover:bg-gray-700"
              title="Project CLI lifecycle actions"
              aria-expanded={lifecycleMenuOpen}
            >
              {lifecycleInFlight(lifecycleStatus)
                ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" aria-label="CLI operation pending" />
                : <RefreshCw className="h-3.5 w-3.5" />}
              CLI
              <ChevronDown className="h-3 w-3" />
            </button>
            {lifecycleMenuOpen && (
              <div className="absolute right-0 top-full z-50 w-80 rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 shadow-xl p-1.5">
                {rebootTargets && rebootTargets.length > 1 ? (
                  // One instance per machine: each reboot restarts that
                  // machine's daemon only, so each is confirmed against that
                  // machine's own panes.
                  rebootTargets.map((target) => {
                    const pending = lifecycleStatus != null
                      && lifecycleInFlight(lifecycleStatus)
                      && lifecycleTargets(lifecycleStatus, target);
                    const otherHosts = rebootTargets
                      .filter((other) => other.key !== target.key)
                      .map((other) => other.hostname);
                    return (
                      <button
                        key={target.key}
                        disabled={pending || Boolean(target.disabledReason)}
                        title={target.disabledReason}
                        onClick={() => {
                          if (confirm(rebootConfirmation(target.inventory, { hostname: target.hostname, otherHosts }))) {
                            onRebootCli(target.machineId);
                            setLifecycleMenuOpen(false);
                          }
                        }}
                        className="w-full rounded px-3 py-2 text-left text-sm text-red-700 dark:text-red-300 hover:bg-red-50 dark:hover:bg-red-950/40 disabled:opacity-50"
                      >
                        <span className="font-medium">Reboot CLI on {target.hostname}</span>
                        <span className="mt-0.5 block text-xs text-gray-500 dark:text-gray-400">
                          {target.disabledReason
                            ?? `${target.primary ? "Primary instance. " : ""}Replaces that machine's CLI; the other machines keep running.`}
                        </span>
                      </button>
                    );
                  })
                ) : (
                  <button
                    disabled={lifecycleInFlight(lifecycleStatus)}
                    onClick={() => {
                      if (confirm(rebootConfirmation(lifecycleInventory))) {
                        onRebootCli();
                        setLifecycleMenuOpen(false);
                      }
                    }}
                    className="w-full rounded px-3 py-2 text-left text-sm text-red-700 dark:text-red-300 hover:bg-red-50 dark:hover:bg-red-950/40 disabled:opacity-50"
                  >
                    <span className="font-medium">Reboot CLI</span>
                    <span className="mt-0.5 block text-xs text-gray-500 dark:text-gray-400">
                      Replace the CLI process; unsupported panes may restart or resume.
                    </span>
                  </button>
                )}
                {(() => {
                  const statusHost = rebootTargets && rebootTargets.length > 1 && lifecycleStatus
                    ? rebootTargets.find((target) => lifecycleTargets(lifecycleStatus, target))?.hostname
                    : undefined;
                  const label = lifecyclePhaseLabel(lifecycleStatus, statusHost);
                  return label && (
                    <div
                      className="mt-1 border-t border-gray-200 dark:border-gray-700 px-3 py-2 text-xs text-gray-600 dark:text-gray-300"
                      role="status"
                    >
                      {label}
                    </div>
                  );
                })()}
              </div>
            )}
          </div>
        )}
      </div>
      {trailing}
      {confirmDialog}
    </div>
  );
}

/**
 * Where a new tab runs: the project's running instances, machines where it is
 * registered (the project is opened first; against a server that records no
 * open state, that machine alone is started), and owned machines without it
 * (cloned first).
 * Shown only when there is a real choice to make.
 */
function MachineChoiceFieldset({
  model,
  selectedKey,
  onSelect,
}: {
  model: MachineChoiceModel;
  selectedKey: string | null;
  onSelect: (key: string) => void;
}) {
  const name = useId();
  return (
    <fieldset className="border-b border-gray-100 px-3 pb-2 dark:border-gray-700">
      <legend className="px-0 pt-2 text-xs font-semibold text-gray-500 dark:text-gray-400">Run on</legend>
      {groupMachineChoices(model.choices).map((group) => (
        <div key={group.key} role="group" aria-label={group.label} className="mt-1">
          <p aria-hidden="true" className="text-[11px] font-medium uppercase tracking-wide text-gray-400 dark:text-gray-500">{group.label}</p>
          {group.choices.map((choice) => (
            <label
              key={choice.key}
              title={choice.disabledReason}
              className={`flex min-h-11 items-start gap-2 rounded px-1 py-1.5 text-sm ${
                choice.disabledReason ? "cursor-not-allowed opacity-60" : "cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-700"
              }`}
            >
              <input
                type="radio"
                name={name}
                value={choice.key}
                checked={selectedKey === choice.key}
                disabled={Boolean(choice.disabledReason)}
                onChange={() => onSelect(choice.key)}
                className="mt-0.5 h-4 w-4 flex-shrink-0"
              />
              <span className="min-w-0">
                <span className="block truncate font-medium">{choice.hostname}</span>
                <span className="block text-xs text-gray-500 dark:text-gray-400">{machineChoiceDetail(choice)}</span>
              </span>
            </label>
          ))}
        </div>
      ))}
    </fieldset>
  );
}

function AddTabButton({
  onAddTab,
  machineChoices,
}: Pick<TabBarProps, "onAddTab" | "machineChoices">) {
  // "+" creates an interactive agent and nothing else. An autonomous worker is
  // created from the Overview tab or /code/workers, where its mandate,
  // readiness and limits are reviewed.
  // Presentation only; server/CLI still enforce interactive profile policy.
  const isAllowed = useIsLaunchProfileAllowed();
  const [showMenu, setShowMenu] = useState(false);
  const [isolatedWorktree, setIsolatedWorktree] = useState(false);
  const [machineKey, setMachineKey] = useState<string | null>(null);
  const [position, setPosition] = useState({ top: 0, left: 12 });
  const menuRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const showMachineStep = machineChoices?.showChoice === true;
  // The user's pick, else the default (the selected tab's machine). Derived
  // rather than copied at opening, so a default that arrives while the menu is
  // open still counts. A pick that disappears or becomes unavailable while
  // the menu is open (an instance detaches) is no pick at all.
  const selectedKey = machineKey ?? machineChoices?.defaultChoice?.key ?? null;
  const selectedMachine = showMachineStep
    ? machineChoices?.choices.find((choice) => choice.key === selectedKey && !choice.disabledReason)
    : undefined;
  const needsMachine = showMachineStep && !selectedMachine;
  // The machine that receives the tab: the pick, or the hidden step's
  // default, which is still sent. What each provider needs there is said on
  // the provider, since a machine without OMP still takes a Claude tab.
  const targetMachine = selectedMachine ?? (showMachineStep ? undefined : machineChoices?.defaultChoice);

  useEffect(() => {
    if (!showMenu) return;
    const handleClick = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) setShowMenu(false);
    };
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") { setShowMenu(false); buttonRef.current?.focus(); }
    };
    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleKey);
    };
  }, [showMenu]);

  const toggleMenu = () => {
    const rect = buttonRef.current?.getBoundingClientRect();
    if (rect) setPosition({ top: rect.bottom + 4, left: Math.max(12, Math.min(rect.right - 288, window.innerWidth - 300)) });
    // Each opening starts from the default: the selected tab's machine.
    setMachineKey(null);
    setShowMenu((value) => !value);
  };
  const handlePick = (provider: string, model?: string) => {
    if (needsMachine || providerOfferOn(targetMachine, provider).disabledReason) return;
    onAddTab(provider, model, isolatedWorktree || undefined, "terminal", selectedMachine);
    setIsolatedWorktree(false);
    setShowMenu(false);
  };
  const providerEntries = [
    { provider: "claude", label: "Claude" },
    { provider: "claude", label: "Claude / DeepSeek 4.1 Flash", model: DEEPSEEK_FLASH_MODEL },
    { provider: "codex", label: "Codex" },
    { provider: "opencode", label: "OpenCode" },
    { provider: "omp", label: "OMP" },
  ].filter((entry) => isAllowed("terminal", entry.provider, entry.model));

  return (
    <div className="relative flex-shrink-0" ref={menuRef}>
      <button
        ref={buttonRef}
        type="button"
        onClick={toggleMenu}
        aria-expanded={showMenu}
        aria-label="Add agent"
        className="flex items-center justify-center w-11 h-11 m-1 rounded text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors"
        title="New tab"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
        </svg>
      </button>
      {showMenu && (
        <div
          aria-label="Add agent choices"
          className="fixed w-72 max-w-[calc(100vw-24px)] overflow-y-auto bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-lg py-1 z-50"
          style={{ ...position, maxHeight: `calc(100dvh - ${position.top + 12}px)` }}
        >
          <label
            className="flex min-h-11 items-center gap-2 px-3 py-2 text-xs text-gray-600 dark:text-gray-300 border-b border-gray-100 dark:border-gray-700 cursor-pointer"
            title="Runs in a fresh git worktree so it cannot race other panes on file edits. Requires a git repository."
          >
            <input type="checkbox" checked={isolatedWorktree} onChange={(event) => setIsolatedWorktree(event.target.checked)} className="h-4 w-4" />
            Isolated git worktree
          </label>
          {showMachineStep && machineChoices && (
            <MachineChoiceFieldset model={machineChoices} selectedKey={selectedKey} onSelect={setMachineKey} />
          )}
          <p className="px-3 pt-2 text-xs font-semibold text-gray-500 dark:text-gray-400">Interactive provider</p>
          {needsMachine && (
            <p className="px-3 pt-1 text-xs text-gray-500 dark:text-gray-400">Choose where the tab runs first.</p>
          )}
          {providerEntries.map((entry) => {
            const offer: ProviderOffer = needsMachine ? {} : providerOfferOn(targetMachine, entry.provider);
            const detail = offer.disabledReason ?? offer.note;
            return (
              <button
                type="button"
                key={entry.model ? `${entry.provider}/${entry.model}` : entry.provider}
                disabled={needsMachine || Boolean(offer.disabledReason)}
                title={needsMachine ? "Choose where the tab runs first" : offer.disabledReason ?? offer.noteTitle}
                onClick={() => handlePick(entry.provider, entry.model)}
                className="w-full min-h-11 text-left px-3 py-2 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-transparent"
              >
                <span className="flex-shrink-0 text-gray-500 dark:text-gray-400"><ProviderIcon provider={entry.provider} className="w-4 h-4" /></span>
                {detail ? (
                  <span className="min-w-0">
                    <span className="block">{entry.label}</span>
                    <span className="block text-xs text-gray-500 dark:text-gray-400">{detail}</span>
                  </span>
                ) : entry.label}
              </button>
            );
          })}
          {!providerEntries.length && <p className="px-3 py-3 text-sm">No interactive provider is allowed by this project&apos;s launch policy.</p>}
        </div>
      )}
    </div>
  );
}
