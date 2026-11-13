"use client";

import { Repeat, X } from "lucide-react";
import type { OmpModesView } from "@apas/protocol";
import {
  autopilotActionDetail,
  autopilotActionLabel,
  autopilotDetailRows,
  autopilotEndedText,
  autopilotProgressText,
  autopilotRequestTitle,
  AUTOPILOT_START_IN_FLIGHT_TEXT,
  formatAutopilotTime,
  ompModeNotices,
  type AutopilotControlModel,
  type AutopilotRequestKind,
} from "@/lib/autopilot";

/**
 * Autopilot on the mobile-browser session screen. The screen works out one
 * `AutopilotControlModel` for the selected tab, the same one the desktop
 * toolbar uses, so a phone offers the same actions and disables them for the
 * same reasons. These are its pieces:
 *
 * - the state notice above the composer, while the tab is on autopilot or has
 *   a paused one, and the ended reason with a way to start again;
 * - the same notice as one row in the raw terminal view, which hides the
 *   composer;
 * - what the tab's "Pane actions" sheet offers: starting, or, while the tab's
 *   state is not reported, Stop and Stop now;
 * - the sheet that asks how to stop, and the details sheet.
 *
 * Nothing here sends anything by itself, and nothing here touches the
 * composer: a person can always steer a tab that is on autopilot.
 */

type AutopilotControlAction = Exclude<AutopilotRequestKind, "start">;

const SHEET =
  "max-h-[82dvh] w-full overflow-y-auto rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] p-4 pb-[max(1rem,env(safe-area-inset-bottom))] shadow-2xl dark:border-[#383842] dark:bg-[#111115]";
const MUTED = "text-[#686873] dark:text-[#aaaab6]";

const NOTICE_TONES = {
  on: {
    box: "border-[#c8c1ff] bg-[#eeecff] text-[#5b4de0] dark:border-[#665cc7] dark:bg-[#292452] dark:text-[#c8c1ff]",
    button: "border-[#c8c1ff] dark:border-[#665cc7]",
  },
  paused: {
    box: "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200",
    button: "border-amber-300 dark:border-amber-800",
  },
} as const;
const NEUTRAL_BOX =
  "border-[#dedee7] bg-white text-[#45454f] dark:border-[#383842] dark:bg-[#1b1b21] dark:text-[#c7c7d1]";

/**
 * The state notice above the composer. It shows what the tab reports, never
 * a state of its own: a request's receipt only says the tab took it. Shown
 * while the tab is on autopilot or has a paused one; an ended autopilot keeps
 * a line here, with its reason and "Start again", until a later turn has run,
 * as it does in the lists. OMP's own goal and loop modes are shown here too,
 * because a message typed below goes to the same terminal.
 */
