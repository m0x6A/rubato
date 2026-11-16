use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use rubato_companion::hooks::{HOOKS_TEMPLATE, render_manual_hooks, write_into};
use rubato_companion::kernel::config::{HookEndpoint, State, Workspace, state_dir};
use rubato_companion::kernel::telemetry;
use rubato_companion::runtime::{DaemonConfig, start};
use rubato_companion::{VERSION, pairing};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rubato-companion", version = VERSION, about = "Per-developer Rubato companion: outbound only, your tmux only.")]
struct Cli {
    /// Path to local.yml (default: $RUBATO_LOCAL, else ~/.config/rubato/local.yml; the old $RUBATO_WORKSPACE and workspace.yml are still read, with a warning).
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
        /// Allow an http:// or ws:// server URL that is not loopback. The pairing token and
        /// launch commands then travel unencrypted on the network. Stored in companion.json, so
        /// `run` accepts the same URL without the flag; pair again without it to undo.
        #[arg(long)]
        insecure: bool,
    },
    /// Run the daemon.
    Run,
    /// Show pairing and hook endpoint state.
    Status,
    /// Render the hooks for a session started by hand (ADR 0078): to stdout, or merged into a
    /// repo's or worktree's `.claude/settings.local.json` with --into, keeping its other settings.
    RenderHooks {
        /// The settings file to merge into, such as `<repo>/.claude/settings.local.json`.
        #[arg(long)]
        into: Option<PathBuf>,
    },
    /// Stay in the foreground doing nothing. The Gherkin suite copies this binary under the name
    /// `claude` as a stand-in pane process; it must not depend on coreutils.
    #[command(hide = true)]
    Idle {
        /// Append every line typed into the pane to this file, as it arrives, so a scenario can
        /// read what a session message typed (plan 122 §D1).
        #[arg(long)]
        record: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let state_dir = cli.state_dir.clone().unwrap_or_else(state_dir);
    let (ws_path, old_name) = Workspace::resolve(cli.workspace.clone());
    match cli.cmd {
        Cmd::Pair { server, insecure } => {
            let _g = telemetry::init("rubato-companion", None)?;
            let hostname = std::fs::read_to_string("/etc/hostname")
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|_| "unknown".into());
            let state = pairing::pair_and_save(
                &state_dir,
                &server,
                insecure,
                &hostname,
                VERSION,
                |start| {
                    eprintln!(
                        "Open {} while logged in with GitHub and enter the code: {}",
                        start.verification_url, start.user_code
                    );
                },
            )
            .await?;
            eprintln!(
                "paired as {} (companion {})",
                state.user, state.companion_id
            );
            eprintln!("{}", pairing::WHAT_LEAVES);
            Ok(())
        }
        Cmd::Run => {
            let workspace = Workspace::load(&ws_path)
                .with_context(|| format!("loading {}", ws_path.display()))?;
            // ADR 0076: the container image runs headless only.
            if let Some(why) = rubato_companion::kernel::config::container_refusal(
                workspace.companion.mode,
                std::env::var("RUBATO_IN_CONTAINER").ok().as_deref(),
            ) {
                bail!("{why}");
            }
            let _g = telemetry::init(
                "rubato-companion",
                workspace.otel.as_ref().map(|o| o.endpoint.as_str()),
            )?;
            if old_name {
                tracing::warn!("{}", Workspace::old_name_warning(&ws_path));
            }
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
            // An essential task ending is an error and a non-zero exit (design D12); Ctrl-C
            // stops every task and exits 0.
            let mut join = join;
            tokio::select! {
                r = &mut join => r??,
                _ = tokio::signal::ctrl_c() => {
                    let _ = shutdown_tx.send(true);
                    join.await??;
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
            // The port and the header file's path, never the endpoint file: it holds the token
            // (ADR 0063).
            let Some(ep) = HookEndpoint::load(&state_dir) else {
                println!("hook endpoint: daemon not running");
                return Ok(());
            };
            let listening = std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], ep.port)),
                std::time::Duration::from_millis(300),
            )
            .is_ok();
            println!(
                "hook endpoint: port {} ({})",
                ep.port,
                if listening {
                    "listening"
                } else {
                    "not listening"
                }
            );
            println!(
                "hook header file: {}",
                HookEndpoint::header_path(&state_dir).display()
            );
            let workspace = match Workspace::load(&ws_path) {
                Ok(w) => w,
                Err(e) => {
                    println!(
                        "no workspace at {}: directories not checked ({e:#})",
                        ws_path.display()
                    );
                    return Ok(());
                }
            };
            // Each repo's clone and its worktrees: where a session may be started by hand.
            let mut dirs = Vec::new();
            for r in &workspace.repos {
                match rubato_companion::kernel::gitx::worktrees(&r.local) {
                    Ok(list) => dirs.extend(list.into_iter().map(|(p, _)| p)),
                    Err(_) => dirs.push(r.local.clone()),
                }
            }
            if !listening {
                println!(
                    "the directories below are judged against port {}, which nothing listens on: start the companion with `rubato-companion run`",
                    ep.port
                );
            }
            for line in rubato_companion::hooks::report(&dirs, ep.port) {
                println!("{line}");
            }
            Ok(())
        }
        Cmd::Idle { record } => {
            if let Some(path) = record {
                std::thread::spawn(move || record_stdin(&path));
            }
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        }
        Cmd::RenderHooks { into } => {
            let text = std::fs::read_to_string(HookEndpoint::path(&state_dir))
                .context("daemon not running (no hook endpoint file)")?;
            let ep: HookEndpoint = serde_json::from_str(&text)?;
            let hooks = render_manual_hooks(
                HOOKS_TEMPLATE,
                ep.port,
                &HookEndpoint::header_path(&state_dir),
            )?;
            match into {
                Some(path) => {
                    write_into(&path, &hooks)?;
                    println!("hooks for port {} written into {}", ep.port, path.display());
                }
                None => println!("{hooks}"),
            }
            Ok(())
        }
    }
}

/// `idle --record`: each line read from stdin, appended to `path` with its newline and flushed.
/// Ends at end of input; the process stays in the foreground either way.
fn record_stdin(path: &std::path::Path) {
    use std::io::{BufRead, Write};
    let mut stdin = std::io::stdin().lock();
    let mut line = Vec::new();
    while matches!(stdin.read_until(b'\n', &mut line), Ok(n) if n > 0) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = f.write_all(&line);
            let _ = f.flush();
        }
        line.clear();
    }
}
