use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{header::HeaderValue, Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::{net::IpAddr, os::unix::fs::PermissionsExt};

fn model(api: shared::JobModelApi) -> ModelProfileConfig {
    ModelProfileConfig {
        key: "qualified".into(),
        label: "Qualified".into(),
        model: "approved-model".into(),
        api,
        context_window: 128_000,
        reasoning: false,
        upstream_base_url: "https://api.example.com/v1".into(),
        credential_file: PathBuf::from("/not-read-by-parser"),
        input_micro_usd_per_million: 2_000_000,
        output_micro_usd_per_million: 8_000_000,
        cached_input_micro_usd_per_million: Some(500_000),
    }
}

fn journal(path: &Path, run: Uuid, profile: &ModelProfileConfig, limits: JobLimits) -> Journal {
    Journal::load_or_create(
        path,
        run,
        Some(Pricing::from(profile)),
        limits,
        JobUsage {
            spend_micro_usd: Some(0),
            ..JobUsage::default()
        },
        Utc::now() + chrono::Duration::minutes(30),
        Utc::now() + chrono::Duration::seconds(120),
    )
    .unwrap()
}

#[test]
fn model_adapters_accept_bounded_function_work_and_reject_unpriced_work() {
    for (api, route, cap, input) in [
        (
            shared::JobModelApi::OpenaiResponses,
            "/v1/responses",
            "max_output_tokens",
            "input",
        ),
        (
            shared::JobModelApi::AnthropicMessages,
            "/v1/messages",
            "max_tokens",
            "messages",
        ),
    ] {
        let profile = model(api);
        let mut value = json!({"model":"approved-model","stream":true});
        value[cap] = json!(512);
        value[input] = json!([{"role":"user","content":"implement the change"}]);
        let uri = route.parse().unwrap();
        assert!(policy::model_request(
            &Method::POST,
            &uri,
            &serde_json::to_vec(&value).unwrap(),
            &profile,
            512
        )
        .is_ok());
        value[cap] = json!(513);
        assert!(policy::model_request(
            &Method::POST,
            &uri,
            &serde_json::to_vec(&value).unwrap(),
            &profile,
            512
        )
        .is_err());
        value[cap] = json!(512);
        value["model"] = json!("other-model");
        assert!(policy::model_request(
            &Method::POST,
            &uri,
            &serde_json::to_vec(&value).unwrap(),
            &profile,
            512
        )
        .is_err());
        value["model"] = json!("approved-model");
        value["tools"] = json!([{"type":"web_search"}]);
        assert!(policy::model_request(
            &Method::POST,
            &uri,
            &serde_json::to_vec(&value).unwrap(),
            &profile,
            512
        )
        .is_err());
    }
}

#[test]
fn cache_creation_remote_content_and_ambient_headers_are_rejected() {
    let profile = model(shared::JobModelApi::AnthropicMessages);
    let value = json!({"model":"approved-model","max_tokens":8,"messages":[{"role":"user","content":[
        {"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}
    ]}]});
    assert!(policy::model_request(
        &Method::POST,
        &"/v1/messages".parse().unwrap(),
        &serde_json::to_vec(&value).unwrap(),
        &profile,
        8
    )
    .is_err());
    let mut headers = hyper::HeaderMap::new();
    headers.insert(policy::TOKEN_HEADER, HeaderValue::from_static("run-token"));
    assert!(policy::check_headers(&headers, "run-token", false).is_ok());
    headers.insert(
        "proxy-authorization",
        HeaderValue::from_static("Bearer secret"),
    );
    assert!(policy::check_headers(&headers, "run-token", false).is_err());
    headers.remove("proxy-authorization");
    headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer upstream-key"),
    );
    assert!(policy::check_headers(&headers, "run-token", true).is_err());
    headers.remove("authorization");
    headers.insert("upgrade", HeaderValue::from_static("websocket"));
    assert!(policy::check_headers(&headers, "run-token", true).is_err());
}

#[test]
fn dependency_allowlists_reject_private_literal_encoded_and_redirect_targets() {
    let hosts = policy::dependency_hosts(&["registry.npmjs.org".into()]).unwrap();
    let allowed = policy::dependency_url(
        &"/dependencies/registry.npmjs.org/pkg/-/pkg.tgz?x=1"
            .parse()
            .unwrap(),
        &hosts,
    )
    .unwrap();
    assert_eq!(
        allowed.as_str(),
        "https://registry.npmjs.org/pkg/-/pkg.tgz?x=1"
    );
    for target in [
        "/dependencies/127.0.0.1/control",
        "/dependencies/registry.npmjs.org.evil.example/a",
        "/dependencies/registry.npmjs.org%40evil.example/a",
        "https://registry.npmjs.org/a",
        "/v1/socket",
    ] {
        assert!(
            policy::dependency_url(&target.parse().unwrap(), &hosts).is_err(),
            "{target}"
        );
    }
    for address in [
        "127.0.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.2",
        "169.254.169.254",
        "100.100.100.200",
        "0.0.0.0",
        "198.18.1.1",
        "224.0.0.1",
        "::1",
        "::ffff:127.0.0.1",
        "fc00::1",
        "fe80::1",
        "2002:7f00:1::",
        "64:ff9b::a00:1",
    ] {
        assert!(
            !policy::public_ip(address.parse::<IpAddr>().unwrap()),
            "{address}"
        );
    }
    assert!(policy::public_ip("1.1.1.1".parse().unwrap()));
    assert!(policy::public_ip("2606:4700:4700::1111".parse().unwrap()));
    for host in [
        "*.example.com",
        "localhost",
        "host.internal",
        "127.0.0.1",
        "https://example.com",
        "example.com:443",
    ] {
        assert!(policy::dependency_hosts(&[host.to_owned()]).is_err());
    }
}

#[test]
fn responses_sse_handles_split_frames_and_requires_terminal_usage() {
    let wire = b"event: response.created\r\ndata: {\"type\":\"response.created\"}\r\n\r\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":120,\"output_tokens\":7,\"input_tokens_details\":{\"cached_tokens\":20}}}}\n\ndata: [DONE]\n\n";
    let mut parser = meter::StreamMeter::new(shared::JobModelApi::OpenaiResponses);
    let observations: Vec<_> = wire
        .chunks(3)
        .flat_map(|chunk| parser.feed(chunk).unwrap())
        .collect();
    parser.finish().unwrap();
    assert_eq!(
        observations,
        vec![(
            meter::Tokens {
                input: 120,
                output: 7,
                cached: 20
            },
            true
        )]
    );
    let mut missing = meter::StreamMeter::new(shared::JobModelApi::OpenaiResponses);
    missing
        .feed(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n")
        .unwrap();
    assert!(missing.finish().is_err());
    assert!(missing
        .feed(b"data: {\"type\":\"response.completed\",\"response\":{}}\n\n")
        .is_err());
}

#[test]
fn messages_sse_reports_input_cache_and_cumulative_output_not_delta_sum() {
    let mut parser = meter::StreamMeter::new(shared::JobModelApi::AnthropicMessages);
    let start = parser.feed(b"data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":30,\"cache_creation_input_tokens\":0,\"output_tokens\":1}}}\n\n").unwrap();
    assert_eq!(
        start[0].0,
        meter::Tokens {
            input: 40,
            output: 1,
            cached: 30
        }
    );
    parser
        .feed(b"data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":15}}\n\n")
        .unwrap();
    let terminal = parser
        .feed(b"data: {\"type\":\"message_stop\"}\n\n")
        .unwrap();
    assert_eq!(
        terminal[0],
        (
            meter::Tokens {
                input: 40,
                output: 15,
                cached: 30
            },
            true
        )
    );
    parser.finish().unwrap();
    let unpriced =
        br#"{"usage":{"input_tokens":10,"output_tokens":1,"cache_creation_input_tokens":4}}"#;
    assert!(meter::response_usage(shared::JobModelApi::AnthropicMessages, unpriced).is_err());
}

#[test]
fn aggregate_spend_is_atomic_cumulative_and_exposes_inflight_overrun() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("usage.json");
    let run = Uuid::new_v4();
    let profile = model(shared::JobModelApi::OpenaiResponses);
    let mut limits = JobLimits::default();
    limits.spend_micro_usd = 15;
    let mut state = journal(&path, run, &profile, limits.clone());
    let root = state.begin(8).unwrap();
    let child = state.begin(8).unwrap();
    assert!(state.begin(8).is_err());
    state
        .observe(
            root,
            meter::Tokens {
                input: 1,
                output: 2,
                cached: 0,
            },
            true,
        )
        .unwrap();
    assert_eq!(state.public_usage().spend_micro_usd, Some(18));
    assert!(state.begin(8).is_err());
    state
        .observe(
            child,
            meter::Tokens {
                input: 2,
                output: 3,
                cached: 0,
            },
            true,
        )
        .unwrap();
    state.persist(&path).unwrap();
    let restored = journal(&path, run, &profile, limits);
    assert_eq!(restored.public_usage().spend_micro_usd, Some(46));
    assert_eq!(restored.public_usage().input_tokens, 3);
    assert_eq!(restored.public_usage().output_tokens, 5);
    assert!(restored.gate(true).is_err());
}