export function MobileAutopilotNotice({
  model,
  modes,
  startInFlight,
  onAction,
  onStop,
  onDetails,
  onStartAgain,
}: {
  model: AutopilotControlModel;
  /** OMP's own modes, as the tab reports them. */
  modes?: OmpModesView | null;
  /** A start is on its way: another would only be refused. */
  startInFlight: boolean;
  onAction: (kind: "pause" | "resume") => void;
  /** Stop asks first whether the turn may finish. */
  onStop: () => void;
  onDetails: () => void;
  onStartAgain: () => void;
}) {
  const { autopilot, lastEnded } = model;
  const tone = NOTICE_TONES[model.phase === "on" ? "on" : "paused"];
  const actionsDisabled = Boolean(model.disabledReason);
  const button = `min-h-11 rounded-xl border px-3 py-2 text-xs font-bold disabled:opacity-40 ${tone.button}`;
  const toggle = model.actions.find((kind): kind is "pause" | "resume" => kind === "pause" || kind === "resume");
  // Lists stop showing an ended autopilot once a later turn has run; so does
  // this line. Its reason stays in the tab's actions until another starts.
  const ended = !autopilot && lastEnded && !lastEnded.turn_since ? lastEnded : undefined;
  const endedAt = ended ? formatAutopilotTime(ended.ended_at) : null;
  const startAgainReason = model.disabledReason ?? (startInFlight ? AUTOPILOT_START_IN_FLIGHT_TEXT : undefined);
  const modeNotices = ompModeNotices(modes);
  if (!autopilot && !ended && modeNotices.length === 0) return null;

  return (
    // Bounded: the composer below does not shrink, so a paused autopilot
    // together with OMP's modes must scroll here, not push it off a phone.
    <div className="max-h-[38dvh] overflow-y-auto">
      {autopilot && (
        <div
          role="group"
          aria-label="Autopilot"
          data-autopilot={model.phase}
          className={`mb-1.5 rounded-xl border px-3 py-2 text-xs ${tone.box}`}
        >
          <p role="status" className="flex items-start gap-2">
            <Repeat aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />
            <span className="min-w-0 flex-1 break-words">
              <span className="font-bold">Autopilot · {model.statusText}</span>
              {" · "}
              {autopilotProgressText(autopilot.iterations, autopilot.spec.iteration_limit)}
            </span>
          </p>
          {model.notice && <p role="note" className="mt-1.5">{model.notice}</p>}
          {/* Disabled, not hidden: the reason is what tells the person why
              the state can be read and not changed from here. */}
          {actionsDisabled && model.actions.length > 0 && (
            <p role="note" className="mt-1.5">{model.disabledReason}</p>
          )}
          <div className="mt-2 flex flex-wrap gap-2">
            {toggle && (
              <button
                type="button"
                disabled={actionsDisabled}
                title={model.disabledReason}
                onClick={() => onAction(toggle)}
                className={button}
              >
                {autopilotActionLabel(toggle)}
              </button>
            )}
            {model.actions.includes("stop") && (
              <button
                type="button"
                disabled={actionsDisabled}
                title={model.disabledReason}
                onClick={onStop}
                className={button}
              >
                {autopilotActionLabel("stop")}
              </button>
            )}
            <button type="button" onClick={onDetails} className={button}>Details</button>
          </div>
        </div>
      )}

      {ended && (
        <div
          role="status"
          aria-label="Autopilot"
          data-autopilot="ended"
          className={`mb-1.5 flex items-start gap-2 rounded-xl border px-3 py-2 text-xs ${NEUTRAL_BOX}`}
        >
          <Repeat aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />
          <span className="min-w-0 flex-1 break-words">
            Autopilot · {autopilotEndedText(ended.reason)}
            {endedAt ? ` · ${endedAt}` : ""}
            <button
              type="button"
              disabled={Boolean(startAgainReason)}
              title={startAgainReason}
              onClick={onStartAgain}
              className="ml-2 font-bold underline disabled:no-underline disabled:opacity-50"
            >
              Start again
            </button>
            {startAgainReason && <span className="mt-1 block">{startAgainReason}</span>}
          </span>
        </div>
      )}

      {modeNotices.map((notice) => (
        <div
          key={notice.mode}
          role="note"
          data-testid={`omp-mode-${notice.mode}`}
          className={`mb-1.5 rounded-xl border px-3 py-2 text-xs ${NEUTRAL_BOX}`}
        >
          <p className="font-bold">{notice.title}</p>
          {/* A goal's objective can run to 500 characters, above a composer
              on a phone; the terminal has it in full. */}
          {notice.facts.map((fact) => <p key={fact} className="line-clamp-3 break-words">{fact}</p>)}
          <p>{notice.howToStop}</p>
          {notice.warning && <p className="font-bold text-amber-800 dark:text-amber-200">{notice.warning}</p>}
        </div>
      ))}
    </div>
  );
}

/**
 * The state notice while the tab's raw terminal is open. The composer and the
 * notice above it are hidden then, and a tab on autopilot is exactly the one
 * a person watches in its terminal: one row under the terminal's header, with
 * the condition and the same actions, so pausing or stopping does not mean
 * leaving the terminal. The terminal is dark in both themes, and so is this.
 */
