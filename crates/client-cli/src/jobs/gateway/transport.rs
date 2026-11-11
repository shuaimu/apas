use super::{meter, policy, Inner};
use anyhow::{ensure, Context, Result};
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full, Limited, StreamBody};
use hyper::{
    body::{Frame, Incoming},
    header::{HeaderValue, CONTENT_TYPE},
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use shared::JobModelApi;
use std::{
    convert::Infallible,
    io,
    net::SocketAddr,
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{
    net::{TcpListener, UnixListener, UnixStream},
    sync::{mpsc, Semaphore},
};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

type Body = UnsyncBoxBody<Bytes, io::Error>;

fn full(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into())
        .map_err(|never| match never {})
        .boxed_unsync()
}

struct DispatchGuard {
    inner: Arc<Inner>,
    handed_off: bool,
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        if self.handed_off {
            return;
        }
        // Losing the worker connection during dispatch must not turn an ambiguous charge into free usage.
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.stop.send_replace(true);
        let inner = self.inner.clone();
        tokio::spawn(async move {
            inner.unknown("abandoned_model_dispatch").await;
        });
    }
}

fn failure(status: StatusCode, code: &'static str) -> Response<Body> {
    let mut response = Response::new(full(format!(
        "{{\"error\":{{\"type\":\"apas_gateway\",\"message\":\"{code}\"}}}}"
    )));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

async fn stopped(inner: &Inner) {
    let mut stop = inner.stop.subscribe();
    if inner.cancelled.load(Ordering::Acquire) || *stop.borrow() {
        return;
    }
    let _ = stop.changed().await;
}

pub(super) async fn serve(listener: UnixListener, inner: Arc<Inner>) {
    loop {
        let accepted = tokio::select! {
            _ = stopped(&inner) => break,
            result = listener.accept() => result,
        };
        let Ok((socket, _)) = accepted else {
            break;
        };
        let Ok(permit) = inner.connections.clone().try_acquire_owned() else {
            continue;
        };
        // Socket access is necessary but not sufficient; every request also authenticates the scoped token.
        let current = inner.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let service_inner = current.clone();
            let service = service_fn(move |request| {
                let inner = service_inner.clone();
                async move { Ok::<_, Infallible>(handle(request, inner).await) }
            });
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .keep_alive(false)
                .max_headers(32)
                .max_buf_size(16 * 1024)
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(10));
            tokio::select! {
                _ = stopped(&current) => {},
                _ = builder.serve_connection(TokioIo::new(socket), service) => {},
            }
        });
    }
}

async fn body_bytes(
    request: Request<Incoming>,
    limit: usize,
) -> Result<(hyper::http::request::Parts, Bytes)> {
    let (parts, body) = request.into_parts();
    let body = tokio::time::timeout(Duration::from_secs(15), Limited::new(body, limit).collect())
        .await
        .context("request body timed out")?
        .map_err(|_| anyhow::anyhow!("request body exceeds limit or is malformed"))?
        .to_bytes();
    Ok((parts, body))
}

async fn handle(request: Request<Incoming>, inner: Arc<Inner>) -> Response<Body> {
    if inner.authority().await.is_err() {
        return failure(StatusCode::FORBIDDEN, "execution_authority_expired");
    }
    let is_model =
        request.uri().path() == "/v1/responses" || request.uri().path() == "/v1/messages";
    if policy::check_headers(request.headers(), &inner.token, is_model).is_err() {
        return failure(StatusCode::FORBIDDEN, "forbidden_request_headers");
    }
    if request.method() == Method::CONNECT
        || request.uri().scheme().is_some()
        || request.uri().authority().is_some()
    {
        return failure(
            StatusCode::FORBIDDEN,
            "tunnels_and_absolute_targets_forbidden",
        );
    }
    if inner.state.lock().await.gate(is_model).is_err() {
        return failure(
            StatusCode::TOO_MANY_REQUESTS,
            "gateway_allowance_unavailable",
        );
    }
    let (parts, body) =
        match body_bytes(request, if is_model { policy::REQUEST_BYTES } else { 0 }).await {
            Ok(value) => value,
            Err(_) => return failure(StatusCode::BAD_REQUEST, "invalid_request_body"),
        };
    if is_model {
        model(parts, body, inner).await
    } else {
        dependency(parts, inner).await
    }
}

