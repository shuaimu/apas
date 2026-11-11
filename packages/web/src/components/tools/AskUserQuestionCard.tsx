"use client";

import { memo, useEffect, useMemo, useRef, useState } from "react";
import { CheckCircle, CheckSquare, HelpCircle, Send, Square } from "lucide-react";
import { useStore } from "@/lib/store";
import { isOmpQuestionInput, ompQuestionKey } from "@/lib/ompQuestions";

interface QuestionOption {
  label: string;
  description?: string;
  preview?: string;
}

interface Question {
  id?: string;
  question: string;
  header?: string;
  options: QuestionOption[];
  multiSelect?: boolean;
  recommended?: number;
}

interface AskUserQuestionInput {
  questions?: Question[];
  provider?: string;
  omp?: { dialog_id?: string | null };
}

interface AskUserQuestionCardProps {
  toolUseId?: string;
  input: unknown;
  sessionId?: string;
  paneId?: number;
}

// Sentinel index used to represent the "Other / free text" choice in the
// selections set, so we don't have to invent a separate parallel
// data structure. Negative values are safe because real option indices
// are always ≥ 0.
const OTHER_INDEX = -1;

function isQuestionList(value: unknown): value is AskUserQuestionInput {
  if (!value || typeof value !== "object") return false;
  const list = (value as { questions?: unknown }).questions;
  return Array.isArray(list);
}

