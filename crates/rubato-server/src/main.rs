use anyhow::Result;
use rubato_server::kernel::config::Config;
use rubato_server::kernel::github::{FakeGitHub, GitHubApi, LocalGit, Octo};
use rubato_server::kernel::state::AppState;
use rubato_server::kernel::{db, telemetry};
use rubato_server::{app, map, tasks};
use std::future::IntoFuture;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;
    let _guard = telemetry::init("rubato-server", config.otel_endpoint.as_deref())?;
    for w in &config.warnings {
        tracing::warn!("{w}");
    }
    if config.dev_user.is_some() {
        tracing::warn!(
            "RUBATO_DEV_USER is set: every request is that user; never run like this in production"
        );
    }
    let pool = db::connect(&config.database_url).await?;
    let github: Arc<dyn GitHubApi> = match (&config.github_token, &config.local_repos) {
        (_, Some(spec)) => {
            tracing::warn!(
                "RUBATO_LOCAL_REPOS is set: branches and plan files come from local repositories, not GitHub"
            );
            Arc::new(LocalGit::from_spec(spec)?)
        }
        (Some(t), None) => Arc::new(Octo::new(t)?),
        (None, None) => {
            tracing::warn!(
                "RUBATO_GITHUB_TOKEN unset: GitHub reads are disabled; map comes from the database copy only"
            );
            Arc::new(FakeGitHub::new())
        }
    };
    let bind = config.bind;
    let state = AppState::new(pool, config, github);
    map::warm(&state).await?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let background = tasks::spawn(state.clone(), shutdown_rx.clone());
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "rubato-server listening");
    let serve = axum::serve(listener, app::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .into_future();
    // Supervise while serving, so a dead poll task ends the process at once (design D12).
    let supervised = tasks::supervise(background, shutdown_rx);
    tokio::pin!(supervised);
    tokio::select! {
        served = serve => {
            let _ = shutdown_tx.send(true);
            supervised.await?;
            served?;
        }
        r = &mut supervised => r?,
    }
    Ok(())
}
