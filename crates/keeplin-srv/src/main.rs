use std::sync::Arc;

use anyhow::Context;
use keeplin_srv::{config::Config, http::router, state::AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("keeplin_srv=info".parse()?),
        )
        .init();

    dotenvy::dotenv().ok();
    let config = Config::from_env();

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .context("connect to PostgreSQL")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("run database migrations")?;

    let state = Arc::new(AppState::new(config.clone(), pool));
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port))
        .await
        .context("bind server address")?;

    tracing::info!(port = config.port, "Keeplin server listening");
    axum::serve(listener, app).await.context("run server")?;

    Ok(())
}
