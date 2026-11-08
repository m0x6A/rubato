use anyhow::Result;
use rubato_server::config::Config;
use rubato_server::github::{FakeGitHub, GitHubApi, Octo};
use rubato_server::state::AppState;
use rubato_server::{app, db, poller, telemetry};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;
    let _guard = telemetry::init("rubato-server", config.otel_endpoint.as_deref())?;
    if config.dev_user.is_some() {
        tracing::warn!(
            "RUBATO_DEV_USER is set: every request is that user; never run like this in production"
        );
    }
    let pool = db::connect(&config.database_url).await?;
    let github: Arc<dyn GitHubApi> = match &config.github_token {
        Some(t) => Arc::new(Octo::new(t)?),
        None => {
            tracing::warn!(
                "RUBATO_GITHUB_TOKEN unset: GitHub reads are disabled; map comes from the database copy only"
            );
            Arc::new(FakeGitHub::new())
        }
    };
    let bind = config.bind;
    let state = AppState::new(pool, config, github);
    poller::warm_map(&state).await?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let poll = tokio::spawn(poller::run(state.clone(), shutdown_rx));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "rubato-server listening");
    axum::serve(listener, app::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    let _ = shutdown_tx.send(true);
    let _ = poll.await;
    Ok(())
}
