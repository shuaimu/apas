"use client";

import { memo, useEffect, useMemo, useRef, useState } from "react";
import { CheckCircle, CheckSquare, HelpCircle, Send, Square } from "lucide-react";
import { useStore } from "@/lib/store";

interface QuestionOption {
  label: string;
  description?: string;
}

interface Question {
  question: string;
  header?: string;
  options: QuestionOption[];
  multiSelect?: boolean;
}

interface AskUserQuestionInput {
  questions?: Question[];
}

interface AskUserQuestionCardProps {
  toolUseId?: string;
  input: unknown;
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
}: AskUserQuestionCardProps) {
  const answeredQuestions = useStore((s) => s.answeredQuestions);
  const answerQuestion = useStore((s) => s.answerQuestion);

  const data = isQuestionList(input) ? input : { questions: [] };
  const questions: Question[] = useMemo(
    () => (Array.isArray(data.questions) ? data.questions : []),
    [data.questions],
  );

  // questionIndex -> set of selected option indices (OTHER_INDEX for free text).
  const [selections, setSelections] = useState<Map<number, Set<number>>>(
    () => new Map(),
  );
  // questionIndex -> the user's free-text input for "Other".
  const [otherTexts, setOtherTexts] = useState<Map<number, string>>(() => new Map());

  const submittedAnswers = toolUseId ? answeredQuestions.get(toolUseId) : undefined;
  const isSubmitted = !!submittedAnswers;

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
      questions.every((_, idx) => {
        const set = selections.get(idx);
        if (!set || set.size === 0) return false;
        if (set.has(OTHER_INDEX)) {
          const txt = otherTexts.get(idx)?.trim() ?? "";
          if (txt.length === 0) return false;
        }
        return true;
      }),
    [questions, selections, otherTexts],
  );

  const toggleOption = (qIdx: number, oIdx: number, multi: boolean) => {
    if (isSubmitted) return;
    setSelections((prev) => {
      const next = new Map(prev);
      const current = next.get(qIdx) ?? new Set<number>();
      if (multi) {
        const updated = new Set(current);
        if (updated.has(oIdx)) updated.delete(oIdx);
        else updated.add(oIdx);
        next.set(qIdx, updated);
      } else {
        next.set(qIdx, new Set([oIdx]));
      }
      return next;
    });
  };

  const updateOtherText = (qIdx: number, value: string) => {
    if (isSubmitted) return;
    setOtherTexts((prev) => {
      const next = new Map(prev);
      next.set(qIdx, value);
      return next;
    });
  };

  const handleSubmit = () => {
    if (!toolUseId || isSubmitted || !allAnswered) return;
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

  const borderColor = isSubmitted
    ? "border-green-400 dark:border-green-600"
    : "border-blue-300 dark:border-blue-700";
  const flashClass = justSubmitted ? "flash-glow" : "";

  return (
    <div
      ref={cardRef}
      className={`border-2 ${borderColor} rounded-lg overflow-hidden my-2 bg-white dark:bg-gray-900 ${flashClass}`}
    >
      {isSubmitted ? (
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
            Claude is asking
          </span>
        </div>
      )}

      <div className="px-4 py-3 space-y-4">
        {questions.map((q, qIdx) => {
          const selectedSet = selections.get(qIdx) ?? new Set<number>();
          const submittedLabel = submittedAnswers?.[q.question];
          const multi = q.multiSelect === true;
          const otherSelected = selectedSet.has(OTHER_INDEX);
          const otherText = otherTexts.get(qIdx) ?? "";
          return (
            <div key={qIdx} className="space-y-2">
              {q.header && (
                <div className="inline-block text-xs font-semibold uppercase tracking-wide px-2 py-0.5 bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-300 rounded">
                  {q.header}
                </div>
              )}
              <div className="text-sm font-medium text-gray-800 dark:text-gray-100">
                {q.question}
              </div>
              {isSubmitted ? (
                <div className="text-sm bg-green-50 dark:bg-green-900/30 border border-green-300 dark:border-green-700 rounded px-3 py-2 flex items-start gap-2">
                  <CheckCircle className="w-4 h-4 text-green-600 dark:text-green-400 mt-0.5 flex-shrink-0" />
                  <div className="flex-1 min-w-0">
                    <div className="text-xs text-green-700 dark:text-green-400 font-medium uppercase tracking-wide">
                      Your answer
                    </div>
                    <div className="text-sm text-gray-900 dark:text-gray-100 font-medium mt-0.5 break-words">
                      {submittedLabel ?? "—"}
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
                        onClick={() => toggleOption(qIdx, oIdx, multi)}
                        className={`w-full flex items-start gap-3 text-left px-3 py-2 rounded border transition-colors ${
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
                            {opt.label}
                          </span>
                          {opt.description && (
                            <span className="block text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                              {opt.description}
                            </span>
                          )}
                        </span>
                      </button>
                    );
                  })}
                  {/* "Other / free text" choice — always rendered after the
                      claude-supplied options so the user can answer with
                      something not on the list. The text is what gets sent
                      to claude (per Anthropic's docs), not the word "Other". */}
                  <button
                    type="button"
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
                  </button>
                  {otherSelected && (
                    <div className="pl-7 pr-1 pt-1">
                      <input
                        type="text"
                        autoFocus
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
          <div className="flex justify-end pt-1">
            <button
              type="button"
              onClick={handleSubmit}
              disabled={!toolUseId || !allAnswered}
              className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
                toolUseId && allAnswered
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
