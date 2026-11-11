import type { JobEvent, JobEventsResponse, JobQuestion } from "@apas/protocol";
import { acceptJobPage, boundedEvents, safePullRequest, validAnswers } from "./continuity";
import { parseJobLink } from "./links";

const job = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const question = "28dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const event = (seq: number, jobId = job): JobEvent => ({ id: `event-${seq}`, job_id: jobId, run_id: null, seq, kind: "progress", summary: "Progress", created_at: "2026-01-01T00:00:00Z", detail: null });
const page = (events: JobEvent[], next = 2): JobEventsResponse => ({ events, next_seq: next, high_watermark: next, has_more: false, resync_required: false });

test("accepts only exact-job contiguous events and coherent durable cursors", () => {
  expect(acceptJobPage(job, 0, page([event(1), event(2)]))).toBe(2);
  expect(() => acceptJobPage(job, 0, page([event(2)]))).toThrow("gap");
  expect(() => acceptJobPage(job, 0, page([event(1, question)], 1))).toThrow("gap");
  expect(() => acceptJobPage(job, 0, page([event(1)], 2))).toThrow("watermark");
  expect(() => acceptJobPage(job, 0, { ...page([], 0), has_more: true })).toThrow("watermark");
  expect(() => acceptJobPage(job, 0, { ...page([], 0), resync_required: true })).toThrow("expired");
  expect(acceptJobPage(job, 2, page([], 2))).toBe(2);
});

test("bounds timeline without changing event order or identities", () => {
  const events = Array.from({ length: 250 }, (_, i) => event(i + 1));
  const retained = boundedEvents(events.slice(0, 150), events.slice(150));
  expect(retained).toHaveLength(200);
  expect(retained[0].seq).toBe(51);
  expect(retained[199].seq).toBe(250);
});

test("validates every question and preserves custom multi-select answers", () => {
  const dialog = { questions: [
    { id: "first", multi: false, options: [{ id: "a" }, { id: "b" }] },
    { id: "second", multi: true, options: [{ id: "x" }, { id: "y" }] },
  ] } as JobQuestion;
  const answers = [{ id: "first", selected_option_ids: ["a"], custom_input: null }, { id: "second", selected_option_ids: ["x", "y"], custom_input: "Keep the existing API" }];
  expect(validAnswers(dialog, answers)).toBe(true);
  expect(validAnswers(dialog, answers.slice(0, 1))).toBe(false);
  expect(validAnswers(dialog, [{ ...answers[0], selected_option_ids: ["a", "b"] }, answers[1]])).toBe(false);
  expect(validAnswers(dialog, [answers[0], { ...answers[1], selected_option_ids: ["unknown"] }])).toBe(false);
  expect(validAnswers(dialog, [answers[0], { ...answers[1], selected_option_ids: [], custom_input: "Custom choice" }])).toBe(true);
});

test("job links retain exact opaque context without session or mutation fallback", () => {
  expect(parseJobLink(`apas://code/jobs/${job}/questions/${question}?run=${job}`)).toEqual({ job, question, review: false, run: job });
  expect(parseJobLink(`https://apas.mpaxos.com/code/jobs/${job}/review`)).toEqual({ job, review: true, run: undefined });
  expect(parseJobLink(`https://attacker.invalid/code/jobs/${job}`)).toBeNull();
  expect(parseJobLink(`apas://code/jobs/${job}/cancel`)).toBeNull();
  expect(parseJobLink(`apas://code/jobs/${job}?run=other`)).toBeNull();
  expect(parseJobLink(`apas://code/session/${job}`)).toBeNull();
});

test("PR handoff cannot escape the exact HTTPS repository", () => {
  expect(safePullRequest("https://github.com/owner/repo/pull/17", "owner/repo")).toBe(true);
  for (const url of ["https://github.com/other/repo/pull/17", "https://github.com.evil.invalid/owner/repo/pull/17", "https://token@github.com/owner/repo/pull/17", "https://github.com/owner/repo/pull/17?redirect=evil", "javascript:alert(1)"]) expect(safePullRequest(url, "owner/repo")).toBe(false);
});
