use super::*;
use crate::jobs::config::{AutonomousJobConfig, PublisherProfileConfig};
use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};
use shared::JobPublication;
use std::future::Future;

pub async fn publish_candidate(
    ctx: &DeliveryContext,
    config: &AutonomousJobConfig,
    expected_candidate: &str,
    fence: &DeliveryFence,
) -> Result<JobPublication> {
    let _lock = ctx.lock()?;
    let mut record = ctx.load()?;
    ensure_retained(&record)?;
    let result = publish_checked(ctx, config, expected_candidate, &mut record, fence).await;
    if let Err(error) = &result {
        record.review.reason = Some(format!("publication_blocked: {error}"));
        if let Some(journal) = &mut record.publication {
            journal.error = Some(error.to_string());
        }
        ctx.save(&record)?;
    }
    result
}

async fn publish_checked(
    ctx: &DeliveryContext,
    config: &AutonomousJobConfig,
    expected: &str,
    record: &mut DeliveryRecord,
    fence: &DeliveryFence,
) -> Result<JobPublication> {
    fence.check()?;
    let candidate = record.candidate.as_ref().context("candidate_missing")?;
    if candidate.candidate_sha != expected {
        bail!("publication_candidate_mismatch");
    }
    candidate::validate_candidate(ctx, candidate, fence).await?;
    let verifier = verify::selected_profile(ctx, config)?;
    let evidence = record
        .review
        .verification
        .as_ref()
        .context("independent_verification_missing")?;
    if !verify::verification_passes(
        evidence,
        verifier,
        expected,
        &ctx.task.base_sha,
        &ctx.task.verification_profile_digest,
    ) {
        bail!("independent_verification_not_passing_for_exact_candidate");
    }
    let api = GithubPublisher::new(ctx, config, expected)?;
    api.validate_target(fence).await?;
    publish_with(ctx, record, &api, expected, fence).await
}

/// Read-only reconciliation is deliberately allowed after cancellation. It can
/// discover a request already accepted by GitHub, but never creates/deletes a
/// branch or PR and never resumes coding or verification.
pub async fn reconcile_publication(
    ctx: &DeliveryContext,
    config: &AutonomousJobConfig,
) -> Result<Option<JobPublication>> {
    let _lock = ctx.lock()?;
    let mut record = ctx.load()?;
    let Some(candidate) = record.candidate.as_ref() else {
        return Ok(None);
    };
    if record.publication.is_none() {
        return Ok(None);
    }
    let api = GithubPublisher::new(ctx, config, &candidate.candidate_sha)?;
    let found = api.find().await?;
    if let Some(publication) = found {
        remember_publication(ctx, &mut record, &publication)?;
        return Ok(Some(publication));
    }
    if record.review.publication.is_some() {
        bail!("recorded_pull_request_no_longer_resolves");
    }
    if record
        .publication
        .as_ref()
        .is_some_and(|journal| journal.phase == "push_in_flight")
    {
        match api.branch_head().await? {
            Some(head) if head == api.candidate => set_phase(ctx, &mut record, "pushed")?,
            Some(_) => bail!("external_job_branch_changed"),
            None => {}
        }
    }
    Ok(None)
}

trait PublicationRemote: Sync {
    fn find(&self) -> impl Future<Output = Result<Option<JobPublication>>> + Send;
    fn branch_head(&self) -> impl Future<Output = Result<Option<String>>> + Send;
    fn push(&self, fence: &DeliveryFence) -> impl Future<Output = Result<()>> + Send;
    fn create(&self, fence: &DeliveryFence) -> impl Future<Output = Result<CreateOutcome>> + Send;
}

enum CreateOutcome {
    Created(JobPublication),
    Rejected(String),
    Uncertain,
}