export function MobileAutopilotTerminalNotice({
  model,
  shownReason,
  onAction,
  onStop,
  onDetails,
}: {
  model: AutopilotControlModel;
  /** A reason the terminal already states above this row. */
  shownReason?: string;
  onAction: (kind: "pause" | "resume") => void;
  /** Stop asks first whether the turn may finish. */
  onStop: () => void;
  onDetails: () => void;
}) {
  const { autopilot } = model;
  if (!autopilot) return null;
  const actionsDisabled = Boolean(model.disabledReason);
  const toggle = model.actions.find((kind): kind is "pause" | "resume" => kind === "pause" || kind === "resume");
  const button = "flex min-h-11 shrink-0 items-center rounded-lg px-2.5 text-xs font-bold hover:bg-neutral-800 disabled:opacity-40";
  // Why the actions are missing or disabled, unless the line above says it.
  const reason = model.notice ?? (model.actions.length > 0 ? model.disabledReason : undefined);
  return (
    <div
      role="group"
      aria-label="Autopilot"
      data-autopilot={model.phase}
      className="shrink-0 border-b border-neutral-800 px-2 text-xs text-neutral-200"
    >
      <div className="flex items-center gap-1">
        <p role="status" className="flex min-w-0 flex-1 items-center gap-1.5 px-1">
          <Repeat
            aria-hidden="true"
            className={`h-3.5 w-3.5 shrink-0 ${model.phase === "on" ? "text-indigo-300" : "text-amber-300"}`}
          />
          <span className="min-w-0 break-words line-clamp-2" title={`Autopilot · ${model.statusText}`}>
            Autopilot · {model.statusText}
          </span>
        </p>
        {toggle && (
          <button
            type="button"
            disabled={actionsDisabled}
            title={model.disabledReason}
            onClick={() => onAction(toggle)}
            className={button}
          >
            {autopilotActionLabel(toggle)}
          </button>
        )}
        {model.actions.includes("stop") && (
          <button
            type="button"
            disabled={actionsDisabled}
            title={model.disabledReason}
            onClick={onStop}
            className={button}
          >
            {autopilotActionLabel("stop")}
          </button>
        )}
        <button type="button" onClick={onDetails} className={button}>Details</button>
      </div>
      {reason && reason !== shownReason && (
        <p role="note" className="px-1 pb-2 text-amber-300">{reason}</p>
      )}
    </div>
  );
}

const ACTION_ROW = "flex w-full items-start gap-3 rounded-2xl border bg-white p-3.5 text-left dark:bg-[#1b1b21]";
const ACTION_ROW_PLAIN = "border-[#dedee7] dark:border-[#383842]";
const ACTION_ROW_DANGER = "border-red-200 text-red-700 dark:border-red-900 dark:text-red-300";

/**
 * What the tab's "Pane actions" sheet offers for autopilot. With no autopilot,
 * or an ended one, that is starting; where autopilot cannot be used the entry
 * stays, disabled, with the reason. While the tab's state is not reported it
 * is Stop and Stop now and nothing else: a stop needs no state, and starting,
 * Pause and Resume need one. A tab with a current autopilot has its actions
 * in the state notice, so nothing is offered here.
 */
export function MobileAutopilotActions({
  model,
  startInFlight,
  onStart,
  onAction,
}: {
  model: AutopilotControlModel;
  startInFlight: boolean;
  onStart: () => void;
  onAction: (kind: AutopilotControlAction) => void;
}) {
  if (model.autopilot) return null;

  if (model.notReported) {
    return (
      <div role="group" aria-label="Autopilot" className="space-y-2">
        <p role="note" className={`px-1 text-xs ${MUTED}`}>{model.notice}</p>
        {model.disabledReason && (
          <p role="note" className="px-1 text-xs text-amber-800 dark:text-amber-200">{model.disabledReason}</p>
        )}
        {model.actions.map((kind) => (
          <button
            key={kind}
            type="button"
            disabled={Boolean(model.disabledReason)}
            title={model.disabledReason}
            onClick={() => onAction(kind)}
            className={`${ACTION_ROW} disabled:opacity-40 ${kind === "stop_now" ? ACTION_ROW_DANGER : ACTION_ROW_PLAIN}`}
          >
            <Repeat aria-hidden="true" className={`mt-0.5 h-5 w-5 shrink-0 ${kind === "stop_now" ? "" : MUTED}`} />
            <span className="min-w-0">
              <span className="block font-bold">{autopilotRequestTitle(kind)}</span>
              <span className={`mt-0.5 block text-xs font-normal ${MUTED}`}>{autopilotActionDetail(kind)}</span>
            </span>
          </button>
        ))}
      </div>
    );
  }

  const disabledReason = model.disabledReason ?? (startInFlight ? AUTOPILOT_START_IN_FLIGHT_TEXT : undefined);
  const endedAt = model.endedAt ? formatAutopilotTime(model.endedAt) : null;
  // An ended autopilot's reason stays on the tab's control until another is
  // started; its values are what the start sheet then offers.
  const ended = model.phase === "ended" && model.statusText
    ? `${model.statusText}${endedAt ? ` · ${endedAt}` : ""}`
    : undefined;
  // Only the action is dimmed while it is disabled. The reason under it is
  // what a phone has in place of a tooltip, so it stays readable.
  const dimmed = disabledReason ? "opacity-40" : "";
  return (
    <button
      type="button"
      disabled={Boolean(disabledReason)}
      title={disabledReason}
      onClick={onStart}
      className={`${ACTION_ROW} ${ACTION_ROW_PLAIN}`}
    >
      <Repeat aria-hidden="true" className={`mt-0.5 h-5 w-5 shrink-0 ${MUTED} ${dimmed}`} />
      <span className="min-w-0">
        <span className={`block font-bold ${dimmed}`}>{autopilotActionLabel("start")}</span>
        {ended && <span className={`mt-0.5 block text-xs font-normal ${MUTED}`}>{ended}</span>}
        {disabledReason && <span className={`mt-0.5 block text-xs font-normal ${MUTED}`}>{disabledReason}</span>}
      </span>
    </button>
  );
}