#[test]
fn partial_usage_survives_restart_and_uncertain_dispatch_fences_replay() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("usage.json");
    let run = Uuid::new_v4();
    let profile = model(shared::JobModelApi::AnthropicMessages);
    let mut state = journal(&path, run, &profile, JobLimits::default());
    let request = state.begin(8).unwrap();
    let partial = meter::Tokens {
        input: 10,
        output: 1,
        cached: 0,
    };
    state.observe(request, partial.clone(), false).unwrap();
    state.observe(request, partial, false).unwrap();
    assert_eq!(state.public_usage().spend_micro_usd, Some(28));
    state.persist(&path).unwrap();
    let restored = journal(&path, run, &profile, JobLimits::default());
    assert!(restored.unknown);
    assert_eq!(restored.usage.spend_micro_usd, Some(28));
    assert_eq!(restored.public_usage().spend_micro_usd, None);
    assert!(restored.gate(true).is_err());
}

#[test]
fn changed_pricing_and_unpriced_cache_never_reset_consumption() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("usage.json");
    let run = Uuid::new_v4();
    let mut profile = model(shared::JobModelApi::OpenaiResponses);
    let mut state = journal(&path, run, &profile, JobLimits::default());
    profile.input_micro_usd_per_million += 1;
    assert!(Journal::load_or_create(
        &path,
        run,
        Some(Pricing::from(&profile)),
        JobLimits::default(),
        JobUsage {
            spend_micro_usd: Some(0),
            ..JobUsage::default()
        },
        state.deadline,
        state.lease_expires_at
    )
    .is_err());
    let request = state.begin(8).unwrap();
    assert!(state
        .observe(
            request,
            meter::Tokens {
                input: 1,
                output: 0,
                cached: 2
            },
            true
        )
        .is_err());
    let unknown_path = temp.path().join("unknown.json");
    assert!(Journal::load_or_create(
        &unknown_path,
        Uuid::new_v4(),
        Some(Pricing::from(&profile)),
        JobLimits::default(),
        JobUsage::default(),
        state.deadline,
        state.lease_expires_at
    )
    .is_err());
}