async fn publish_with<R: PublicationRemote>(
    ctx: &DeliveryContext,
    record: &mut DeliveryRecord,
    remote: &R,
    expected: &str,
    fence: &DeliveryFence,
) -> Result<JobPublication> {
    let branch = format!("apas/job-{}", ctx.identity.job_id);
    if let Some(journal) = &record.publication {
        if journal.candidate_sha != expected
            || journal.repository != ctx.task.repository
            || journal.branch != branch
        {
            bail!("publication_journal_identity_conflict");
        }
    } else {
        record.publication = Some(PublicationJournal {
            phase: "prepared".into(),
            candidate_sha: expected.into(),
            repository: ctx.task.repository.clone(),
            branch,
            error: None,
        });
        ctx.save(record)?;
    }
    // Always reconcile before any retry, including a retry after a lost reply.
    if let Some(found) = remote.find().await? {
        if found.head_sha != expected {
            bail!("external_pull_request_head_changed");
        }
        remember_publication(ctx, record, &found)?;
        return Ok(found);
    }
    if record.review.publication.is_some() {
        bail!("recorded_pull_request_no_longer_resolves");
    }
    if record
        .publication
        .as_ref()
        .is_some_and(|j| matches!(j.phase.as_str(), "create_in_flight" | "create_uncertain"))
    {
        // GitHub has no idempotency key for PR creation. A successful lookup
        // resolves uncertainty; an empty lookup does not prove a lost POST was
        // rejected. Never trade at-most-one PR for a blind replay.
        bail!("publication_response_uncertain_reconciliation_required");
    }
    fence.check()?;
    match remote.branch_head().await? {
        Some(head) if head != expected => bail!("external_job_branch_changed"),
        Some(_) => {}
        None => {
            fence.check()?;
            set_phase(ctx, record, "push_in_flight")?;
            let push = remote.push(fence).await;
            // Read-only confirmation also runs after cancellation or a lost
            // push reply. It cannot overwrite or remove an external branch.
            match remote.branch_head().await? {
                Some(head) if head == expected => {}
                Some(_) => bail!("external_job_branch_changed"),
                None => {
                    push?;
                    bail!("publication_push_not_observed");
                }
            }
        }
    }
    set_phase(ctx, record, "pushed")?;
    // An external actor may have created the PR while objects were uploaded.
    if let Some(found) = remote.find().await? {
        remember_publication(ctx, record, &found)?;
        return Ok(found);
    }
    fence.check()?;
    if remote.branch_head().await?.as_deref() != Some(expected) {
        bail!("external_job_branch_changed");
    }
    fence.check()?;
    set_phase(ctx, record, "create_in_flight")?;
    if let Err(error) = fence.check() {
        set_phase(ctx, record, "pushed")?;
        return Err(error);
    }
    match remote.create(fence).await {
        Ok(CreateOutcome::Created(publication)) => {
            remember_publication(ctx, record, &publication)?;
            Ok(publication)
        }
        Ok(CreateOutcome::Rejected(reason)) => {
            // 422 can be GitHub's concurrent-creation response, so look up
            // before exposing a definitive rejection or allowing later retry.
            if let Some(found) = remote.find().await? {
                remember_publication(ctx, record, &found)?;
                return Ok(found);
            }
            set_phase(ctx, record, "create_rejected")?;
            bail!("draft_creation_rejected: {reason}")
        }
        Ok(CreateOutcome::Uncertain) | Err(_) => {
            set_phase(ctx, record, "create_uncertain")?;
            if let Some(found) = remote.find().await? {
                remember_publication(ctx, record, &found)?;
                return Ok(found);
            }
            bail!("publication_response_uncertain_reconciliation_required")
        }
    }
}

fn set_phase(ctx: &DeliveryContext, record: &mut DeliveryRecord, phase: &str) -> Result<()> {
    let journal = record
        .publication
        .as_mut()
        .context("publication_journal_missing")?;
    journal.phase = phase.into();
    ctx.save(record)
}