async fn dependency(parts: hyper::http::request::Parts, inner: Arc<Inner>) -> Response<Body> {
    if parts.method != Method::GET && parts.method != Method::HEAD {
        return failure(
            StatusCode::METHOD_NOT_ALLOWED,
            "dependency_method_forbidden",
        );
    }
    let url = match policy::dependency_url(&parts.uri, &inner.hosts) {
        Ok(url) => url,
        Err(_) => return failure(StatusCode::FORBIDDEN, "dependency_destination_forbidden"),
    };
    let Ok(permit) = inner.dependencies.clone().try_acquire_owned() else {
        return failure(
            StatusCode::TOO_MANY_REQUESTS,
            "dependency_concurrency_exhausted",
        );
    };
    let client = match tokio::select! { _ = stopped(&inner) => return failure(StatusCode::FORBIDDEN, "gateway_stopped"), result = policy::pinned_client(&url) => result }
    {
        Ok(client) => client,
        Err(_) => return failure(StatusCode::BAD_GATEWAY, "dependency_resolution_denied"),
    };
    if inner.authority().await.is_err() || inner.state.lock().await.gate(false).is_err() {
        return failure(StatusCode::FORBIDDEN, "gateway_allowance_unavailable");
    }
    let response = tokio::select! {
        _ = stopped(&inner) => return failure(StatusCode::FORBIDDEN, "gateway_stopped"),
        result = client.request(parts.method, url).header("accept-encoding", "identity").send() => result,
    };
    let response = match response {
        Ok(response) if !response.status().is_redirection() => response,
        _ => {
            return failure(
                StatusCode::BAD_GATEWAY,
                "dependency_request_or_redirect_denied",
            )
        }
    };
    let status = response.status();
    let content_type = response.headers().get(CONTENT_TYPE).cloned();
    let (sender, receiver) = mpsc::channel::<std::result::Result<Frame<Bytes>, io::Error>>(4);
    tokio::spawn(async move {
        let _permit = permit;
        let mut stream = response.bytes_stream();
        let mut received = 0u64;
        let result = async {
            loop {
                let next = tokio::select! { _ = stopped(&inner) => anyhow::bail!("gateway stopped"), chunk = stream.next() => chunk };
                let Some(chunk) = next else { break; };
                let chunk = chunk.context("dependency response failed")?;
                received = received.checked_add(chunk.len() as u64).context("dependency size overflow")?;
                ensure!(received <= 256 * 1024 * 1024, "dependency response too large");
                inner.output(chunk.len()).await?;
                tokio::time::timeout(Duration::from_secs(15), sender.send(Ok(Frame::data(chunk))))
                    .await.context("dependency consumer stalled")?.context("dependency consumer closed")?;
            }
            Ok::<(), anyhow::Error>(())
        }.await;
        if result.is_err() {
            let _ = sender.try_send(Err(io::Error::other("dependency stream stopped")));
        }
    });
    let mut result = Response::new(StreamBody::new(ReceiverStream::new(receiver)).boxed_unsync());
    *result.status_mut() = status;
    if let Some(content_type) = content_type {
        result.headers_mut().insert(CONTENT_TYPE, content_type);
    }
    result
}

