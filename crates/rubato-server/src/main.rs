use anyhow::Result;
use rubato_server::kernel::config::Config;
use rubato_server::kernel::github::{
    FakeGitHub, GitHubApi, LocalGit, Octo, READ_TIMEOUT, check_app, read_dev_issues,
};
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
    let dev_issues = match &config.dev_issues {
        Some(path) => {
            tracing::warn!(
                "RUBATO_DEV_ISSUES is set: open issues come from that file; development and tests only"
            );
            read_dev_issues(path)?
        }
        None => Default::default(),
    };
    let api = config.github_api_url.as_deref();
    if api.is_some() || config.github_web_url.is_some() {
        tracing::warn!(
            api = config.github_api(),
            web = config.github_web(),
            "RUBATO_GITHUB_API_URL or RUBATO_GITHUB_WEB_URL is set: not talking to github.com"
        );
    }
    let github: Arc<dyn GitHubApi> = match (
        &config.local_repos,
        &config.github_app,
        &config.github_token,
    ) {
        (Some(spec), _, _) => {
            tracing::warn!(
                "RUBATO_LOCAL_REPOS is set: branches and plan files come from local repositories, not GitHub"
            );
            Arc::new(LocalGit::from_spec(spec)?.with_open_issues(dev_issues))
        }
        (None, Some(app), _) => {
            // A wrong key, client id or installation ends the start here, with its name.
            if let Err(e) = check_app(app, config.github_client_id.as_deref(), api).await {
                tracing::error!(error = %e, "github app check failed");
                anyhow::bail!("github app check failed: {e}");
            }
            tracing::info!(
                app_id = app.app_id,
                installation_id = app.installation_id,
                "acting as a GitHub App installation; sessions need a team record"
            );
            Arc::new(Octo::for_installation(app, api)?)
        }
        (None, None, Some(t)) => {
            tracing::warn!(
                "no GitHub App configured: reading with RUBATO_GITHUB_TOKEN, login is open to any GitHub account, and map changes cannot be proposed"
            );
            Arc::new(match api {
                Some(base) => Octo::with_base_uri(t, base, READ_TIMEOUT)?,
                None => Octo::new(t)?,
            })
        }
        (None, None, None) => {
            tracing::warn!(
                "no GitHub App and no RUBATO_GITHUB_TOKEN: GitHub reads are disabled; map comes from the database copy only"
            );
            let fake = FakeGitHub::new();
            fake.with(|g| g.open = dev_issues);
            Arc::new(fake)
        }
    };
    let bind = config.bind;
    let state = AppState::new(pool, config, github);
    rubato_server::kernel::workspaces::report(&state).await?;
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
