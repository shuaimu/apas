"use client";

import { useCallback, useEffect, useState } from "react";
import type { JobArtifactContent, JobDetail, JobReview } from "@apas/protocol";
import { jobFetch, jobPath } from "@/lib/jobs";
import { jobCacheRevision, readJobRecord, writeJobRecord } from "@/lib/jobCache";
import type { JobScope } from "@/lib/jobCache";
import { JobError, Publication } from "./JobElements";

export function JobReviewScreen({ scope, detail, ready }: { scope: JobScope; detail: JobDetail; ready: boolean }) {
  const [review, setReview] = useState<JobReview>();
  const [cursor, setCursor] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [artifact, setArtifact] = useState<{ id: string; label: string }>();
  const [error, setError] = useState("");
  const scopeKey = JSON.stringify(scope);
  useEffect(() => {
    let active = true;
    const load = async () => {
      const scope = JSON.parse(scopeKey) as JobScope;
      const revision = jobCacheRevision();
      const cache = await readJobRecord<JobReview>(scope, "review", cursor);
      if (active && cache) setReview(cache);
      if (!ready) return;
      const response = await jobFetch<JobReview>(`${jobPath(scope[1], scope[2])}/review?limit=40${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`, "review");
      await writeJobRecord(scope, "review", response, cursor, revision);
      if (active) { setReview(response); setError(""); }
    };
    void load().catch((failure: unknown) => { if (active) setError(failure instanceof Error ? failure.message : "Review evidence unavailable."); });
    return () => { active = false; };
  }, [scopeKey, ready, cursor]);
  return <section aria-label="Retained review"><h2>Candidate review and independent checks</h2>
    <JobError message={error} />
    <section className="job-card"><h3>Admitted task</h3><p className="job-prose">{detail.task.instruction}</p><ul>{detail.task.acceptance_criteria.map((criterion, index) => <li key={index}>{criterion}</li>)}</ul><p>Repository: {detail.task.repository}</p><p>Base: {detail.task.base_ref} · <code>{review?.base_sha ?? detail.task.base_sha}</code></p><p>Candidate: <code>{review?.candidate_sha ?? "No candidate retained yet"}</code></p>{review && <p>Evidence run: <code>{review.run_id}</code></p>}</section>
    {review?.reason && <p className="job-notice">{review.reason}</p>}
    <Publication publication={review?.publication} />
    <section className="job-card"><h3>Changed files</h3>{review?.files.length ? <ul className="job-files">{review.files.map((file) => <li key={file.path}><p><strong>{file.path}</strong> · +{file.additions} / −{file.deletions}{file.binary ? " · binary" : ""}</p>{file.unavailable_reason && <p>{file.unavailable_reason}</p>}{file.artifact_id ? <button onClick={() => setArtifact({ id: file.artifact_id!, label: `Unified diff: ${file.path}` })}>Read diff for {file.path}</button> : <p>{file.binary ? "Binary content is not rendered as a text diff." : "Diff content is unavailable."}</p>}</li>)}</ul> : <p>{review ? "No changed files on this page." : "Loading retained candidate manifest…"}</p>}
      <div className="job-actions"><button disabled={!history.length} onClick={() => { setCursor(history[history.length - 1]); setHistory((items) => items.slice(0, -1)); setArtifact(undefined); }}>Previous file page</button><button disabled={!review?.next_cursor || !ready} onClick={() => { setHistory((items) => [...items, cursor]); setCursor(review!.next_cursor!); setArtifact(undefined); }}>Next file page</button></div>
    </section>
    <section className="job-card"><h3>Independent verification</h3>{review?.verification ? <>
      <p>Verified candidate <code>{review.verification.candidate_sha}</code> against base <code>{review.verification.base_sha}</code></p><p>Completed {new Date(review.verification.completed_at).toLocaleString()}</p>
      <details><summary>Trusted verifier identity</summary><p>Profile digest <code>{review.verification.profile_digest}</code></p><p>Image digest <code>{review.verification.image_digest}</code></p></details>
      {review.verification.checks.map((check, index) => <div className="job-check" key={`${check.name}:${index}`}><h4>{check.name}: {check.status.replaceAll("_", " ")}</h4><p>Exit {check.exit_code ?? "unknown"} · {check.elapsed_seconds}s</p><details><summary>Check command</summary><pre tabIndex={0}>{JSON.stringify(check.argv)}</pre></details>{check.log_artifact_id ? <button onClick={() => setArtifact({ id: check.log_artifact_id!, label: `Check log: ${check.name}` })}>Read check log</button> : <p>No retained check log is available.</p>}</div>)}
    </> : <p>No independent verification result has been recorded. Worker-authored success is not verification.</p>}</section>
    {artifact && <ArtifactReader key={`${scopeKey}:${artifact.id}`} scope={scope} id={artifact.id} label={artifact.label} ready={ready} onClose={() => setArtifact(undefined)} />}
  </section>;
}
function ArtifactReader({ scope, id, label, ready, onClose }: { scope: JobScope; id: string; label: string; ready: boolean; onClose: () => void }) {
  const [content, setContent] = useState<JobArtifactContent>();
  const [cursor, setCursor] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const scopeKey = JSON.stringify(scope);
  const load = useCallback(async () => {
    setLoading(true); setError("");
    const revision = jobCacheRevision();
    try {
      const scope = JSON.parse(scopeKey) as JobScope;
      const key = `${id}:${cursor}`;
      const cache = await readJobRecord<JobArtifactContent>(scope, "artifact", key);
      if (cache && revision === jobCacheRevision()) setContent(cache);
      if (!ready) { if (!cache) setError("This evidence page is not cached. Reconnect and refresh authorization to read it."); return; }
      const response = await jobFetch<JobArtifactContent>(`${jobPath(scope[1], scope[2])}/artifacts/${encodeURIComponent(id)}?limit=32768${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`, "artifact");
      await writeJobRecord(scope, "artifact", response, key, revision);
      if (revision === jobCacheRevision()) setContent(response);
    } catch (failure) { if (revision === jobCacheRevision()) setError(failure instanceof Error ? failure.message : "Evidence could not be read."); }
    finally { setLoading(false); }
  }, [scopeKey, id, cursor, ready]);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => { document.getElementById("job-artifact-heading")?.focus(); }, [id]);
  return <section className="job-card" aria-labelledby="job-artifact-heading"><h3 id="job-artifact-heading" tabIndex={-1}>{label}</h3><button onClick={onClose}>Close evidence</button><JobError message={error} />
    {content && <><p>SHA-256 <code>{content.artifact.sha256}</code> · {content.artifact.size_bytes.toLocaleString()} bytes</p>{content.artifact.expires_at && <p>Retention expires {new Date(content.artifact.expires_at).toLocaleString()}</p>}{content.artifact.truncated && <p className="job-notice">This artifact was truncated at its retention limit. It is not the complete original output.</p>}{content.expired ? <p className="job-notice">This artifact has expired. Its manifest is retained but content is no longer available.</p> : <pre className="job-diff" tabIndex={0} aria-label={label}>{content.text}</pre>}</>}
    {loading && <p role="status">Loading a bounded evidence page…</p>}
    <div className="job-actions"><button disabled={loading || !history.length} onClick={() => { setContent(undefined); setCursor(history[history.length - 1]); setHistory((items) => items.slice(0, -1)); }}>Previous evidence page</button><button disabled={loading || !ready || !content?.next_cursor || content.expired} onClick={() => { setHistory((items) => [...items, cursor]); setCursor(content!.next_cursor!); setContent(undefined); }}>Next evidence page</button></div>
  </section>;
}
