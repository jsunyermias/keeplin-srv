use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use keeplin_srv::{config::Config, http::router, state::AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env();

    // Structured (JSON) logging in production, pretty logging in development.
    let env_filter = EnvFilter::from_default_env().add_directive("keeplin_srv=info".parse()?);
    if config.log_json {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }

    // Bounded pool: cap connections, fail fast instead of blocking forever when
    // the pool is exhausted, and reap idle/old connections so zombies do not
    // accumulate.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(Duration::from_secs(config.db_acquire_timeout_secs))
        .idle_timeout(Duration::from_secs(config.db_idle_timeout_secs))
        .max_lifetime(Duration::from_secs(config.db_max_lifetime_secs))
        .connect(&config.database_url)
        .await
        .context("connect to PostgreSQL")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("run database migrations")?;

    let state = Arc::new(AppState::new(config.clone(), pool));

    if config.retention_days > 0 || config.lines_gc_days > 0 || config.resource_purge_days > 0 {
        tokio::spawn(maintenance_loop(
            state.clone(),
            config.retention_days,
            config.lines_gc_days,
            config.resource_purge_days,
        ));
    }

    let grace = config.shutdown_grace_secs;
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port))
        .await
        .context("bind server address")?;

    tracing::info!(
        port = config.port,
        rate_limit_per_min = config.rate_limit_per_min,
        "Keeplin sync server listening"
    );
    // `ConnectInfo` is required so the rate limiter can key on the peer IP.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(grace))
    .await
    .context("run server")?;

    tracing::info!("shutdown complete");
    Ok(())
}

/// Resolve on `SIGTERM` (containers/systemd) or `Ctrl-C`, then arm a watchdog:
/// graceful shutdown drains in-flight REST requests, but long-lived WebSocket
/// connections would otherwise keep the process alive forever, so after
/// `grace` seconds the process force-exits.
async fn shutdown_signal(grace: u64) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!(grace_secs = grace, "shutdown signal received; draining");

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(grace)).await;
        tracing::warn!("grace period elapsed; forcing exit");
        std::process::exit(0);
    });
}

/// Hourly maintenance: prune relay journal rows already delivered to every connected
/// device (when `retention_days > 0`), compact old line tombstones (design §6.4, when
/// `lines_gc_days > 0`), and reclaim the payloads of long-deleted resources (when
/// `resource_purge_days > 0`).
async fn maintenance_loop(
    state: Arc<AppState>,
    retention_days: u64,
    lines_gc_days: u64,
    resource_purge_days: u64,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        interval.tick().await;
        if retention_days > 0 {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
            match state.store.prune_delivered_changes(cutoff).await {
                Ok(0) => {}
                Ok(rows) => tracing::info!(rows, "pruned delivered changes"),
                Err(e) => tracing::warn!(error = %e, "prune failed"),
            }
        }
        if lines_gc_days > 0 {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(lines_gc_days as i64);
            match state.store.gc_line_tombstones(cutoff).await {
                Ok(0) => {}
                Ok(rows) => tracing::info!(rows, "compacted line tombstones"),
                Err(e) => tracing::warn!(error = %e, "line GC failed"),
            }
        }
        if resource_purge_days > 0 {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(resource_purge_days as i64);
            match state.store.purge_deleted_resource_blobs(cutoff).await {
                Ok(0) => {}
                Ok(rows) => tracing::info!(rows, "purged deleted resource blobs"),
                Err(e) => tracing::warn!(error = %e, "resource blob purge failed"),
            }
        }
    }
}