/**
 * Stop, from a phone: let the turn in progress finish, or interrupt it now.
 * Stop now says what it does not stop, as it does wherever it is offered.
 */
export function MobileAutopilotStopSheet({
  tabLabel,
  model,
  onChoose,
  onClose,
}: {
  tabLabel: string;
  model: AutopilotControlModel;
  onChoose: (kind: "stop" | "stop_now") => void;
  onClose: () => void;
}) {
  const choices = model.actions.filter((kind): kind is "stop" | "stop_now" => kind === "stop" || kind === "stop_now");
  return (
    <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`Stop autopilot on ${tabLabel}`}
        onClick={(event) => event.stopPropagation()}
        className={SHEET}
      >
        <h2 className="text-lg font-extrabold">Stop autopilot on {tabLabel}?</h2>
        <p className={`mt-1.5 text-sm leading-5 ${MUTED}`}>
          {model.statusText}
          {model.autopilot && ` · ${autopilotProgressText(model.autopilot.iterations, model.autopilot.spec.iteration_limit)}`}
        </p>
        {model.disabledReason && (
          <p role="note" className="mt-2 text-sm text-amber-800 dark:text-amber-200">{model.disabledReason}</p>
        )}
        <div className="mt-4 space-y-2">
          {choices.map((kind) => (
            <button
              key={kind}
              type="button"
              disabled={Boolean(model.disabledReason)}
              title={model.disabledReason}
              onClick={() => onChoose(kind)}
              className={`w-full rounded-2xl border bg-white p-3.5 text-left disabled:opacity-40 dark:bg-[#1b1b21] ${kind === "stop_now" ? ACTION_ROW_DANGER : ACTION_ROW_PLAIN}`}
            >
              <span className="block font-bold">{autopilotActionLabel(kind)}</span>
              <span className={`mt-0.5 block text-xs font-normal ${MUTED}`}>{autopilotActionDetail(kind)}</span>
            </button>
          ))}
          <button
            type="button"
            onClick={onClose}
            className="w-full rounded-xl border border-[#dedee7] px-4 py-2.5 text-sm font-bold dark:border-[#383842]"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * One autopilot's values, read-only: the prompt, the timing, the limits or
 * that none is set, when it started, the progress and, with a time limit,
 * its end. Everyone who can open the session can read them.
 */
export function MobileAutopilotDetailsSheet({
  tabLabel,
  model,
  onClose,
}: {
  tabLabel: string;
  model: AutopilotControlModel;
  onClose: () => void;
}) {
  const { autopilot } = model;
  if (!autopilot) return null;
  return (
    <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Autopilot details"
        onClick={(event) => event.stopPropagation()}
        className={SHEET}
      >
        <div className="flex items-center justify-between gap-2">
          <h2 className="min-w-0 break-words text-lg font-extrabold">Autopilot · {tabLabel}</h2>
          <button
            type="button"
            aria-label="Close autopilot details"
            onClick={onClose}
            className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg hover:bg-[#efeff5] dark:hover:bg-[#25252d]"
          >
            <X className="h-5 w-5" />
          </button>
        </div>
        <p className={`mt-1 text-sm ${MUTED}`}>{model.statusText}</p>
        <h3 className="mt-4 text-sm font-bold">Prompt</h3>
        <p className="mt-1.5 max-h-56 overflow-y-auto whitespace-pre-wrap break-words rounded-xl bg-[#efeff5] p-3 text-sm leading-6 dark:bg-[#25252d]">
          {autopilot.spec.prompt}
        </p>
        <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-sm">
          {autopilotDetailRows(autopilot).map((row) => (
            <div key={row.label} className="contents">
              <dt className={MUTED}>{row.label}</dt>
              <dd className="min-w-0 break-words">{row.value}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
}
