//! Handling a `LaunchCommand` (ADR 0004): refuse anything not addressed to the paired user,
//! one worktree and one tmux session per issue, Claude Code started with `--session-id`,
//! `--settings` (hooks) and the OTel resource attributes.

use super::apply::LiveSession;
use super::restore::{Restored, restore};
use super::script::{check_ref, launcher_script, sh_quote};
use crate::hooks::{HOOKS_TEMPLATE, render_hooks};
use crate::kernel::config::{HookEndpoint, Workspace, write_private};
use crate::kernel::gitx::{Git, default_worktree_path};
use crate::kernel::proc;
use crate::kernel::tmux::{Tmux, session_name, version_at_least};
use crate::sessions::turn_script_path;
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
    /// The launch's tmux, under `companion.command_timeout()` (plan 147 Decision 9).
    pub tmux: Tmux,
}

pub struct Started {
    pub pane_id: String,
    pub tmux_session: String,
    pub worktree: PathBuf,
    /// What happened to the command's draft (plan 122 §D5); default when it carried none.
    pub draft: Restored,
    /// No pane: the launch task starts the first turn as a child process (ADR 0062).
    pub headless: bool,
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
        // A headless companion needs no tmux (ADR 0062); an interactive one needs tmux at or
        // above the pin, and nothing else (ADR 0070).
        let headless = self.workspace.companion.is_headless();
        let min = &self.workspace.companion.tmux_min_version;
        match self.tmux.version() {
            _ if headless => {}
            Ok(v) if version_at_least(&v, min) => {}
            Ok(v) => {
                return (
                    refused(
                        cmd,
                        RefusalReason::MissingDependency,
                        format!("tmux {v} is below the pinned {min}"),
                    ),
                    None,
                );
            }
            // A hung tmux is not a missing one (plan 40 S4, A2).
            Err(e) if proc::is_timeout(&e) => {
                return (
                    LaunchResult::Failed {
                        command_id: cmd.id,
                        error: format!("tmux version check: {e:#}"),
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
                        format!("tmux {min} or newer is needed: {e:#}"),
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
                    draft_restored: started.draft.restored,
                    restore_skipped: started.draft.skipped,
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
        super::script::check_branch(cmd)?;
        check_ref("base ref", &cmd.base_ref)?;
        let repo = self
            .workspace
            .repo(&cmd.issue.repo)
            .with_context(|| format!("{} is not in workspace.yml repos", cmd.issue.repo))?;
        let git = Git::new(
            self.workspace.companion.git_command.clone(),
            self.workspace.companion.command_timeout(),
        );
        git.fetch(&repo.local).context("git fetch")?;
        let mut listed = git.worktrees(&repo.local).context("git worktree list")?;
        // A registration for this branch whose directory is gone would be reused below and the
        // session started in a missing directory. Only that one goes (plan 147 Decision 12).
        for stale in listed.iter().filter(|w| {
            w.branch.as_deref() == Some(cmd.branch.as_str()) && (w.prunable || !w.path.exists())
        }) {
            tracing::warn!(path = %stale.path.display(), branch = %cmd.branch, "stale worktree registration removed");
            git.remove_registration(&repo.local, &stale.path)
                .context("git worktree remove")?;
        }
        listed.retain(|w| !w.prunable && w.path.exists());
        // One way for both modes (ADR 0070): the worktree the branch has, else a new one at the
        // path tmux-ide used.
        let (worktree, created) = match listed
            .into_iter()
            .find(|w| w.branch.as_deref() == Some(cmd.branch.as_str()))
        {
            Some(w) => (w.path, false),
            None => {
                let path = default_worktree_path(&repo.local, &cmd.branch);
                git.worktree_add(&repo.local, &path, &cmd.branch, &cmd.base_ref)
                    .context("git worktree add")?;
                (path, true)
            }
        };
        // Before the session starts, so the agent finds the draft on its first turn. Only a
        // worktree this launch created is written (ADR 0053).
        let draft = match &cmd.draft {
            Some(d) => restore(&worktree, d, created).context("restoring the draft")?,
            None => Restored::default(),
        };
        let launch_dir = self.state_dir.join("launches").join(cmd.id.to_string());
        // Owner only: the directory holds the hook token (header file, launch script) and the prompt.
        std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700)
            .create(&launch_dir)?;
        // The token goes in a 0600 header file curl reads; never in a hook command (ADR 0063).
        let header_path = HookEndpoint::header_path(&launch_dir);
        self.hook_endpoint.write_header(&header_path)?;
        let hooks_path = launch_dir.join("hooks.json");
        write_private(
            &hooks_path,
            render_hooks(HOOKS_TEMPLATE, self.hook_endpoint.port, &header_path)?.as_bytes(),
        )?;
        let prompt_path = launch_dir.join("prompt.txt");
        write_private(&prompt_path, cmd.prompt.as_bytes())?;
        let script_path = launch_dir.join("launch.sh");
        // Private: it exports the hook token (`RUBATO_HOOK_TOKEN`), like the header file holds it.
        write_private(
            &script_path,
            launcher_script(
                &self.workspace,
                cmd,
                &worktree,
                &hooks_path,
                &prompt_path,
                &self.hook_endpoint,
            )
            .as_bytes(),
        )?;
        if self.workspace.companion.is_headless() {
            // Every turn runs `turn.sh <claude args>`: the launch's environment and hooks, then
            // the turn's own arguments. The first turn is started by the launch task (async).
            write_private(
                &turn_script_path(&self.state_dir, cmd.id),
                turn_script(&script_path, &self.workspace.companion.claude_command).as_bytes(),
            )?;
            return Ok(Started {
                pane_id: String::new(),
                tmux_session: String::new(),
                worktree,
                draft,
                headless: true,
            });
        }
        let name = session_name(&cmd.issue.repo, &cmd.branch);
        if self.tmux.has_session(&name)? {
            anyhow::bail!("tmux session {name} already exists");
        }
        let pane_id = self
            .tmux
            .new_session(
                &name,
                &worktree,
                &format!("sh {}", sh_quote(&script_path.display().to_string())),
            )
            .context("tmux new-session")?;
        Ok(Started {
            pane_id,
            tmux_session: name,
            worktree,
            draft,
            headless: false,
        })
    }
}

/// A headless turn's script: everything `launch.sh` sets up, with `exec claude …` replaced by the
/// turn's arguments. `launch.sh` is sourced up to its last line, the `exec`, so the two never drift.
fn turn_script(launch_sh: &std::path::Path, claude: &str) -> String {
    format!(
        "#!/bin/sh\n# rubato headless turn; generated, do not edit\n\
         eval \"$(sed '$d' '{}')\"\n\
         exec {claude} --settings \"$(dirname \"$0\")/hooks.json\" \"$@\"\n",
        launch_sh.display()
    )
}
