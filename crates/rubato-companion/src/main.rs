use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use rubato_companion::config::{HookEndpoint, State, Workspace, state_dir};
use rubato_companion::daemon::{DaemonConfig, start};
use rubato_companion::hooks::{HOOKS_TEMPLATE, render_hooks};
use rubato_companion::{VERSION, pairing, telemetry};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "rubato-companion", version = VERSION, about = "Per-developer Rubato companion: outbound only, your tmux only.")]
struct Cli {
    /// Path to workspace.yml (default: $RUBATO_WORKSPACE or ~/.config/rubato/workspace.yml).
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    /// State directory (default: $RUBATO_STATE_DIR or ~/.local/state/rubato).
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Pair this machine with your GitHub login on the server (device-code flow).
    Pair {
        #[arg(long)]
        server: String,
    },
    /// Run the daemon.
    Run,
    /// Show pairing and hook endpoint state.
    Status,
    /// Render hooks.json for the current hook endpoint (for a manual session).
    RenderHooks,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let state_dir = cli.state_dir.clone().unwrap_or_else(state_dir);
    let ws_path = cli
        .workspace
        .clone()
        .unwrap_or_else(Workspace::default_path);
    match cli.cmd {
        Cmd::Pair { server } => {
            let _g = telemetry::init("rubato-companion", None)?;
            let companion_id = State::load(&state_dir)?
                .map(|s| s.companion_id)
                .unwrap_or_else(Uuid::new_v4);
            let hostname = std::fs::read_to_string("/etc/hostname")
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|_| "unknown".into());
            let paired = pairing::pair(&server, companion_id, &hostname, VERSION, |start| {
                eprintln!(
                    "Open {} while logged in with GitHub and enter the code: {}",
                    start.verification_url, start.user_code
                );
            })
            .await?;
            State {
                companion_id,
                server_url: server,
                user: paired.user.clone(),
                pairing_token: paired.token,
                last_seq: 0,
                last_server_seq: 0,
            }
            .save(&state_dir)?;
            eprintln!("paired as {} (companion {})", paired.user, companion_id);
            Ok(())
        }
        Cmd::Run => {
            let workspace = Workspace::load(&ws_path)
                .with_context(|| format!("loading {}", ws_path.display()))?;
            let _g = telemetry::init(
                "rubato-companion",
                workspace.otel.as_ref().map(|o| o.endpoint.as_str()),
            )?;
            let Some(state) = State::load(&state_dir)? else {
                bail!("not paired: run `rubato-companion pair --server <url>` first");
            };
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let (_handles, join) = start(
                DaemonConfig {
                    workspace,
                    state,
                    state_dir,
                },
                shutdown_rx,
            )
            .await?;
            tokio::select! {
                r = join => r??,
                _ = tokio::signal::ctrl_c() => {
                    let _ = shutdown_tx.send(true);
                }
            }
            Ok(())
        }
        Cmd::Status => {
            match State::load(&state_dir)? {
                Some(s) => println!(
                    "paired as {} with {} (companion {})",
                    s.user, s.server_url, s.companion_id
                ),
                None => println!("not paired"),
            }
            let ep = HookEndpoint::path(&state_dir);
            match std::fs::read_to_string(&ep) {
                Ok(t) => println!("hook endpoint: {}", t.trim()),
                Err(_) => println!("hook endpoint: daemon not running"),
            }
            Ok(())
        }
        Cmd::RenderHooks => {
            let text = std::fs::read_to_string(HookEndpoint::path(&state_dir))
                .context("daemon not running (no hook endpoint file)")?;
            let ep: HookEndpoint = serde_json::from_str(&text)?;
            print!("{}", render_hooks(HOOKS_TEMPLATE, &ep)?);
            Ok(())
        }
    }
}