fn remember_publication(
    ctx: &DeliveryContext,
    record: &mut DeliveryRecord,
    publication: &JobPublication,
) -> Result<()> {
    let journal = record
        .publication
        .as_ref()
        .context("publication_journal_missing")?;
    if publication.repository != journal.repository
        || publication.branch != journal.branch
        || publication.head_sha != journal.candidate_sha
        || publication.base_sha != ctx.task.base_sha
    {
        bail!("publication_response_identity_conflict");
    }
    if let Some(previous) = &record.review.publication {
        if previous.number != publication.number {
            bail!("duplicate_job_pull_request");
        }
    }
    record.review.publication = Some(publication.clone());
    record.review.reason = None;
    set_phase(ctx, record, "reconciled")?;
    // The review is the canonical refreshable external disposition. Retain one
    // historical publication artifact, not a new artifact on every GET/refresh.
    if !record
        .artifacts
        .iter()
        .any(|a| a.kind == JobArtifactKind::Publication)
    {
        ctx.retain(
            record,
            JobArtifactKind::Publication,
            &serde_json::to_vec(publication)?,
            "application/json",
            false,
        )?;
        ctx.save(record)?;
    }
    Ok(())
}

struct GithubPublisher<'a> {
    ctx: &'a DeliveryContext,
    profile: &'a PublisherProfileConfig,
    client: Client,
    credential: String,
    repository: String,
    base: String,
    branch: String,
    candidate: String,
}

impl<'a> GithubPublisher<'a> {
    fn new(
        ctx: &'a DeliveryContext,
        config: &'a AutonomousJobConfig,
        candidate: &str,
    ) -> Result<Self> {
        let worker = config
            .profiles
            .iter()
            .find(|p| p.key == ctx.task.worker_profile)
            .context("publisher_worker_profile_unavailable")?;
        let profile = config
            .publishers
            .iter()
            .find(|p| p.key == worker.publisher)
            .context("publisher_profile_unavailable")?;
        let repository = canonical_repository(&ctx.task.repository)?;
        if !profile
            .repositories
            .iter()
            .any(|r| canonical_repository(r).ok().as_deref() == Some(&repository))
        {
            bail!("publisher_repository_not_allowed");
        }
        let base = canonical_base(&ctx.task.base_ref)?;
        if !profile
            .protected_base_refs
            .iter()
            .any(|r| canonical_base(r).ok().as_deref() == Some(&base))
        {
            bail!("publisher_base_not_allowed");
        }
        if !profile.ci_qualified {
            bail!("publisher_ci_not_qualified");
        }
        let credential_path = fs::canonicalize(&profile.credential_file)
            .context("dedicated_publication_credential_unavailable")?;
        if credential_path.starts_with(&ctx.workspace_root)
            || credential_path.starts_with(&ctx.worker_output)
        {
            bail!("publication_credential_inside_untrusted_boundary");
        }
        let file = private_open(&profile.credential_file, false)
            .context("dedicated_publication_credential_unavailable")?;
        if file.metadata()?.mode() & 0o077 != 0 {
            bail!("publication_credential_not_private");
        }
        let mut credential = String::new();
        file.take(16 * 1024 + 1).read_to_string(&mut credential)?;
        let credential = credential.trim().to_owned();
        if credential.is_empty()
            || credential.len() > 16 * 1024
            || credential
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        {
            bail!("invalid_publication_credential");
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("APAS-autonomous-draft-publisher/1")
            .build()?;
        Ok(Self {
            ctx,
            profile,
            client,
            credential,
            repository,
            base,
            branch: format!("apas/job-{}", ctx.identity.job_id),
            candidate: candidate.into(),
        })
    }

    fn url(&self, suffix: &str) -> String {
        format!("https://api.github.com/repos/{}{suffix}", self.repository)
    }

    async fn request(
        &self,
        method: Method,
        suffix: &str,
        body: Option<&Value>,
    ) -> Result<(StatusCode, Value)> {
        let mut request = self
            .client
            .request(method, self.url(suffix))
            .bearer_auth(&self.credential)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request = request.json(body);
        }
        // Never format reqwest's request object: its headers contain credentials.
        let mut response = request.send().await.context("github_request_failed")?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("github_response_incomplete")?
        {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                bail!("github_response_exceeds_bound");
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).context("invalid_github_response")?
        };
        Ok((status, value))
    }

    async fn validate_target(&self, fence: &DeliveryFence) -> Result<()> {
        fence.check()?;
        let (status, repository) = self.request(Method::GET, "", None).await?;
        if status != StatusCode::OK
            || repository["full_name"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some(&self.repository.to_ascii_lowercase())
            || repository["permissions"]["push"].as_bool() != Some(true)
        {
            bail!("publication_repository_access_denied");
        }
        if repository["archived"].as_bool() != Some(false)
            || repository["disabled"].as_bool() == Some(true)
        {
            bail!("publication_repository_unavailable");
        }
        fence.check()?;
        let (status, branch) = self
            .request(
                Method::GET,
                &format!("/branches/{}", self.base.replace('/', "%2F")),
                None,
            )
            .await?;
        if status != StatusCode::OK || branch["protected"].as_bool() != Some(true) {
            bail!("publication_base_not_protected");
        }
        if branch["commit"]["sha"].as_str() != Some(&self.ctx.task.base_sha) {
            bail!("publication_base_changed_since_admission");
        }
        if !self.profile.ci_qualified {
            bail!("publisher_ci_not_qualified");
        }
        Ok(())
    }

    fn parse_pr(&self, pr: &Value) -> Result<JobPublication> {
        let number = pr["number"]
            .as_u64()
            .filter(|n| *n > 0)
            .context("invalid_pull_request_number")?;
        let expected_url = format!("https://github.com/{}/pull/{number}", self.repository);
        let marker = format!(
            "<!-- apas-job:{} candidate:{} -->",
            self.ctx.identity.job_id, self.candidate
        );
        if pr["head"]["ref"].as_str() != Some(&self.branch)
            || pr["head"]["sha"].as_str() != Some(&self.candidate)
            || pr["head"]["repo"]["full_name"].as_str() != Some(&self.repository)
            || pr["base"]["ref"].as_str() != Some(&self.base)
            || pr["base"]["repo"]["full_name"].as_str() != Some(&self.repository)
            || pr["html_url"].as_str() != Some(&expected_url)
            || !pr["body"].as_str().unwrap_or("").contains(&marker)
        {
            bail!("external_pull_request_identity_changed");
        }
        let state = if !pr["merged_at"].is_null() || pr["merged"].as_bool() == Some(true) {
            "merged"
        } else if pr["state"].as_str() == Some("closed") {
            "closed"
        } else if pr["state"].as_str() == Some("open") {
            "open"
        } else {
            bail!("invalid_pull_request_state");
        };
        Ok(JobPublication {
            repository: self.ctx.task.repository.clone(),
            number,
            url: expected_url,
            branch: self.branch.clone(),
            base_sha: self.ctx.task.base_sha.clone(),
            head_sha: self.candidate.clone(),
            draft: pr["draft"]
                .as_bool()
                .context("pull_request_draft_state_missing")?,
            state: state.into(),
            observed_at: Utc::now().to_rfc3339(),
        })
    }
}