#[test]
fn credentials_require_private_regular_files_and_do_not_follow_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("key");
    meter::atomic_private(&path, b"dedicated-key").unwrap();
    assert_eq!(meter::read_private(&path, 32).unwrap(), b"dedicated-key");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(meter::read_private(&path, 32).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(meter::read_private(&alias, 32).is_err());
    assert!(meter::read_private(&path, 2).is_err());
}

async fn uds_request(
    socket: &Path,
    token: Option<&str>,
    method: Method,
    target: &str,
) -> StatusCode {
    let socket = tokio::net::UnixStream::connect(socket).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(socket))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = Request::builder()
        .method(method)
        .uri(target)
        .header("host", "localhost");
    if let Some(token) = token {
        request = request.header(policy::TOKEN_HEADER, token);
    }
    let response = sender
        .send_request(request.body(Full::new(Bytes::new())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let _ = response.into_body().collect().await;
    status
}

#[tokio::test]
async fn live_uds_requires_scoped_auth_and_dependency_gateway_cannot_call_models() {
    let temp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let mut gateway = Gateway::start_dependencies(DependencyGatewayConfig {
        run_id: Uuid::new_v4(),
        directory: temp.path().join("gateway"),
        dependency_hosts: vec!["registry.npmjs.org".into()],
        limits: JobLimits::default(),
        deadline: now + chrono::Duration::minutes(5),
        lease_expires_at: now + chrono::Duration::seconds(60),
    })
    .await
    .unwrap();
    let socket = gateway.bridge_dir().join("gateway.sock");
    let token =
        String::from_utf8(meter::read_private(&gateway.bridge_dir().join("token"), 128).unwrap())
            .unwrap();
    assert_eq!(
        uds_request(
            &socket,
            None,
            Method::GET,
            "/dependencies/evil.example/file"
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        uds_request(
            &socket,
            Some(&token),
            Method::GET,
            "/dependencies/evil.example/file"
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        uds_request(&socket, Some(&token), Method::POST, "/v1/responses").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(!gateway.bridge_dir().join("usage.json").exists());
    assert!(gateway
        .renew_lease(now + chrono::Duration::seconds(121))
        .await
        .is_err());
    gateway.shutdown().await.unwrap();
}

#[tokio::test]
async fn loopback_bridge_reaches_only_authenticated_narrow_gateway() {
    let temp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let mut gateway = Gateway::start_dependencies(DependencyGatewayConfig {
        run_id: Uuid::new_v4(),
        directory: temp.path().join("gateway"),
        dependency_hosts: vec![],
        limits: JobLimits::default(),
        deadline: now + chrono::Duration::minutes(5),
        lease_expires_at: now + chrono::Duration::seconds(60),
    })
    .await
    .unwrap();
    let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reservation.local_addr().unwrap();
    let bridge = tokio::spawn(transport::serve_bridge(
        gateway.bridge_dir().join("gateway.sock"),
        gateway.bridge_dir().join("token"),
        reservation,
    ));
    let socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(socket))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = sender
        .send_request(
            Request::builder()
                .uri("/dependencies/evil.example/control")
                .header("host", "localhost")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    bridge.abort();
    gateway.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_fences_expired_lease_without_controller_and_does_not_refresh_it_on_restart() {
    let temp = tempfile::tempdir().unwrap();
    let run = Uuid::new_v4();
    let directory = temp.path().join("gateway");
    let now = Utc::now();
    let mut gateway = Gateway::start_dependencies(DependencyGatewayConfig {
        run_id: run,
        directory: directory.clone(),
        dependency_hosts: vec![],
        limits: JobLimits::default(),
        deadline: now + chrono::Duration::minutes(5),
        lease_expires_at: now + chrono::Duration::milliseconds(150),
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(gateway.inner.authority().await.is_err());
    assert!(gateway
        .renew_lease(Utc::now() + chrono::Duration::seconds(60))
        .await
        .is_err());
    gateway.shutdown().await.unwrap();
    drop(gateway);
    assert!(Gateway::start_dependencies(DependencyGatewayConfig {
        run_id: run,
        directory,
        dependency_hosts: vec![],
        limits: JobLimits::default(),
        deadline: Utc::now() + chrono::Duration::minutes(5),
        lease_expires_at: Utc::now() + chrono::Duration::seconds(60)
    })
    .await
    .is_err());
}

#[test]
fn reconnect_never_restores_narrowed_limits_or_an_expired_lease() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("usage.json");
    let run = Uuid::new_v4();
    let profile = model(shared::JobModelApi::OpenaiResponses);
    let mut state = journal(&path, run, &profile, JobLimits::default());
    state.limits.model_concurrency = 1;
    state.limits.spend_micro_usd = 100;
    state.lease_expires_at = Utc::now() - chrono::Duration::seconds(1);
    state.persist(&path).unwrap();
    let restored = journal(&path, run, &profile, JobLimits::default());
    assert_eq!(restored.limits.model_concurrency, 1);
    assert_eq!(restored.limits.spend_micro_usd, 100);
    assert!(restored.gate(true).is_err());
}

async fn model_gateway(directory: &Path, api: shared::JobModelApi) -> Gateway {
    let mut profile = model(api);
    profile.credential_file = directory.join("upstream-key");
    meter::atomic_private(&profile.credential_file, b"private-upstream-test-key").unwrap();
    Gateway::start(GatewayConfig {
        run_id: Uuid::new_v4(),
        directory: directory.join("gateway"),
        model: profile,
        dependency_hosts: vec![],
        limits: JobLimits::default(),
        prior_usage: JobUsage {
            spend_micro_usd: Some(0),
            ..JobUsage::default()
        },
        deadline: Utc::now() + chrono::Duration::minutes(5),
        lease_expires_at: Utc::now() + chrono::Duration::seconds(60),
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn narrowing_during_preparation_rejects_prevalidated_output_before_dispatch() {
    for (api, route, cap_key, input_key) in [
        (
            shared::JobModelApi::OpenaiResponses,
            "/v1/responses",
            "max_output_tokens",
            "input",
        ),
        (
            shared::JobModelApi::AnthropicMessages,
            "/v1/messages",
            "max_tokens",
            "messages",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut gateway = model_gateway(temp.path(), api).await;
        let inner = gateway.inner.clone();
        let (prepared, preparation_complete) = tokio::sync::oneshot::channel();
        let (resume, preparation_resumed) = tokio::sync::oneshot::channel();
        let dispatched = Arc::new(AtomicBool::new(false));
        let dispatch_observer = dispatched.clone();
        let request = tokio::spawn(async move {
            let profile = inner.model.as_ref().unwrap();
            let mut body = json!({"model":"approved-model","stream":true});
            body[cap_key] = json!(512);
            body[input_key] = json!([{"role":"user","content":"bounded work"}]);
            let original_cap = inner.state.lock().await.journal.limits.model_output_tokens;
            let (_, _, requested_output) = policy::model_request(
                &Method::POST,
                &route.parse().unwrap(),
                &serde_json::to_vec(&body).unwrap(),
                profile,
                original_cap,
            )
            .unwrap();
            // Deterministically hold the asynchronous preparation window before final dispatch admission.
            prepared.send(()).unwrap();
            preparation_resumed.await.unwrap();
            let mut state = inner.state.lock().await;
            state.gate(true)?;
            state.journal.begin(requested_output)?;
            dispatch_observer.store(true, Ordering::Release);
            Ok::<(), anyhow::Error>(())
        });
        preparation_complete.await.unwrap();
        let mut narrowed = JobLimits::default();
        narrowed.model_output_tokens = 128;
        gateway.narrow_limits(&narrowed).await.unwrap();
        resume.send(()).unwrap();
        assert!(request.await.unwrap().is_err());
        assert!(!dispatched.load(Ordering::Acquire));
        assert!(gateway.inner.state.lock().await.journal.pending.is_empty());
        gateway.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn bridge_adapts_only_sdk_markers_and_preserves_strict_host_authentication() {
    for (api, route, auth_header, sdk_auth, cap_key, input_key) in [
        (
            shared::JobModelApi::OpenaiResponses,
            "/v1/responses",
            "authorization",
            "Bearer apas-run-scoped-bridge",
            "max_output_tokens",
            "input",
        ),
        (
            shared::JobModelApi::AnthropicMessages,
            "/v1/messages",
            "x-api-key",
            "apas-run-scoped-bridge",
            "max_tokens",
            "messages",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut gateway = model_gateway(temp.path(), api).await;
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = reservation.local_addr().unwrap();
        let bridge = tokio::spawn(transport::serve_bridge(
            gateway.bridge_dir().join("gateway.sock"),
            gateway.bridge_dir().join("token"),
            reservation,
        ));
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // The deliberately invalid model proves authentication reaches the adapter without a billable call.
        let mut body = json!({"model":"not-approved","stream":true});
        body[cap_key] = json!(8);
        body[input_key] = json!([{"role":"user","content":"no upstream dispatch"}]);
        let endpoint = format!("http://{address}{route}");
        let response = client
            .post(&endpoint)
            .header(auth_header, sdk_auth)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = client
            .post(&endpoint)
            .header(auth_header, "unexpected-credential")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = client
            .get(format!("http://{address}/dependencies/example.com/file"))
            .header(auth_header, sdk_auth)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let token = String::from_utf8(
            meter::read_private(&gateway.bridge_dir().join("token"), 128).unwrap(),
        )
        .unwrap();
        let socket = tokio::net::UnixStream::connect(gateway.bridge_dir().join("gateway.sock"))
            .await
            .unwrap();
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(socket))
            .await
            .unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let request = Request::builder()
            .method(Method::POST)
            .uri(route)
            .header("host", "localhost")
            .header(policy::TOKEN_HEADER, token)
            .header(auth_header, sdk_auth)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(serde_json::to_vec(&body).unwrap())))
            .unwrap();
        assert_eq!(
            sender.send_request(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        assert!(gateway.inner.state.lock().await.journal.pending.is_empty());
        bridge.abort();
        gateway.shutdown().await.unwrap();
    }
}
