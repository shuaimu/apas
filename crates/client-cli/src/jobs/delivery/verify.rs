use super::*;
use crate::jobs::config::{AutonomousJobConfig, VerificationProfileConfig};
use crate::jobs::gateway::{DependencyGatewayConfig, Gateway};
use crate::jobs::sandbox::{profile_digest, Sandbox};
use shared::{JobCheckResult, JobCheckStatus, JobVerification};

pub async fn verify_candidate(
    ctx: &DeliveryContext,
    config: &AutonomousJobConfig,
    fence: &DeliveryFence,
) -> Result<JobReview> {
    let _lock = ctx.lock()?;
    let mut record = ctx.load()?;
    ensure_retained(&record)?;
    let result = verify(ctx, config, &mut record, fence).await;
    if let Err(error) = &result {
        record.review.reason = Some(format!("verification_blocked: {error}"));
        ctx.save(&record)?;
    }
    result?;
    Ok(record.review)
}

async fn verify(
    ctx: &DeliveryContext,
    config: &AutonomousJobConfig,
    record: &mut DeliveryRecord,
    fence: &DeliveryFence,
) -> Result<()> {
    fence.check()?;
    let manifest = record.candidate.clone().context("candidate_missing")?;
    candidate::validate_candidate(ctx, &manifest, fence).await?;
    let profile = selected_profile(ctx, config)?;
    if let Some(previous) = &record.review.verification {
        if previous.candidate_sha != manifest.candidate_sha
            || previous.profile_digest != ctx.task.verification_profile_digest
        {
            bail!("existing_verification_binding_conflict");
        }
        // A check whose host died is missing, not implicitly passing and not an
        // invitation to re-execute arbitrary repository code on reconnect.
        if previous.completed_at.is_empty() {
            record.review.reason = Some("verification_interrupted_required_checks_missing".into());
            ctx.save(record)?;
        }
        return Ok(());
    }
    let sandbox = Sandbox::new(config)?;
    let mut verification = JobVerification {
        candidate_sha: manifest.candidate_sha.clone(),
        base_sha: manifest.base_sha.clone(),
        profile_digest: profile_digest(profile)?,
        image_digest: profile.image.clone(),
        checks: profile
            .checks
            .iter()
            .map(|check| JobCheckResult {
                name: check.name.clone(),
                argv: check.argv.clone(),
                status: JobCheckStatus::Missing,
                exit_code: None,
                elapsed_seconds: 0,
                log_artifact_id: None,
            })
            .collect(),
        completed_at: String::new(),
    };
    record.review.verification = Some(verification.clone());
    ctx.save(record)?;
    for (index, check) in profile.checks.iter().enumerate() {
        if let Err(error) = fence.check() {
            verification.checks[index].status = JobCheckStatus::Cancelled;
            record.review.reason = Some(error.to_string());
            break;
        }
        let checkout = ctx.data_root().join(format!("verifier-{}", Uuid::new_v4()));
        candidate::materialize(ctx, &manifest, &checkout)?;
        let mut limits = ctx.limits.clone();
        limits.elapsed_seconds = limits.elapsed_seconds.min(fence.remaining_seconds()?);
        limits.output_bytes = limits
            .output_bytes
            .saturating_sub(record.artifacts.iter().map(|a| a.size_bytes).sum::<u64>())
            .clamp(1, 2 * 1024 * 1024);
        let started = std::time::Instant::now();
        let dependency = if profile.dependency_hosts.is_empty() {
            None
        } else {
            Some(
                Gateway::start_dependencies(DependencyGatewayConfig {
                    run_id: ctx.identity.run_id,
                    directory: ctx
                        .root()
                        .join(format!("verifier-egress-{}", Uuid::new_v4())),
                    dependency_hosts: profile.dependency_hosts.clone(),
                    limits: limits.clone(),
                    deadline: fence.deadline,
                    lease_expires_at: fence.deadline,
                })
                .await?,
            )
        };
        let outcome = match fence.remaining_seconds() {
            Ok(remaining) => {
                limits.elapsed_seconds = limits.elapsed_seconds.min(remaining);
                sandbox
                    .run_verifier(
                        ctx.identity.run_id,
                        profile,
                        &limits,
                        &checkout,
                        check,
                        dependency.as_ref().map(|g| g.bridge_dir()),
                        fence.cancelled.clone(),
                    )
                    .await
            }
            Err(error) => Err(error),
        };
        let shutdown = if let Some(mut gateway) = dependency {
            gateway.shutdown().await
        } else {
            Ok(())
        };
        let elapsed_seconds = started.elapsed().as_secs();
        let (status, code, log, truncated) = match outcome {
            Ok(outcome) => {
                let status = if outcome.cancelled {
                    JobCheckStatus::Cancelled
                } else if outcome.timed_out {
                    JobCheckStatus::TimedOut
                } else if outcome.exit_code == Some(0) {
                    JobCheckStatus::Passed
                } else {
                    JobCheckStatus::Failed
                };
                let mut log = format!("candidate={}\nbase={}\nprofile={}\nimage={}\ncheck={}\ncontainer={}\n--- stdout ---\n",
                    manifest.candidate_sha, manifest.base_sha, verification.profile_digest, profile.image, check.name, outcome.container_id).into_bytes();
                log.extend_from_slice(&outcome.stdout);
                log.extend_from_slice(b"\n--- stderr ---\n");
                log.extend_from_slice(&outcome.stderr);
                (status, outcome.exit_code, log, outcome.truncated)
            }
            Err(error) => (
                JobCheckStatus::Missing,
                None,
                format!("required_check_unavailable: {error}\n").into_bytes(),
                false,
            ),
        };
        let artifact = ctx.retain(
            record,
            JobArtifactKind::CheckLog,
            &log,
            "text/plain; charset=utf-8",
            truncated,
        )?;
        verification.checks[index] = JobCheckResult {
            name: check.name.clone(),
            argv: check.argv.clone(),
            status,
            exit_code: code,
            elapsed_seconds,
            log_artifact_id: Some(artifact.id),
        };
        record.review.verification = Some(verification.clone());
        ctx.save(record)?;
        // Only the verifier's private checkout is removed, and only after the
        // sandbox API positively establishes that its full stop unit exited.
        // An error may mean stop uncertainty; retain that checkout in this case.
        if !matches!(status, JobCheckStatus::Missing) {
            fs::remove_dir_all(&checkout)?;
        }
        shutdown?;
        if matches!(status, JobCheckStatus::Cancelled) {
            break;
        }
    }
    verification.completed_at = Utc::now().to_rfc3339();
    let passed = verification_passes(
        &verification,
        profile,
        &manifest.candidate_sha,
        &ctx.task.base_sha,
        &ctx.task.verification_profile_digest,
    );
    record.review.reason = if passed {
        None
    } else {
        Some("required_verification_checks_not_passed".into())
    };
    let evidence = serde_json::to_vec(&verification)?;
    ctx.retain(
        record,
        JobArtifactKind::Verification,
        &evidence,
        "application/json",
        false,
    )?;
    record.review.verification = Some(verification);
    ctx.save(record)?;
    Ok(())
}