impl PublicationRemote for GithubPublisher<'_> {
    async fn find(&self) -> Result<Option<JobPublication>> {
        let owner = self
            .repository
            .split('/')
            .next()
            .context("invalid_repository")?;
        let (status, value) = self
            .request(
                Method::GET,
                &format!("/pulls?state=all&head={owner}:{}&per_page=100", self.branch),
                None,
            )
            .await?;
        if status != StatusCode::OK {
            bail!("github_pull_request_lookup_failed: {}", status.as_u16());
        }
        let prs = value.as_array().context("invalid_pull_request_list")?;
        if prs.len() > 1 {
            bail!("multiple_job_pull_requests");
        }
        prs.first().map(|pr| self.parse_pr(pr)).transpose()
    }

    async fn branch_head(&self) -> Result<Option<String>> {
        let (status, value) = self
            .request(
                Method::GET,
                &format!("/git/ref/heads/{}", self.branch),
                None,
            )
            .await?;
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status != StatusCode::OK
            || value["ref"].as_str() != Some(&format!("refs/heads/{}", self.branch))
            || value["object"]["type"].as_str() != Some("commit")
        {
            bail!("github_branch_lookup_failed");
        }
        let sha = value["object"]["sha"]
            .as_str()
            .context("github_branch_head_missing")?;
        if !valid_sha(sha) {
            bail!("github_branch_head_invalid");
        }
        Ok(Some(sha.into()))
    }

    async fn push(&self, fence: &DeliveryFence) -> Result<()> {
        fence.check()?;
        let reference = format!("refs/heads/{}", self.branch);
        let mut command = git_command(self.ctx, Some(&self.ctx.repo()));
        // Credentials exist only in this trusted Git child's environment. The
        // immutable candidate Git directory contains no worker config/hooks.
        // An EMPTY expected ref permits atomic creation only; even a concurrent
        // external branch creation cannot be force-overwritten.
        let authorization = base64::engine::general_purpose::STANDARD
            .encode(format!("x-access-token:{}", self.credential));
        let header = format!("Authorization: Basic {authorization}");
        let overrides = [
            ("protocol.https.allow", "always"),
            ("http.https://github.com/.extraheader", header.as_str()),
            ("http.followRedirects", "false"),
            ("http.sslVerify", "true"),
        ];
        command.env(
            "GIT_CONFIG_COUNT",
            (SAFE_GIT_CONFIG.len() + overrides.len()).to_string(),
        );
        for (index, (key, value)) in overrides.iter().enumerate() {
            let index = SAFE_GIT_CONFIG.len() + index;
            command
                .env(format!("GIT_CONFIG_KEY_{index}"), key)
                .env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
        command.args([
            "push",
            "--no-verify",
            "--porcelain",
            &format!("--force-with-lease={reference}:"),
            "--",
            &format!("https://github.com/{}.git", self.repository),
            &format!("{}:{reference}", self.candidate),
        ]);
        run_git(command, Vec::new(), 16 * 1024, fence)
            .await
            .context("expected_ref_protected_push_failed")?;
        Ok(())
    }

    async fn create(&self, fence: &DeliveryFence) -> Result<CreateOutcome> {
        let body = json!({
            "title": format!("APAS job {}", self.ctx.identity.job_id),
            "body": format!("<!-- apas-job:{} candidate:{} -->\n\nAPAS autonomous job `{}`. Human review is required; configured checks are evidence, not a proof of correctness.\n\nAdmitted base: `{}` (`{}`).\nVerified candidate: `{}`.\nVerification profile digest: `{}`.\nRetained task, per-file diffs and check logs: APAS `/code/jobs/{}/review`.\n\nNo merge or approval has been requested.", self.ctx.identity.job_id, self.candidate, self.ctx.identity.job_id, self.base, self.ctx.task.base_sha, self.candidate, self.ctx.task.verification_profile_digest, self.ctx.identity.job_id),
            "head": self.branch, "base": self.base, "draft": true, "maintainer_can_modify": false,
        });
        fence.check()?;
        let response = self.request(Method::POST, "/pulls", Some(&body)).await;
        match response {
            Ok((StatusCode::CREATED, value)) => {
                let publication = self.parse_pr(&value)?;
                if !publication.draft {
                    bail!("github_did_not_create_draft");
                }
                Ok(CreateOutcome::Created(publication))
            }
            Ok((status, _)) if status.is_client_error() => Ok(CreateOutcome::Rejected(format!(
                "github_http_{}",
                status.as_u16()
            ))),
            _ => Ok(CreateOutcome::Uncertain),
        }
    }
}