async fn model(
    parts: hyper::http::request::Parts,
    body: Bytes,
    inner: Arc<Inner>,
) -> Response<Body> {
    let Some(profile) = inner.model.as_ref() else {
        return failure(StatusCode::FORBIDDEN, "model_route_disabled");
    };
    if parts
        .headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.split(';').next().unwrap_or("").trim() != "application/json")
    {
        return failure(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "model_request_requires_json",
        );
    }
    let output_limit = inner.state.lock().await.journal.limits.model_output_tokens;
    let (body, streaming, requested_output_tokens) =
        match policy::model_request(&parts.method, &parts.uri, &body, profile, output_limit) {
            Ok(value) => value,
            Err(_) => return failure(StatusCode::BAD_REQUEST, "model_request_not_approved"),
        };
    let url = match policy::model_url(profile) {
        Ok(url) => url,
        Err(_) => return failure(StatusCode::FORBIDDEN, "model_profile_invalid"),
    };
    let client = match tokio::select! { _ = stopped(&inner) => return failure(StatusCode::FORBIDDEN, "gateway_stopped"), result = policy::pinned_client(&url) => result }
    {
        Ok(client) => client,
        Err(_) => return failure(StatusCode::BAD_GATEWAY, "model_resolution_denied"),
    };
    let api = profile.api;
    let Some(credential) = inner.credential.as_ref() else {
        return failure(StatusCode::FORBIDDEN, "model_credentials_unavailable");
    };
    let mut request = client
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .header("accept-encoding", "identity")
        .header(
            "accept",
            if streaming {
                "text/event-stream"
            } else {
                "application/json"
            },
        )
        .body(body);
    request = match api {
        JobModelApi::OpenaiResponses => request.bearer_auth(credential),
        JobModelApi::AnthropicMessages => request
            .header("x-api-key", credential)
            .header("anthropic-version", "2023-06-01"),
    };
    if api == JobModelApi::AnthropicMessages {
        if let Some(beta) = parts.headers.get("anthropic-beta") {
            request = request.header("anthropic-beta", beta.clone());
        }
    }
    let request_id = {
        let mut state = inner.state.lock().await;
        if state.gate(true).is_err() {
            return failure(StatusCode::TOO_MANY_REQUESTS, "model_allowance_exhausted");
        }
        let id = match state.journal.begin(requested_output_tokens) {
            Ok(id) => id,
            Err(_) => return failure(StatusCode::TOO_MANY_REQUESTS, "model_allowance_exhausted"),
        };
        if inner.persist(&mut state).await.is_err() {
            return failure(StatusCode::SERVICE_UNAVAILABLE, "usage_journal_unavailable");
        }
        id
    };
    let mut dispatch_guard = DispatchGuard {
        inner: inner.clone(),
        handed_off: false,
    };
    // Dispatch intent is durable before any network write. Any ambiguous outcome is metering uncertainty.
    let response = tokio::select! {
        biased;
        _ = stopped(&inner) => {
            inner.unknown("cancelled_model_dispatch").await;
            return failure(StatusCode::FORBIDDEN, "gateway_stopped");
        },
        response = request.send() => response,
    };
    let response = match response {
        Ok(response) if response.status().is_success() => response,
        _ => {
            inner
                .unknown("provider_error_or_unknown_request_outcome")
                .await;
            return failure(
                StatusCode::BAD_GATEWAY,
                "model_provider_failed_metering_unknown",
            );
        }
    };
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if content_type
        != if streaming {
            "text/event-stream"
        } else {
            "application/json"
        }
    {
        inner.unknown("unsupported_provider_response_type").await;
        return failure(StatusCode::BAD_GATEWAY, "model_provider_response_invalid");
    }
    let (sender, receiver) = mpsc::channel::<std::result::Result<Frame<Bytes>, io::Error>>(4);
    dispatch_guard.handed_off = true;
    tokio::spawn(async move {
        if forward_model(response, api, streaming, request_id, sender.clone(), &inner)
            .await
            .is_err()
        {
            inner.unknown("incomplete_or_unpriced_provider_usage").await;
            let _ = sender.try_send(Err(io::Error::other("model stream metering failed")));
        }
    });
    let mut response = Response::new(StreamBody::new(ReceiverStream::new(receiver)).boxed_unsync());
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static(if streaming {
            "text/event-stream"
        } else {
            "application/json"
        }),
    );
    response
}

async fn observe(inner: &Inner, id: Uuid, tokens: meter::Tokens, terminal: bool) -> Result<()> {
    let mut state = inner.state.lock().await;
    state.journal.observe(id, tokens, terminal)?;
    inner.persist(&mut state).await
}

