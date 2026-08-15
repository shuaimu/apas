use anyhow::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod db;
mod error;
mod mobile_metrics;
mod notifications;
mod project_lifecycle;
mod routes;
mod session;
mod state;
mod storage;
mod work_summary;

use state::AppState;

fn main() -> Result<()> {
    // Build a runtime without the signal driver to avoid socketpair restrictions in containers.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(async_main())
}

async fn async_main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "apas_server=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = config::Config::load()?;
    tracing::info!(
        "Starting APAS server on {}:{}",
        config.server.host,
        config.server.port
    );

    // Initialize database
    let db = db::Database::new(&config.database.path).await?;
    db.run_migrations().await?;
    routes::seed_system_admin(&db, &config.system_admin).await?;

    // Create app state
    let state = AppState::new(db, config.clone());
    project_lifecycle::recover_interrupted_deletions(&state).await?;
    if state.config.mobile.features.notifications {
        let transport = notifications::ExpoPushTransport::new(state.config.mobile.push.clone())?;
        tokio::spawn(notifications::run_worker(
            state.clone(),
            std::sync::Arc::new(transport),
        ));
    }

    // Spawn the message GC task. Runs once at boot to catch the backlog,
    // then every 24h. Pure delete — drops messages with created_at older
    // than the cutoff, no archive.
    let storage_for_gc = state.storage.clone();
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(60 * 60 * 24);
        let retention_days: i64 = 7;
        loop {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days);
            tracing::info!(
                cutoff = %cutoff,
                retention_days,
                "Running message GC sweep"
            );
            match storage_for_gc.gc_all_sessions_before(cutoff).await {
                Ok(stats) => tracing::info!(
                    sessions_scanned = stats.sessions_scanned,
                    sessions_modified = stats.sessions_modified,
                    messages_kept = stats.messages_kept,
                    messages_dropped = stats.messages_dropped,
                    bytes_freed = stats.bytes_freed,
                    "Message GC sweep complete"
                ),
                Err(e) => tracing::warn!("Message GC sweep failed: {}", e),
            }
            tokio::time::sleep(interval).await;
        }
    });
    let summary_timeouts = state.pane_work_summaries.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            summary_timeouts.sweep_timeouts().await;
        }
    });

    // Reconcile completed summary windows at boot and every configured
    // interval. Generation is capability-gated and never blocks appends.
    let summary_service = state.pane_work_summaries.clone();
    let summary_interval_minutes = state.config.summaries.reconcile_interval_minutes.max(1);
    tokio::spawn(async move {
        loop {
            summary_service.reconcile_all().await;
            tokio::time::sleep(std::time::Duration::from_secs(
                summary_interval_minutes * 60,
            ))
            .await;
        }
    });

    // Build router
    let app = routes::create_router(state);

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