fn canonical_repository(input: &str) -> Result<String> {
    let repo = input
        .strip_prefix("https://github.com/")
        .unwrap_or(input)
        .strip_suffix(".git")
        .unwrap_or(input.strip_prefix("https://github.com/").unwrap_or(input));
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|s| {
            s.is_empty()
                || *s == "."
                || *s == ".."
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        })
    {
        bail!("publisher_repository_must_be_explicit_github_owner_repo");
    }
    Ok(repo.into())
}

fn canonical_base(input: &str) -> Result<String> {
    let base = input.strip_prefix("refs/heads/").unwrap_or(input);
    if base.is_empty()
        || base.starts_with('/')
        || base.ends_with('/')
        || base.ends_with('.')
        || base.ends_with(".lock")
        || base.contains("..")
        || base.contains("//")
        || !base
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/' | b'.'))
    {
        bail!("publisher_base_ref_invalid");
    }
    Ok(base.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    struct FakeRemote {
        publication: Mutex<Option<JobPublication>>,
        branch: Mutex<Option<String>>,
        template: JobPublication,
        creates: std::sync::atomic::AtomicUsize,
        pushes: std::sync::atomic::AtomicUsize,
        lose_reply: bool,
        cancellation: Option<Arc<AtomicBool>>,
    }
    impl PublicationRemote for FakeRemote {
        async fn find(&self) -> Result<Option<JobPublication>> {
            Ok(self.publication.lock().clone())
        }
        async fn branch_head(&self) -> Result<Option<String>> {
            Ok(self.branch.lock().clone())
        }
        async fn push(&self, fence: &DeliveryFence) -> Result<()> {
            fence.check()?;
            self.pushes.fetch_add(1, Ordering::SeqCst);
            *self.branch.lock() = Some(self.template.head_sha.clone());
            Ok(())
        }
        async fn create(&self, fence: &DeliveryFence) -> Result<CreateOutcome> {
            fence.check()?;
            self.creates.fetch_add(1, Ordering::SeqCst);
            *self.publication.lock() = Some(self.template.clone());
            if let Some(cancel) = &self.cancellation {
                cancel.store(true, Ordering::Release);
            }
            if self.lose_reply {
                Ok(CreateOutcome::Uncertain)
            } else {
                Ok(CreateOutcome::Created(self.template.clone()))
            }
        }
    }

    fn fixture(path: &Path) -> (DeliveryContext, DeliveryRecord, FakeRemote, DeliveryFence) {
        let job_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let ctx = DeliveryContext {
            identity: JobRunIdentity {
                project_id: "project".into(),
                job_id,
                run_id,
                cluster_owner_user_id: Uuid::new_v4(),
                machine_id: Uuid::new_v4(),
                epoch: 1,
            },
            task: JobTaskSpec {
                worker: None,
                instruction: "task".into(),
                acceptance_criteria: vec![],
                source_url: None,
                repository: "owner/repo".into(),
                base_ref: "main".into(),
                base_sha: "a".repeat(40),
                worker_profile: "worker".into(),
                worker_profile_digest: "w".into(),
                verification_profile: "check".into(),
                verification_profile_digest: "v".into(),
                model_profile: "model".into(),
            },
            limits: JobLimits::default(),
            run_dir: path.join("run"),
            workspace_root: path.join("workspace"),
            trusted_source_repo: path.join("source"),
            worker_output: path.join("output"),
        };
        private_dir(&ctx.root()).unwrap();
        let record = ctx.load_or_create().unwrap();
        let template = JobPublication {
            repository: "owner/repo".into(),
            number: 42,
            url: "https://github.com/owner/repo/pull/42".into(),
            branch: format!("apas/job-{job_id}"),
            base_sha: ctx.task.base_sha.clone(),
            head_sha: "b".repeat(40),
            draft: true,
            state: "open".into(),
            observed_at: Utc::now().to_rfc3339(),
        };
        let remote = FakeRemote {
            publication: Mutex::new(None),
            branch: Mutex::new(None),
            template,
            creates: 0.into(),
            pushes: 0.into(),
            lose_reply: true,
            cancellation: None,
        };
        let fence = DeliveryFence::new(
            Arc::new(AtomicBool::new(false)),
            Utc::now() + ChronoDuration::minutes(1),
        );
        (ctx, record, remote, fence)
    }

    #[tokio::test]
    async fn lost_creation_reply_reconciles_one_pr_without_a_second_push() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, mut record, remote, fence) = fixture(tmp.path());
        let first = publish_with(
            &ctx,
            &mut record,
            &remote,
            &remote.template.head_sha,
            &fence,
        )
        .await
        .unwrap();
        let mut restored = ctx.load().unwrap();
        let second = publish_with(
            &ctx,
            &mut restored,
            &remote,
            &remote.template.head_sha,
            &fence,
        )
        .await
        .unwrap();
        assert_eq!(first.number, second.number);
        assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
        assert_eq!(remote.pushes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn externally_changed_branch_is_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, mut record, remote, fence) = fixture(tmp.path());
        *remote.branch.lock() = Some("c".repeat(40));
        assert!(publish_with(
            &ctx,
            &mut record,
            &remote,
            &remote.template.head_sha,
            &fence
        )
        .await
        .is_err());
        assert_eq!(remote.pushes.load(Ordering::SeqCst), 0);
        assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancellation_during_creation_retains_the_external_partial_result() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, mut record, mut remote, fence) = fixture(tmp.path());
        remote.cancellation = Some(fence.cancelled.clone());
        let publication = publish_with(
            &ctx,
            &mut record,
            &remote,
            &remote.template.head_sha,
            &fence,
        )
        .await
        .unwrap();
        assert!(fence.check().is_err());
        assert_eq!(publication.number, 42);
        assert_eq!(ctx.load().unwrap().review.publication.unwrap().number, 42);
        assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancellation_before_push_and_unresolved_post_never_start_new_publication() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, mut record, remote, fence) = fixture(tmp.path());
        fence.cancelled.store(true, Ordering::Release);
        assert!(publish_with(
            &ctx,
            &mut record,
            &remote,
            &remote.template.head_sha,
            &fence
        )
        .await
        .is_err());
        fence.cancelled.store(false, Ordering::Release);
        record.publication.as_mut().unwrap().phase = "create_in_flight".into();
        assert!(publish_with(
            &ctx,
            &mut record,
            &remote,
            &remote.template.head_sha,
            &fence
        )
        .await
        .is_err());
        assert_eq!(remote.pushes.load(Ordering::SeqCst), 0);
        assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn retained_artifacts_expose_truncation_paging_and_expiry() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut ctx, mut record, _, _) = fixture(tmp.path());
        ctx.limits.output_bytes = 5;
        let artifact = ctx
            .retain(
                &mut record,
                JobArtifactKind::CheckLog,
                b"123456789",
                "text/plain",
                false,
            )
            .unwrap();
        ctx.save(&record).unwrap();
        assert!(artifact.truncated);
        assert_eq!(artifact.size_bytes, 5);
        let JobWorkerReport::ArtifactChunk {
            offset,
            data_base64,
            final_chunk,
            ..
        } = read_artifact_chunk(&ctx, artifact.id, 0, 4).unwrap()
        else {
            panic!("expected first artifact chunk")
        };
        assert_eq!(offset, 0);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(data_base64)
                .unwrap(),
            b"1234"
        );
        assert!(!final_chunk);
        let JobWorkerReport::ArtifactChunk {
            offset,
            data_base64,
            final_chunk,
            ..
        } = read_artifact_chunk(&ctx, artifact.id, 4, 4).unwrap()
        else {
            panic!("expected final artifact chunk")
        };
        assert_eq!(offset, 4);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(data_base64)
                .unwrap(),
            b"5"
        );
        assert!(final_chunk);
        record.artifacts[0].expires_at =
            Some((Utc::now() - ChronoDuration::seconds(1)).to_rfc3339());
        ctx.save(&record).unwrap();
        assert!(read_artifact_chunk(&ctx, artifact.id, 0, 4).is_err());
        prune_expired_artifacts(&ctx, Utc::now()).unwrap();
        assert!(!ctx.artifact_path(artifact.id).exists());
        assert_eq!(artifacts(&ctx).unwrap()[0].id, artifact.id);
        assert!(read_artifact_chunk(&ctx, artifact.id, 0, 4).is_err());
        record.retain_until = (Utc::now() - ChronoDuration::seconds(1)).to_rfc3339();
        record.publication = Some(PublicationJournal {
            phase: "create_uncertain".into(),
            candidate_sha: "b".repeat(40),
            repository: ctx.task.repository.clone(),
            branch: format!("apas/job-{}", ctx.identity.job_id),
            error: None,
        });
        ctx.save(&record).unwrap();
        assert!(!workspace_release_eligible(&ctx, Utc::now()).unwrap());
        assert!(acquire_purge_guard(&ctx).is_err());
        record.publication.as_mut().unwrap().phase = "reconciled".into();
        ctx.save(&record).unwrap();
        assert!(workspace_release_eligible(&ctx, Utc::now()).unwrap());
        let guard = acquire_purge_guard(&ctx).unwrap();
        assert!(acquire_purge_guard(&ctx).is_err());
        drop(guard);
        assert!(acquire_purge_guard(&ctx).is_ok());
    }
}
