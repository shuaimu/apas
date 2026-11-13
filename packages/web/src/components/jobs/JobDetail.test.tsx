import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { JobDetail, JobQuestion } from "@apas/protocol";
import { JobDetailScreen, JobQuestionForm } from "./JobDetail";

const hooks = vi.hoisted(() => ({ detail: undefined as JobDetail | undefined, submit: vi.fn() }));
vi.mock("@/lib/store", () => ({ useStore: (selector: (state: { userId: string }) => unknown) => selector({ userId: "account" }) }));
vi.mock("@/lib/useJobs", () => ({
  useJob: () => ({ saved: { detail: hooks.detail, events: [] }, ready: true, project: "project", error: "", refresh: vi.fn() }),
  useJobDraft: (_scope: unknown, _key: string, initial: unknown) => ({ value: initial, loaded: true, error: "", update: vi.fn(), flush: vi.fn() }),
  useJobMutation: () => ({ submit: hooks.submit, busy: false, error: "", unresolved: false, retryAllowed: false, resolve: vi.fn() }),
}));
vi.mock("./JobReview", () => ({ JobReviewScreen: ({ runId }: { runId: string }) => <p>Review run {runId}</p> }));
const question: JobQuestion = { id: "question", run_id: "old-run", epoch: 2, version: 3, dialog_id: "dialog", tool_call_id: "call", state: "pending", questions: [{ id: "choice", header: "Decision", question: "Choose a disposition", multi: false, options: [], recommended: null }], answers: null, created_at: "2026-11-12T00:00:00Z", confirmed_at: null };
afterEach(() => cleanup());

it("keeps standalone questions read-only even when legacy authority allows answering", () => {
  hooks.detail = { job: { id: "job", title: "Earlier task", project_name: "Project", current_run_id: "old-run", allowed_actions: { answer: true } }, task: {}, runs: [{ id: "old-run", state: "needs_input" }], questions: [question] } as JobDetail;
  render(<JobDetailScreen id="job" projectHint="project" runId="old-run" questionId="question" />);
  expect((screen.getByRole("button", { name: "Submit all answers for this dialog" }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByLabelText("Custom text for Decision") as HTMLTextAreaElement).closest("fieldset:disabled")).not.toBeNull();
  expect(screen.queryByRole("button", { name: "Retry coding from approved base" })).toBeNull();
  expect(screen.getByRole("link", { name: "Reuse instructions in an unapproved worker draft" }).getAttribute("href")).toBe("/code/workers/new?project=project&source_job=job");
  expect(hooks.submit).not.toHaveBeenCalled();
});

it("does not present accepted or delivered answers as provider confirmation", () => {
  const accepted = { ...question, state: "answer_delivered" as const, answers: [{ id: "choice", selected_option_ids: [], custom_input: "Retain evidence" }] };
  const { rerender } = render(<JobQuestionForm question={accepted} scope={["account", "project", "job"]} currentRun="old-run" ready answerAllowed refresh={vi.fn()} answerPath="/projects/project/workers/worker/runs/old-run/questions/question/answers" />);
  expect(screen.getByText("Answer delivered. Provider confirmation is still pending.")).toBeTruthy();
  expect((screen.getByRole("button", { name: "Submit all answers for this dialog" }) as HTMLButtonElement).disabled).toBe(true);
  rerender(<JobQuestionForm question={{ ...accepted, state: "confirmed", confirmed_at: "2026-11-12T01:00:00Z" }} scope={["account", "project", "job"]} currentRun="old-run" ready answerAllowed refresh={vi.fn()} />);
  expect(screen.getByRole("heading", { name: "Provider-confirmed answers" })).toBeTruthy();
});

it("keeps an old pending dialog disabled when a later run is current", () => {
  render(<JobQuestionForm question={question} scope={["account", "project", "job"]} currentRun="new-run" ready answerAllowed refresh={vi.fn()} />);
  expect(screen.getByText(/Stale run: these questions belong to an earlier attempt/)).toBeTruthy();
  expect((screen.getByRole("button", { name: "Submit all answers for this dialog" }) as HTMLButtonElement).disabled).toBe(true);
});