async fn forward_model(
    response: reqwest::Response,
    api: JobModelApi,
    streaming: bool,
    id: Uuid,
    sender: mpsc::Sender<std::result::Result<Frame<Bytes>, io::Error>>,
    inner: &Inner,
) -> Result<()> {
    let mut stream = response.bytes_stream();
    let mut meter = meter::StreamMeter::new(api);
    let mut json = BytesMut::new();
    let mut received = 0u64;
    let mut consumer = true;
    loop {
        let chunk = tokio::select! { _ = stopped(inner) => anyhow::bail!("gateway stopped"), chunk = stream.next() => chunk };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.context("provider body transport failed")?;
        received = received
            .checked_add(chunk.len() as u64)
            .context("provider response overflow")?;
        ensure!(
            received <= policy::RESPONSE_BYTES,
            "provider response exceeds limit"
        );
        inner.output(chunk.len()).await?;
        if streaming {
            for (tokens, terminal) in meter.feed(&chunk)? {
                observe(inner, id, tokens, terminal).await?;
            }
            // A disconnected/stalled worker cannot suppress accounting: finish draining bounded provider usage.
            if consumer {
                consumer = matches!(
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        sender.send(Ok(Frame::data(chunk)))
                    )
                    .await,
                    Ok(Ok(()))
                );
            }
        } else {
            json.extend_from_slice(&chunk);
        }
    }
    if streaming {
        meter.finish()?;
    } else {
        observe(inner, id, meter::response_usage(api, &json)?, true).await?;
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            sender.send(Ok(Frame::data(json.freeze()))),
        )
        .await;
    }
    Ok(())
}

/// Runs inside the worker's network=none namespace. This listener only reaches one mounted UDS.
/// Neither the scoped token nor upstream credentials appear in command-line arguments.
pub async fn run_bridge(socket: PathBuf, token_file: PathBuf, listen: SocketAddr) -> Result<()> {
    ensure!(
        listen.ip().is_loopback() && listen.port() != 0,
        "bridge must bind a fixed loopback endpoint"
    );
    let listener = TcpListener::bind(listen).await?;
    serve_bridge(socket, token_file, listener).await
}

pub(super) async fn serve_bridge(
    socket: PathBuf,
    token_file: PathBuf,
    listener: TcpListener,
) -> Result<()> {
    let token = String::from_utf8(meter::read_private(&token_file, 128)?)?;
    ensure!(
        token.len() == 64 && token.bytes().all(|v| v.is_ascii_hexdigit()),
        "invalid bridge token"
    );
    let token = HeaderValue::from_str(&token)?;
    let slots = Arc::new(Semaphore::new(32));
    loop {
        let (connection, peer) = listener.accept().await?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let socket = socket.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let service = service_fn(move |request| {
                let socket = socket.clone();
                let token = token.clone();
                async move {
                    Ok::<_, Infallible>(match bridge_request(request, socket, token).await {
                        Ok(response) => response,
                        Err(_) => failure(StatusCode::BAD_GATEWAY, "scoped_gateway_unavailable"),
                    })
                }
            });
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .keep_alive(false)
                .max_headers(32)
                .max_buf_size(16 * 1024)
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(10));
            let _ = tokio::time::timeout(
                Duration::from_secs(650),
                builder.serve_connection(TokioIo::new(connection), service),
            )
            .await;
        });
    }
}

async fn bridge_request(
    request: Request<Incoming>,
    socket: PathBuf,
    token: HeaderValue,
) -> Result<Response<Body>> {
    ensure!(
        request.method() != Method::CONNECT
            && request.uri().scheme().is_none()
            && request.uri().authority().is_none(),
        "bridge tunneling is forbidden"
    );
    ensure!(
        !request.headers().contains_key(policy::TOKEN_HEADER),
        "worker cannot override bridge authentication"
    );
    let (mut parts, body) = body_bytes(request, policy::REQUEST_BYTES).await?;
    if matches!(parts.uri.path(), "/v1/responses" | "/v1/messages") {
        // The SDK's deliberately non-secret key is only a loopback bootstrap marker.
        // Remove exactly that marker; preserve every unexpected credential for host-side rejection.
        for (name, marker) in [
            ("authorization", "Bearer apas-run-scoped-bridge"),
            ("x-api-key", "apas-run-scoped-bridge"),
        ] {
            if parts
                .headers
                .get_all(name)
                .iter()
                .all(|value| value == marker)
            {
                parts.headers.remove(name);
            }
        }
    }
    parts.headers.insert(policy::TOKEN_HEADER, token);
    let stream =
        tokio::time::timeout(Duration::from_secs(5), UnixStream::connect(socket)).await??;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = sender
        .send_request(Request::from_parts(parts, Full::new(body)))
        .await?;
    Ok(response.map(|body| body.map_err(io::Error::other).boxed_unsync()))
}
