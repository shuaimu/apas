"use client";

import { memo, useMemo, useState } from "react";
import { CheckSquare, HelpCircle, Send, Square } from "lucide-react";
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

  // Map of questionIndex -> set of selected option indices.
  const [selections, setSelections] = useState<Map<number, Set<number>>>(
    () => new Map(),
  );

  const submittedAnswers = toolUseId ? answeredQuestions.get(toolUseId) : undefined;
  const isSubmitted = !!submittedAnswers;

  const allAnswered = useMemo(
    () =>
      questions.every((_, idx) => {
        const set = selections.get(idx);
        return set && set.size > 0;
      }),
    [questions, selections],
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

  const handleSubmit = () => {
    if (!toolUseId || isSubmitted || !allAnswered) return;
    const answers: Record<string, string> = {};
    questions.forEach((q, idx) => {
      const set = selections.get(idx);
      if (!set || set.size === 0) return;
      const labels = Array.from(set)
        .sort((a, b) => a - b)
        .map((i) => q.options[i]?.label)
        .filter((s): s is string => !!s);
      // Key must exactly match the question text — claude correlates
      // answers by question text, not by index.
      answers[q.question] = labels.join(", ");
    });
    answerQuestion(toolUseId, answers);
  };

  if (questions.length === 0) {
    return (
      <div className="border border-gray-200 dark:border-gray-700 rounded-lg my-2 px-3 py-2 text-sm text-gray-500 dark:text-gray-400">
        <HelpCircle className="inline w-4 h-4 mr-2" />
        Question received but no options were provided.
      </div>
    );
  }

  return (
    <div className="border-2 border-blue-300 dark:border-blue-700 rounded-lg overflow-hidden my-2 bg-white dark:bg-gray-900">
      <div className="flex items-center gap-2 px-4 py-2 bg-blue-50 dark:bg-blue-900/30">
        <HelpCircle className="w-5 h-5 text-blue-500" />
        <span className="font-medium text-blue-700 dark:text-blue-300">
          {isSubmitted ? "Question answered" : "Claude is asking"}
        </span>
      </div>

      <div className="px-4 py-3 space-y-4">
        {questions.map((q, qIdx) => {
          const selectedSet = selections.get(qIdx) ?? new Set<number>();
          const submittedLabel = submittedAnswers?.[q.question];
          const multi = q.multiSelect === true;
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
                <div className="text-sm text-gray-700 dark:text-gray-200 bg-gray-50 dark:bg-gray-800 rounded px-3 py-2 border border-gray-200 dark:border-gray-700">
                  <span className="text-gray-500 dark:text-gray-400">Your answer: </span>
                  <span className="font-medium">{submittedLabel ?? "—"}</span>
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
