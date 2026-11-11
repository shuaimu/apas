import type { JobEvent, JobEventsResponse, JobQuestion, JobQuestionAnswer } from "@apas/protocol";

export function acceptJobPage(job: string, cursor: number, page: JobEventsResponse): number {
  if (page.resync_required) throw new Error("Job history expired; a fresh snapshot is required");
  let next = cursor;
  for (const event of page.events) {
    if (event.job_id !== job || event.seq !== next + 1) throw new Error("Job event gap; synchronization required");
    next = event.seq;
  }
  if (page.next_seq !== next || page.high_watermark < next || (page.has_more && next === cursor)) {
    throw new Error("Invalid job event watermark");
  }
  return next;
}

export function boundedEvents(existing: JobEvent[], incoming: JobEvent[]): JobEvent[] {
  return [...existing, ...incoming].slice(-200);
}

export function validAnswers(question: JobQuestion, answers: JobQuestionAnswer[]): boolean {
  return answers.length === question.questions.length && question.questions.every((item, index) => {
    const answer = answers[index];
    return answer.id === item.id && new Set(answer.selected_option_ids).size === answer.selected_option_ids.length
      && (item.multi || answer.selected_option_ids.length <= 1)
      && answer.selected_option_ids.every((id) => item.options.some((option) => option.id === id))
      && (answer.selected_option_ids.length > 0 || Boolean(answer.custom_input?.trim()));
  });
}

export function safePullRequest(url: string, repository: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" && parsed.hostname === "github.com" && !parsed.port
      && !parsed.username && !parsed.password && !parsed.search && !parsed.hash
      && parsed.pathname.startsWith(`/${repository}/pull/`)
      && /^\d+$/.test(parsed.pathname.slice(`/${repository}/pull/`.length));
  } catch { return false; }
}