export const AskUserQuestionCard = memo(function AskUserQuestionCard({
  toolUseId,
  input,
  sessionId,
  paneId,
}: AskUserQuestionCardProps) {
  const answeredQuestions = useStore((s) => s.answeredQuestions);
  const answerQuestion = useStore((s) => s.answerQuestion);
  const answerOmpQuestion = useStore((s) => s.answerOmpQuestion);
  const ompState = useStore((s) => sessionId && paneId !== undefined && toolUseId
    ? s.ompQuestions.get(ompQuestionKey(sessionId, paneId, toolUseId)) : undefined);

  const data: AskUserQuestionInput = isQuestionList(input) ? input : { questions: [] };
  const questions: Question[] = useMemo(
    () => (Array.isArray(data.questions) ? data.questions : []),
    [data.questions],
  );
  const isOmp = isOmpQuestionInput(input);
  const dialogId = isOmp ? data.omp?.dialog_id : undefined;
  const nativeIdsValid = questions.every((q) => typeof q.id === "string") &&
    new Set(questions.map((q) => q.id)).size === questions.length;
  const canAnswerOmp = !!sessionId && paneId !== undefined && !!dialogId && nativeIdsValid &&
    !!ompState && ompState.dialogId === dialogId;
  const nativeResult = isOmp ? ompState?.result : undefined;
  const unsupportedProvider = !!data.provider && data.provider !== "omp" && data.provider !== "claude";
  const locked = unsupportedProvider || isOmp && (!canAnswerOmp || !!ompState && ompState.phase !== "pending");

  // OMP keys are native IDs (question text and ordering are not identities).
  const [selections, setSelections] = useState<Map<string | number, Set<number>>>(
    () => new Map(),
  );
  const [otherTexts, setOtherTexts] = useState<Map<string | number, string>>(() => new Map());

  const submittedAnswers = !isOmp && toolUseId ? answeredQuestions.get(toolUseId) : undefined;
  const isSubmitted = isOmp ? !!nativeResult : !!submittedAnswers;

  // Local "just-submitted" flag — drives the 1.5s green ring pulse and the
  // scroll-into-view nudge so the user has unambiguous feedback that their
  // click registered. The persistent submitted state comes from the store's
  // answeredQuestions map; this is purely the transient celebration.
  const [justSubmitted, setJustSubmitted] = useState(false);
  const cardRef = useRef<HTMLDivElement | null>(null);

  // A question counts as answered when:
  //  - at least one option is selected, AND
  //  - if "Other" is among the selections, the free-text field is non-empty
  //    (otherwise we'd send an empty answer / dangling commas).
  const allAnswered = useMemo(
    () =>
      questions.every((question, idx) => {
        const key = isOmp ? question.id! : idx;
        const set = selections.get(key);
        if (!set || set.size === 0) return isOmp && (question.multiSelect === true || !!otherTexts.get(key)?.trim());
        if (set.has(OTHER_INDEX)) {
          const txt = otherTexts.get(key)?.trim() ?? "";
          if (txt.length === 0) return false;
        }
        return true;
      }),
    [questions, selections, otherTexts, isOmp],
  );

  const toggleOption = (qIdx: number, oIdx: number, multi: boolean) => {
    if (isSubmitted || locked) return;
    const key = isOmp ? questions[qIdx].id! : qIdx;
    setSelections((prev) => {
      const next = new Map(prev);
      const current = next.get(key) ?? new Set<number>();
      if (multi) {
        const updated = new Set(current);
        if (updated.has(oIdx)) updated.delete(oIdx);
        else updated.add(oIdx);
        next.set(key, updated);
      } else {
        next.set(key, new Set(isOmp && current.has(oIdx) ? [] : [oIdx]));
      }
      return next;
    });
  };

  const updateOtherText = (qIdx: number, value: string) => {
    if (isSubmitted || locked) return;
    const key = isOmp ? questions[qIdx].id! : qIdx;
    setOtherTexts((prev) => {
      const next = new Map(prev);
      next.set(key, value);
      return next;
    });
  };

  const handleSubmit = () => {
    if (!toolUseId || isSubmitted || locked || !allAnswered) return;
    if (isOmp) {
      if (!sessionId || paneId === undefined || !dialogId) return;
      answerOmpQuestion({
        sessionId, paneId, toolUseId, dialogId,
        answers: questions.map((q) => ({
          id: q.id!,
          selectedOptions: [...(selections.get(q.id!) ?? [])].filter((i) => i >= 0).sort((a, b) => a - b).map((i) => q.options[i].label),
          ...(otherTexts.get(q.id!) ? { customInput: otherTexts.get(q.id!)! } : {}),
        })),
      });
      return;
    }
    const answers: Record<string, string> = {};
    questions.forEach((q, idx) => {
      const set = selections.get(idx);
      if (!set || set.size === 0) return;
      const parts: string[] = [];
      // Iterate sorted real option indices first, then the Other text at the
      // end, so "Option A, my custom answer" reads naturally for multi-select.
      Array.from(set)
        .filter((i) => i !== OTHER_INDEX)
        .sort((a, b) => a - b)
        .forEach((i) => {
          const label = q.options[i]?.label;
          if (label) parts.push(label);
        });
      if (set.has(OTHER_INDEX)) {
        const txt = otherTexts.get(idx)?.trim();
        if (txt) parts.push(txt);
      }
      // Key must exactly match the question text — claude correlates
      // answers by question text, not by index. Per Anthropic's docs,
      // free-text answers go through as the literal text (not "Other").
      answers[q.question] = parts.join(", ");
    });
    setJustSubmitted(true);
    answerQuestion(toolUseId, answers);
    cardRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  };

  // Strip the flash class after the animation finishes so subsequent
  // re-renders don't keep replaying it.
  useEffect(() => {
    if (!justSubmitted) return;
    const t = setTimeout(() => setJustSubmitted(false), 1700);
    return () => clearTimeout(t);
  }, [justSubmitted]);

  if (questions.length === 0) {
    return (
      <div className="border border-gray-200 dark:border-gray-700 rounded-lg my-2 px-3 py-2 text-sm text-gray-500 dark:text-gray-400">
        <HelpCircle className="inline w-4 h-4 mr-2" />
        Question received but no options were provided.
      </div>
    );
  }

  const borderColor = isSubmitted && (!isOmp || nativeResult?.status === "answered")
    ? "border-green-400 dark:border-green-600"
    : "border-blue-300 dark:border-blue-700";
  const flashClass = justSubmitted ? "flash-glow" : "";

  return (
    <div
      ref={cardRef}
      className={`border-2 ${borderColor} rounded-lg overflow-hidden my-2 bg-white dark:bg-gray-900 ${flashClass}`}
    >
      {isOmp ? (
        <div role="status" aria-live="polite" className="px-4 py-3 bg-blue-50 dark:bg-blue-900/30 text-sm text-blue-800 dark:text-blue-200">
          <span className="font-semibold">
            {nativeResult?.status === "answered" ? "Answer recorded by OMP"
              : nativeResult?.status === "cancelled" ? "Question cancelled"
              : nativeResult?.status === "unavailable" ? "Question no longer available"
              : ompState?.phase === "accepted" ? "Accepted by OMP — waiting for recorded result"
              : ompState?.phase === "submitting" ? "Sending to OMP — not yet confirmed"
              : ompState?.phase === "uncertain" ? "Delivery unconfirmed — check the terminal"
              : "OMP is asking"}
          </span>
          {!canAnswerOmp && !nativeResult && <p className="mt-1">Answer in the terminal. No active native question bridge is available.</p>}
          {ompState?.error && <p className="mt-1">{ompState.error}</p>}
          {nativeResult?.message && <p className="mt-1 whitespace-pre-wrap break-words">{nativeResult.message}</p>}
        </div>
      ) : isSubmitted ? (
        <div className="flex items-center gap-2 px-4 py-3 bg-green-500 dark:bg-green-600 text-white">
          <CheckCircle className="w-6 h-6" />
          <span className="font-bold text-base tracking-wide uppercase">
            Answer Submitted
          </span>
          <span className="ml-auto text-xs opacity-90">
            Sent to Claude
          </span>
        </div>
      ) : (
        <div className="flex items-center gap-2 px-4 py-2 bg-blue-50 dark:bg-blue-900/30">
          <HelpCircle className="w-5 h-5 text-blue-500" />
          <span className="font-medium text-blue-700 dark:text-blue-300">
            {unsupportedProvider ? "Answer in the terminal; web answers are unavailable for this provider." : "Claude is asking"}
          </span>
        </div>
      )}

      <div className="px-4 py-3 space-y-4">
        {questions.map((q, qIdx) => {
          const selectedSet = selections.get(isOmp ? q.id! : qIdx) ?? new Set<number>();
          const submittedLabel = submittedAnswers?.[q.question];
          const recorded = nativeResult?.results.find((answer) => answer.id === q.id);
          const multi = q.multiSelect === true;
          const otherSelected = selectedSet.has(OTHER_INDEX);
          const otherText = otherTexts.get(isOmp ? q.id! : qIdx) ?? "";
          return (
            <div key={isOmp ? q.id : qIdx} className="space-y-2">
              {q.header && (
                <div className="inline-block text-xs font-semibold uppercase tracking-wide px-2 py-0.5 bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-300 rounded">
                  {q.header}
                </div>
              )}
              <div className="text-sm font-medium text-gray-800 dark:text-gray-100">
                {q.question}
              </div>
              {isSubmitted ? (
                <div className={`text-sm rounded px-3 py-2 flex items-start gap-2 ${!isOmp || nativeResult?.status === "answered" ? "bg-green-50 dark:bg-green-900/30 border border-green-300 dark:border-green-700" : "bg-gray-50 dark:bg-gray-800 border border-gray-300 dark:border-gray-700"}`}>
                  <CheckCircle className="w-4 h-4 text-green-600 dark:text-green-400 mt-0.5 flex-shrink-0" />
                  <div className="flex-1 min-w-0">
                    <div className="text-xs text-green-700 dark:text-green-400 font-medium uppercase tracking-wide">
                      {isOmp ? "Native result" : "Your answer"}
                    </div>
                    <div className="text-sm text-gray-900 dark:text-gray-100 font-medium mt-0.5 break-words">
                      {isOmp ? (
                        recorded ? <>
                          {recorded.selectedOptions.map((label, index) => <div key={index}>{label}</div>)}
                          {recorded.customInput !== undefined && <div className="whitespace-pre-wrap">{recorded.customInput}</div>}
                          {recorded.selectedOptions.length === 0 && !recorded.customInput && <div>No options selected</div>}
                          {recorded.note && <div>{recorded.note}</div>}
                          {recorded.timedOut && <div>Timed out</div>}
                        </> : "—"
                      ) : submittedLabel ?? "—"}
                    </div>
                  </div>
                </div>
              ) : (
                <div className="space-y-1">
                  {q.options.map((opt, oIdx) => {
                    const selected = selectedSet.has(oIdx);
                    return (
                      <button
                        key={oIdx}
                        type="button"
                        disabled={locked}
                        aria-pressed={selected}
                        onClick={() => toggleOption(qIdx, oIdx, multi)}
                        className={`min-h-11 w-full flex items-start gap-3 text-left px-3 py-2 rounded border transition-colors disabled:opacity-60 ${
                          selected
                            ? "border-blue-400 dark:border-blue-500 bg-blue-50 dark:bg-blue-900/40"
                            : "border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800"
                        }`}
                      >
                        <span className="mt-0.5 flex-shrink-0 text-blue-500">
                          {multi ? (
                            selected ? (
                              <CheckSquare className="w-4 h-4" />
                            ) : (
                              <Square className="w-4 h-4 text-gray-400" />
                            )
                          ) : (
                            <span
                              className={`block w-4 h-4 rounded-full border-2 ${
                                selected
                                  ? "border-blue-500 bg-blue-500"
                                  : "border-gray-400"
                              }`}
                            />
                          )}
                        </span>
                        <span className="flex-1 min-w-0">
                          <span className="block text-sm font-medium text-gray-900 dark:text-gray-100">
                            {opt.label}{isOmp && q.recommended === oIdx && <span className="ml-2 text-xs">(Recommended)</span>}
                          </span>
                          {opt.description && (
                            <span className="block text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                              {opt.description}
                            </span>
                          )}
                          {isOmp && opt.preview && <span className="block text-xs whitespace-pre-wrap break-words mt-1">{opt.preview}</span>}
                        </span>
                      </button>
                    );
                  })}
                  {/* "Other / free text" choice — always rendered after the
                      claude-supplied options so the user can answer with
                      something not on the list. The text is what gets sent
                      to claude (per Anthropic's docs), not the word "Other". */}
                  {!isOmp && <button
                    type="button"
                    aria-pressed={otherSelected}
                    onClick={() => toggleOption(qIdx, OTHER_INDEX, multi)}
                    className={`w-full flex items-start gap-3 text-left px-3 py-2 rounded border transition-colors ${
                      otherSelected
                        ? "border-blue-400 dark:border-blue-500 bg-blue-50 dark:bg-blue-900/40"
                        : "border-dashed border-gray-300 dark:border-gray-600 hover:bg-gray-50 dark:hover:bg-gray-800"
                    }`}
                  >
                    <span className="mt-0.5 flex-shrink-0 text-blue-500">
                      {multi ? (
                        otherSelected ? (
                          <CheckSquare className="w-4 h-4" />
                        ) : (
                          <Square className="w-4 h-4 text-gray-400" />
                        )
                      ) : (
                        <span
                          className={`block w-4 h-4 rounded-full border-2 ${
                            otherSelected
                              ? "border-blue-500 bg-blue-500"
                              : "border-gray-400"
                          }`}
                        />
                      )}
                    </span>
                    <span className="flex-1 min-w-0">
                      <span className="block text-sm font-medium text-gray-700 dark:text-gray-300 italic">
                        Other (write your own answer)
                      </span>
                    </span>
                  </button>}
                  {(isOmp || otherSelected) && (
                    <div className="pl-7 pr-1 pt-1">
                      <input
                        type="text"
                        aria-label={isOmp ? `Custom answer for question ${qIdx + 1}: ${q.question}` : `Other answer: ${q.question}`}
                        disabled={locked}
                        autoFocus={!isOmp}
                        value={otherText}
                        onChange={(e) => updateOtherText(qIdx, e.target.value)}
                        placeholder="Type your answer…"
                        className="w-full rounded border border-blue-300 dark:border-blue-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100 focus:outline-none focus:ring-2 focus:ring-blue-400"
                        onKeyDown={(e) => {
                          if (e.key === "Enter" && allAnswered) {
                            e.preventDefault();
                            handleSubmit();
                          }
                        }}
                      />
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}

        {!isSubmitted && (
          <div className="flex flex-wrap justify-end gap-2 pt-1">
            {isOmp && <button
              type="button"
              disabled={locked || !toolUseId}
              className="min-h-11 px-4 py-2 rounded-lg border border-gray-300 dark:border-gray-700 text-sm disabled:opacity-50"
              onClick={() => {
                if (sessionId && paneId !== undefined && toolUseId && dialogId) {
                  answerOmpQuestion({ sessionId, paneId, toolUseId, dialogId, answers: [], cancel: true });
                }
              }}
            >Cancel question</button>}
            <button
              type="button"
              onClick={handleSubmit}
              disabled={locked || !toolUseId || !allAnswered}
              className={`min-h-11 flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
                !locked && toolUseId && allAnswered
                  ? "bg-blue-500 text-white hover:bg-blue-600"
                  : "bg-gray-200 dark:bg-gray-700 text-gray-400 dark:text-gray-500 cursor-not-allowed"
              }`}
            >
              <Send className="w-4 h-4" />
              Submit
            </button>
          </div>
        )}
      </div>
    </div>
  );
});
