"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import type { JobLimits, JobPublication, JobUsage } from "@apas/protocol";
import { jobHref } from "@/lib/jobs";
import type { SavedJobRequest } from "@/lib/jobCache";

export const limitLabels: Record<keyof JobLimits, string> = {
  elapsed_seconds: "Total elapsed seconds", spend_micro_usd: "Reported spend threshold (micro USD)", attempts: "Maximum coding attempts",
  cpu_millis: "CPU allowance (millicores)", memory_bytes: "Memory (bytes)", workspace_bytes: "Workspace (bytes)",
  output_bytes: "Retained output (bytes)", pids: "Process limit", model_concurrency: "Concurrent model requests", model_output_tokens: "Maximum output tokens per request",
};
export function JobLink({ href, onOpen, className, current, children }: {
  href: string; onOpen?: () => void; className?: string; current?: boolean; children: ReactNode;
}) {
  return onOpen
    ? <button type="button" className={className} aria-current={current ? "page" : undefined} onClick={onOpen}>{children}</button>
    : <Link href={href} className={className} aria-current={current ? "page" : undefined}>{children}</Link>;
}
export function JobError({ message }: { message?: string }) {
  return message ? <p className="job-notice job-error" role="alert">{message}</p> : null;
}
export function Usage({ usage, limits }: { usage: JobUsage; limits?: JobLimits }) {
  return <dl className="job-facts">
    <div><dt>Elapsed</dt><dd>{usage.elapsed_seconds}s{limits ? ` / ${limits.elapsed_seconds}s cumulative allowance` : ""}</dd></div>
    <div><dt>{usage.estimated ? "Estimated spend" : "Reported spend"}</dt><dd>{usage.spend_micro_usd == null ? "Unknown — not zero" : `$${(usage.spend_micro_usd / 1_000_000).toFixed(6)}`}{limits ? ` / $${(limits.spend_micro_usd / 1_000_000).toFixed(6)} threshold` : ""}</dd></div>
    <div><dt>Tokens</dt><dd>{usage.input_tokens.toLocaleString()} input · {usage.output_tokens.toLocaleString()} output</dd></div>
  </dl>;
}
export function Publication({ publication }: { publication?: JobPublication | null }) {
  if (!publication) return <p>No draft pull request has been published. Retained candidate and checks can still be reviewed below.</p>;
  let allowed = false;
  try {
    const url = new URL(publication.url);
    const repository = publication.repository.replace(/^https:\/\/github\.com\//, "").replace(/^github\.com\//, "").replace(/\.git$/, "");
    allowed = url.protocol === "https:" && url.hostname === "github.com" && !url.username && !url.password && !url.port && url.pathname === `/${repository}/pull/${publication.number}`;
  } catch { /* An invalid provider URL is never navigable. */ }
  return <section className="job-card" aria-label="Pull request disposition">
    <h2>{publication.draft ? "Draft pull request" : "External pull request"} #{publication.number}</h2>
    <p>{publication.state} · last checked <time dateTime={publication.observed_at}>{new Date(publication.observed_at).toLocaleString()}</time></p>
    <p>Head <code>{publication.head_sha}</code> · base <code>{publication.base_sha}</code></p>
    {allowed ? <a className="job-button" href={publication.url} target="_blank" rel="noopener noreferrer">Open pull request on GitHub (new tab)</a> : <p role="alert">The publication URL does not match the approved repository. External navigation is disabled.</p>}
    <p>Review here does not approve or merge the pull request.</p>
  </section>;
}
export function RequestStatus({ request, busy, retryAllowed, canRetry, canResolve = canRetry, onResolve, onRetry, onOpenJob }: {
  request?: SavedJobRequest; busy: boolean; retryAllowed: boolean; canRetry: boolean; canResolve?: boolean;
  onResolve: () => Promise<void>; onRetry: () => Promise<unknown>; onOpenJob?: (jobId: string) => void;
}) {
  if (!request) return null;
  const receipt = request.receipt;
  const unresolved = !receipt || receipt.status === "pending";
  return <section className="job-notice" aria-label="Durable request status">
    <p role="status">{receipt ? `Request ${receipt.status}` : "Response unconfirmed — this is not a failed job."}</p>
    <p>Request <code>{request.id}</code></p>
    {receipt?.message && <p>{receipt.message}</p>}
    {receipt?.error_code && <p>Reason: {receipt.error_code}</p>}
    {receipt?.status === "rejected" && <p>Refresh current job state or obtain a fresh admission review before submitting changed inputs.</p>}
    {receipt?.job_id && <JobLink className="job-button" href={jobHref(request.scope[1], receipt.job_id)} onOpen={onOpenJob ? () => onOpenJob(receipt.job_id!) : undefined}>Open original job</JobLink>}
    {unresolved && <><p>Check the saved request before retrying. Reconnect never resubmits it automatically.</p>
      <button type="button" disabled={busy || !canResolve} onClick={() => void onResolve()}>Check original receipt</button>
      {retryAllowed && <button type="button" disabled={busy || !canRetry} onClick={() => void onRetry()}>Retry saved request with the same ID</button>}
    </>}
  </section>;
}
