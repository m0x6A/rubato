//! Handling a `LaunchCommand` (ADR 0004): refuse anything not addressed to the paired user,
//! one worktree and one tmux session per issue, Claude Code started with `--session-id`,
//! `--settings` (hooks) and the OTel resource attributes.

use crate::config::{HookEndpoint, Workspace, write_private};
use crate::gitx;
use crate::hooks::{HOOKS_TEMPLATE, render_hooks};
use crate::sessions::Registry;
use crate::tmux::{Tmux, session_name};
use crate::tmux_ide::{TmuxIde, version_at_least};
use anyhow::{Context, Result};
use chrono::Utc;
use rubato_protocol::{LaunchCommand, LaunchResult, RefusalReason};
use std::path::{Path, PathBuf};

pub struct Launcher<'a> {
    pub workspace: &'a Workspace,
    pub paired_user: &'a str,
    pub state_dir: &'a Path,
    pub hook_endpoint: &'a HookEndpoint,
    pub tmux: &'a Tmux,
    pub tmux_ide: &'a TmuxIde,
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

impl Launcher<'_> {
    /// Decide and act. Blocking: call from `spawn_blocking`.
    pub fn handle(
        &self,
        cmd: &LaunchCommand,
        registry: &Registry,
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
        if let Some(e) = registry.live_for_issue(&cmd.issue) {
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
        gitx::fetch(&repo.local).context("git fetch")?;
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
            render_hooks(HOOKS_TEMPLATE, self.hook_endpoint)?.as_bytes(),
        )?;
        let prompt_path = launch_dir.join("prompt.txt");
        std::fs::write(&prompt_path, &cmd.prompt)?;
        let script_path = launch_dir.join("launch.sh");
        std::fs::write(
            &script_path,
            self.launcher_script(cmd, &worktree, &hooks_path, &prompt_path),
        )?;
        let name = session_name(&cmd.issue.repo, &cmd.branch);
        if self.tmux.has_session(&name) {
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

    fn launcher_script(
        &self,
        cmd: &LaunchCommand,
        worktree: &Path,
        hooks: &Path,
        prompt: &Path,
    ) -> String {
        let mut s = String::from("#!/bin/sh\n# rubato launch; generated, do not edit\n");
        s.push_str(&format!("export RUBATO_LAUNCH_ID='{}'\n", cmd.id));
        if let Some(otel) = &self.workspace.otel {
            s.push_str("export CLAUDE_CODE_ENABLE_TELEMETRY=1\nexport OTEL_METRICS_EXPORTER=otlp\nexport OTEL_LOGS_EXPORTER=otlp\n");
            s.push_str(&format!(
                "export OTEL_EXPORTER_OTLP_ENDPOINT='{}'\n",
                otel.endpoint
            ));
            s.push_str(&format!(
                "export OTEL_EXPORTER_OTLP_PROTOCOL='{}'\n",
                otel.protocol.as_deref().unwrap_or("http/protobuf")
            ));
        }
        let attrs: Vec<String> = cmd
            .otel_resource
            .iter()
            .map(|(k, v)| format!("{k}={}", v.replace([',', ' '], "_")))
            .collect();
        if !attrs.is_empty() {
            s.push_str(&format!(
                "export OTEL_RESOURCE_ATTRIBUTES='{}'\n",
                attrs.join(",")
            ));
        }
        s.push_str(&format!("cd '{}' || exit 1\n", worktree.display()));
        s.push_str(&format!(
            "exec {} --session-id '{}' --settings '{}' --permission-mode '{}' \"$(cat '{}')\"\n",
            self.workspace.companion.claude_command,
            cmd.session_id,
            hooks.display(),
            self.workspace.companion.permission_mode,
            prompt.display()
        ));
        s
    }
}
