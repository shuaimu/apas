"use client";

import { useEffect, useState } from "react";
import type { JobArtifactContent, JobDetail, JobReview } from "@apas/protocol";
import { jobFetch, jobPath } from "@/lib/jobs";
import { jobCacheRevision, onJobCachePurge, readJobRecord, writeJobRecord } from "@/lib/jobCache";
import type { JobScope } from "@/lib/jobCache";
import { JobError, Publication } from "./JobElements";

export function JobReviewScreen({ scope, detail, ready, runId = detail.job.current_run_id, endpoint = jobPath(scope[1], scope[2]) }: { scope: JobScope; detail: JobDetail; ready: boolean; runId?: string; endpoint?: string }) {
  const [review, setReview] = useState<JobReview>();
  const [cursor, setCursor] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [artifact, setArtifact] = useState<{ id: string; label: string }>();
  const [error, setError] = useState("");
  const scopeKey = JSON.stringify(scope);
  useEffect(() => onJobCachePurge(() => { setReview(undefined); setArtifact(undefined); setError("Access changed. Refresh authorization before reading evidence."); }), []);
  useEffect(() => {
    let active = true;
    const revision = jobCacheRevision();
    setReview(undefined);
    const load = async () => {
      const scope = JSON.parse(scopeKey) as JobScope;
      const key = `${endpoint}:${runId}:${cursor}`;
      const cache = await readJobRecord<JobReview>(scope, "review", key);
      if (active && revision === jobCacheRevision() && cache?.run_id === runId) setReview(cache);
      if (!ready || !active || revision !== jobCacheRevision()) return;
      const response = await jobFetch<JobReview>(`${endpoint}/review?limit=40&run_id=${encodeURIComponent(runId)}${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`, "review");
      if (response.run_id !== runId || response.job_id !== scope[2]) throw new Error("The requested run does not match this review. No later result has been substituted.");
      await writeJobRecord(scope, "review", response, key, revision);
      if (active && revision === jobCacheRevision()) { setReview(response); setError(""); }
    };
    void load().catch((failure: unknown) => { if (active && revision === jobCacheRevision()) setError(failure instanceof Error ? failure.message : "Review evidence unavailable."); });
    return () => { active = false; };
  }, [scopeKey, ready, cursor, endpoint, runId]);
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
    {artifact && <ArtifactReader key={`${scopeKey}:${endpoint}:${runId}:${artifact.id}`} scope={scope} endpoint={endpoint} runId={runId} id={artifact.id} label={artifact.label} ready={ready} onClose={() => setArtifact(undefined)} />}
  </section>;
}
function ArtifactReader({ scope, endpoint, runId, id, label, ready, onClose }: { scope: JobScope; endpoint: string; runId: string; id: string; label: string; ready: boolean; onClose: () => void }) {
  const [content, setContent] = useState<JobArtifactContent>();
  const [cursor, setCursor] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const scopeKey = JSON.stringify(scope);
  useEffect(() => onJobCachePurge(() => { setContent(undefined); setLoading(false); setError("Access changed. Evidence cleared."); }), []);
  useEffect(() => {
    let active = true;
    const revision = jobCacheRevision();
    setLoading(true); setError(""); setContent(undefined);
    const load = async () => {
      try {
        const scope = JSON.parse(scopeKey) as JobScope;
        const key = `${endpoint}:${runId}:${id}:${cursor}`;
        const cache = await readJobRecord<JobArtifactContent>(scope, "artifact", key);
        if (!active || revision !== jobCacheRevision()) return;
        if (cache) setContent(cache);
        if (!ready) { if (!cache) setError("This evidence page is not cached. Reconnect and refresh authorization to read it."); return; }
        const parameter = endpoint.includes("/workers/") ? "offset" : "cursor";
        const response = await jobFetch<JobArtifactContent>(`${endpoint}/artifacts/${encodeURIComponent(id)}?limit=32768${cursor ? `&${parameter}=${encodeURIComponent(cursor)}` : ""}`, "artifact");
        if (response.artifact.run_id !== runId || response.artifact.id !== id) throw new Error("Evidence belongs to a different run.");
        await writeJobRecord(scope, "artifact", response, key, revision);
        if (active && revision === jobCacheRevision()) setContent(response);
      } catch (failure) { if (active && revision === jobCacheRevision()) setError(failure instanceof Error ? failure.message : "Evidence could not be read."); }
      finally { if (active && revision === jobCacheRevision()) setLoading(false); }
    };
    void load();
    return () => { active = false; };
  }, [scopeKey, endpoint, runId, id, cursor, ready]);
  useEffect(() => { document.getElementById("job-artifact-heading")?.focus(); }, [id]);
  return <section className="job-card" aria-labelledby="job-artifact-heading"><h3 id="job-artifact-heading" tabIndex={-1}>{label}</h3><button onClick={onClose}>Close evidence</button><JobError message={error} />
    {content && <><p>SHA-256 <code>{content.artifact.sha256}</code> · {content.artifact.size_bytes.toLocaleString()} bytes</p>{content.artifact.expires_at && <p>Retention expires {new Date(content.artifact.expires_at).toLocaleString()}</p>}{content.artifact.truncated && <p className="job-notice">This artifact was truncated at its retention limit. It is not the complete original output.</p>}{content.expired ? <p className="job-notice">This artifact has expired. Its manifest is retained but content is no longer available.</p> : <pre className="job-diff" tabIndex={0} aria-label={label}>{content.text}</pre>}</>}
    {loading && <p role="status">Loading a bounded evidence page…</p>}
    <div className="job-actions"><button disabled={loading || !history.length} onClick={() => { setContent(undefined); setCursor(history[history.length - 1]); setHistory((items) => items.slice(0, -1)); }}>Previous evidence page</button><button disabled={loading || !ready || !content?.next_cursor || content.expired} onClick={() => { setHistory((items) => [...items, cursor]); setCursor(content!.next_cursor!); setContent(undefined); }}>Next evidence page</button></div>
  </section>;
}