pub(super) fn selected_profile<'a>(
    ctx: &DeliveryContext,
    config: &'a AutonomousJobConfig,
) -> Result<&'a VerificationProfileConfig> {
    if !config.enabled {
        bail!("autonomous_jobs_disabled");
    }
    let worker = config
        .profiles
        .iter()
        .find(|p| p.key == ctx.task.worker_profile)
        .context("worker_profile_unavailable")?;
    if crate::jobs::sandbox::worker_profile_digest(config, worker)?
        != ctx.task.worker_profile_digest
    {
        bail!("admitted_worker_profile_changed");
    }
    if !worker.verifiers.contains(&ctx.task.verification_profile) {
        bail!("verification_profile_not_allowed");
    }
    let profile = config
        .verifiers
        .iter()
        .find(|p| p.key == ctx.task.verification_profile)
        .context("verification_profile_unavailable")?;
    if profile_digest(profile)? != ctx.task.verification_profile_digest {
        bail!("admitted_verification_profile_changed");
    }
    if profile.checks.is_empty()
        || profile
            .checks
            .iter()
            .any(|c| c.argv.is_empty() || c.argv[0].is_empty() || c.timeout_seconds == 0)
    {
        bail!("required_verification_checks_missing");
    }
    if profile.checks.len() > 128
        || serde_json::to_vec(&profile.checks)?.len() > shared::JOB_MAX_FRAME_BYTES / 8
    {
        bail!("verification_profile_manifest_exceeds_transport_bound");
    }
    crate::jobs::sandbox::validate_image_pin(&profile.image)?;
    Ok(profile)
}

pub(super) fn verification_passes(
    evidence: &JobVerification,
    profile: &VerificationProfileConfig,
    sha: &str,
    base: &str,
    digest: &str,
) -> bool {
    evidence.candidate_sha == sha
        && evidence.base_sha == base
        && evidence.profile_digest == digest
        && evidence.image_digest == profile.image
        && !evidence.completed_at.is_empty()
        && !profile.checks.is_empty()
        && evidence.checks.len() == profile.checks.len()
        && evidence
            .checks
            .iter()
            .zip(&profile.checks)
            .all(|(actual, expected)| {
                actual.name == expected.name
                    && actual.argv == expected.argv
                    && actual.status == JobCheckStatus::Passed
                    && actual.exit_code == Some(0)
                    && actual.log_artifact_id.is_some()
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::config::VerificationCheckConfig;

    #[test]
    fn only_complete_exact_head_checks_authorize_publication() {
        let profile = VerificationProfileConfig {
            key: "test".into(),
            label: "Tests".into(),
            image: "image@sha256:abc".into(),
            dependency_hosts: vec![],
            checks: vec![VerificationCheckConfig {
                name: "test".into(),
                argv: vec!["test".into()],
                timeout_seconds: 60,
            }],
        };
        let mut evidence = JobVerification {
            candidate_sha: "head".into(),
            base_sha: "base".into(),
            profile_digest: "digest".into(),
            image_digest: profile.image.clone(),
            completed_at: Utc::now().to_rfc3339(),
            checks: vec![JobCheckResult {
                name: "test".into(),
                argv: vec!["test".into()],
                status: JobCheckStatus::Passed,
                exit_code: Some(0),
                elapsed_seconds: 1,
                log_artifact_id: Some(Uuid::new_v4()),
            }],
        };
        assert!(verification_passes(
            &evidence, &profile, "head", "base", "digest"
        ));
        assert!(!verification_passes(
            &evidence,
            &profile,
            "different-head",
            "base",
            "digest"
        ));
        assert!(!verification_passes(
            &evidence,
            &profile,
            "head",
            "base",
            "changed-profile"
        ));
        for status in [
            JobCheckStatus::Failed,
            JobCheckStatus::TimedOut,
            JobCheckStatus::Missing,
            JobCheckStatus::Cancelled,
        ] {
            evidence.checks[0].status = status;
            assert!(!verification_passes(
                &evidence, &profile, "head", "base", "digest"
            ));
        }
        evidence.checks.clear();
        assert!(!verification_passes(
            &evidence, &profile, "head", "base", "digest"
        ));
    }
}
