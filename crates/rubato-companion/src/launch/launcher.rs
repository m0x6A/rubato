//! Handling a `LaunchCommand` (ADR 0004): refuse anything not addressed to the paired user,
//! one worktree and one tmux session per issue, Claude Code started with `--session-id`,
//! `--settings` (hooks) and the OTel resource attributes.

use super::apply::LiveSession;
use super::script::launcher_script;
use crate::hooks::{HOOKS_TEMPLATE, render_hooks};
use crate::kernel::config::{HookEndpoint, Workspace, write_private};
use crate::kernel::tmux::{Tmux, session_name};
use crate::kernel::tmux_ide::{TmuxIde, version_at_least};
use crate::kernel::{gitx, proc};
use anyhow::{Context, Result};
use chrono::Utc;
use rubato_protocol::{LaunchCommand, LaunchResult, RefusalReason};
use std::path::PathBuf;

/// Owns what a launch needs, cloned once when the runtime starts, so the blocking work can move
/// into `spawn_blocking`.
pub struct Launcher {
    pub workspace: Workspace,
    pub paired_user: String,
    pub state_dir: PathBuf,
    pub hook_endpoint: HookEndpoint,
    pub tmux: Tmux,
    pub tmux_ide: TmuxIde,
}

pub struct Started {
    pub pane_id: String,
    pub tmux_session: String,
    pub worktree: PathBuf,
}

fn refused(cmd: &LaunchCommand, reason: RefusalReason, detail: impl Into<String>) -> LaunchResult {
    LaunchResult::Refused {
        command_id: cmd.id,
        reason,
        detail: detail.into(),
        at: Utc::now(),
    }
}

impl Launcher {
    /// Decide and act. Blocking: call from `spawn_blocking`. `live` is the session already
    /// running for the issue, read from the registry before the call.
    pub fn handle(
        &self,
        cmd: &LaunchCommand,
        live: Option<LiveSession>,
    ) -> (LaunchResult, Option<Started>) {
        if cmd.user != self.paired_user {
            return (
                refused(
                    cmd,
                    RefusalReason::NotPairedUser,
                    format!(
                        "command addressed to {}, this companion is paired with {}",
                        cmd.user, self.paired_user
                    ),
                ),
                None,
            );
        }
        if let Some(e) = live {
            return (
                refused(
                    cmd,
                    RefusalReason::AlreadyRunning,
                    format!(
                        "session {} already runs for {} in {}",
                        e.session_id.as_deref().unwrap_or("?"),
                        cmd.issue,
                        e.tmux_session.as_deref().unwrap_or("?")
                    ),
                ),
                None,
            );
        }
        if cmd.attempt > cmd.attempt_limit {
            return (
                refused(
                    cmd,
                    RefusalReason::AttemptLimitReached,
                    format!("attempt {} of {}", cmd.attempt, cmd.attempt_limit),
                ),
                None,
            );
        }
        match self.tmux_ide.version() {
            Ok(v) if version_at_least(&v, &self.workspace.companion.tmux_ide_min_version) => {}
            Ok(v) => {
                return (
                    refused(
                        cmd,
                        RefusalReason::MissingDependency,
                        format!(
                            "tmux-ide {v} is below the pinned {}",
                            self.workspace.companion.tmux_ide_min_version
                        ),
                    ),
                    None,
                );
            }
            // A hung tmux-ide is not a missing one (plan 40 S4, A2).
            Err(e) if proc::is_timeout(&e) => {
                return (
                    LaunchResult::Failed {
                        command_id: cmd.id,
                        error: format!("tmux-ide version check: {e:#}"),
                        at: Utc::now(),
                    },
                    None,
                );
            }
            Err(e) => {
                return (
                    refused(
                        cmd,
                        RefusalReason::MissingDependency,
                        format!("tmux-ide: {e}"),
                    ),
                    None,
                );
            }
        }
        match self.start(cmd) {
            Ok(started) => (
                LaunchResult::Started {
                    command_id: cmd.id,
                    session_id: cmd.session_id,
                    worktree: started.worktree.to_string_lossy().into_owned(),
                    tmux_session: started.tmux_session.clone(),
                    at: Utc::now(),
                },
                Some(started),
            ),
            Err(e) => (
                LaunchResult::Failed {
                    command_id: cmd.id,
                    error: format!("{e:#}"),
                    at: Utc::now(),
                },
                None,
            ),
        }
    }

    fn start(&self, cmd: &LaunchCommand) -> Result<Started> {
        let repo = self
            .workspace
            .repo(&cmd.issue.repo)
            .with_context(|| format!("{} is not in workspace.yml repos", cmd.issue.repo))?;
        gitx::fetch(&repo.local, self.workspace.companion.command_timeout())
            .context("git fetch")?;
        let worktree = match gitx::worktrees(&repo.local)?
            .into_iter()
            .find(|(_, b)| b.as_deref() == Some(cmd.branch.as_str()))
        {
            Some((path, _)) => path,
            None => self
                .tmux_ide
                .worktree_create(&repo.local, &cmd.branch, &cmd.base_ref)
                .context("tmux-ide worktree create")?,
        };
        let launch_dir = self.state_dir.join("launches").join(cmd.id.to_string());
        std::fs::create_dir_all(&launch_dir)?;
        let hooks_path = launch_dir.join("hooks.json");
        write_private(
            &hooks_path,
            render_hooks(HOOKS_TEMPLATE, &self.hook_endpoint)?.as_bytes(),
        )?;
        let prompt_path = launch_dir.join("prompt.txt");
        std::fs::write(&prompt_path, &cmd.prompt)?;
        let script_path = launch_dir.join("launch.sh");
        std::fs::write(
            &script_path,
            launcher_script(&self.workspace, cmd, &worktree, &hooks_path, &prompt_path),
        )?;
        let name = session_name(&cmd.issue.repo, &cmd.branch);
        if self.tmux.has_session(&name)? {
            anyhow::bail!("tmux session {name} already exists");
        }
        let pane_id = self
            .tmux
            .new_session(&name, &worktree, &format!("sh '{}'", script_path.display()))
            .context("tmux new-session")?;
        if let Err(e) = self.tmux_ide.adopt(&name) {
            tracing::warn!(session = %name, error = %e, "tmux-ide adopt failed; status bar absent");
        }
        Ok(Started {
            pane_id,
            tmux_session: name,
            worktree,
        })
    }
}
