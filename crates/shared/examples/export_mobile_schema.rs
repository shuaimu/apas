use schemars::{schema_for, JsonSchema};
use shared::jobs::{
    EffectiveJobPolicy, JobAdmissionRequest, JobAnswerRequest, JobApiError, JobArtifactContent,
    JobDetail, JobEventsResponse, JobListResponse, JobMutationReceipt, JobMutationRequest,
    JobPolicy, JobPolicyManagement, JobPolicyOverride, JobPreview, JobPreviewRequest, JobReview,
    JobTargetsResponse, MobileJobsBootstrap, ProjectJobListResponse,
};
use shared::{
    CodeEvent, MobileAuthResponse, MobileBootstrapResponse, MobileDeviceSession,
    MobileLoginRequest, MobileLogoutRequest, MobileNotificationPreferences, MobilePushTokenRequest,
    MobileRefreshRequest, MobileTaskLaunchRequest, MobileTaskLaunchResponse, ServerToWeb,
    WebToServer,
};
use std::path::{Path, PathBuf};

#[derive(JsonSchema)]
#[allow(dead_code)]
struct MobileProtocolContract {
    web_to_server: WebToServer,
    server_to_web: ServerToWeb,
    login_request: MobileLoginRequest,
    refresh_request: MobileRefreshRequest,
    logout_request: MobileLogoutRequest,
    auth_response: MobileAuthResponse,
    device_session: MobileDeviceSession,
    bootstrap_response: MobileBootstrapResponse,
    code_event: CodeEvent,
    task_launch_request: MobileTaskLaunchRequest,
    task_launch_response: MobileTaskLaunchResponse,
    push_token_request: MobilePushTokenRequest,
    notification_preferences: MobileNotificationPreferences,
    jobs: JobProtocolContract,
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct JobProtocolContract {
    preview_request: JobPreviewRequest,
    preview: JobPreview,
    admission_request: JobAdmissionRequest,
    mutation_request: JobMutationRequest,
    answer_request: JobAnswerRequest,
    targets: JobTargetsResponse,
    list: JobListResponse,
    project_list: ProjectJobListResponse,
    detail: JobDetail,
    events: JobEventsResponse,
    receipt: JobMutationReceipt,
    review: JobReview,
    artifact: JobArtifactContent,
    error: JobApiError,
    policy: EffectiveJobPolicy,
    policy_management: JobPolicyManagement,
    deployment_policy: JobPolicy,
    policy_override: JobPolicyOverride,
    bootstrap: MobileJobsBootstrap,
}

fn write_schema<T: JsonSchema + ?Sized>(output: &Path, name: &str) -> anyhow::Result<()> {
    let schema = schema_for!(T);
    let json = serde_json::to_string_pretty(&schema)?;
    std::fs::write(output.join(name), format!("{json}\n"))?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/protocol/schema")
        });
    std::fs::create_dir_all(&output)?;
    write_schema::<MobileProtocolContract>(&output, "mobile-protocol.schema.json")?;
    write_schema::<WebToServer>(&output, "web-to-server.schema.json")?;
    write_schema::<ServerToWeb>(&output, "server-to-web.schema.json")?;
    write_schema::<CodeEvent>(&output, "code-event.schema.json")?;
    write_schema::<JobProtocolContract>(&output, "jobs.schema.json")?;
    println!("exported mobile schemas to {}", output.display());
    Ok(())
}
