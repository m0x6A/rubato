//! The `launch.sh` a new tmux session runs: OTel environment, resource attributes, then
//! `exec claude --session-id … --settings … --permission-mode …` with the prompt.

use crate::kernel::config::Workspace;
use rubato_protocol::LaunchCommand;
use std::path::Path;

pub fn launcher_script(
    workspace: &Workspace,
    cmd: &LaunchCommand,
    worktree: &Path,
    hooks: &Path,
    prompt: &Path,
) -> String {
    let mut s = String::from("#!/bin/sh\n# rubato launch; generated, do not edit\n");
    s.push_str(&format!("export RUBATO_LAUNCH_ID='{}'\n", cmd.id));
    if let Some(otel) = &workspace.otel {
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
        workspace.companion.claude_command,
        cmd.session_id,
        hooks.display(),
        workspace.companion.permission_mode,
        prompt.display()
    ));
    s
}
