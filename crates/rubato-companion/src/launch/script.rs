//! The `launch.sh` a new tmux session runs: OTel environment, resource attributes, then
//! `exec claude --session-id … --settings … --permission-mode …` with the prompt.

use crate::kernel::config::{HookEndpoint, Workspace};
use rubato_protocol::LaunchCommand;
use std::path::Path;

pub fn launcher_script(
    workspace: &Workspace,
    cmd: &LaunchCommand,
    worktree: &Path,
    hooks: &Path,
    prompt: &Path,
    hook: &HookEndpoint,
) -> String {
    let mut s = String::from("#!/bin/sh\n# rubato launch; generated, do not edit\n");
    s.push_str(&format!("export RUBATO_LAUNCH_ID='{}'\n", cmd.id));
    // For `plan.ts handoff` (ADR 0055): how the session plans, and where it reports its
    // outcome (`/handoff` on the loopback receiver, the hooks' bearer token). The token is
    // URL-safe base64, so single quotes hold it; the file is written 0600.
    s.push_str(&format!(
        "export RUBATO_PLANNING='{}'\n",
        cmd.planning.name()
    ));
    s.push_str(&format!(
        "export RUBATO_HOOK_URL='http://127.0.0.1:{}'\n",
        hook.port
    ));
    s.push_str(&format!("export RUBATO_HOOK_TOKEN='{}'\n", hook.token));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::CompanionConfig;
    use rubato_protocol::{IssueRef, PlanningMode};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn command(planning: PlanningMode) -> LaunchCommand {
        LaunchCommand {
            id: Uuid::from_u128(7),
            user: "m0x6A".into(),
            issue: IssueRef::new("acme/pantry", 7),
            repo_url: String::new(),
            branch: "issue/7-restock".into(),
            base_ref: "origin/main".into(),
            node_ids: Vec::new(),
            spend_cap_usd: 20.0,
            attempt_limit: 3,
            attempt: 1,
            session_id: Uuid::from_u128(1),
            prompt: String::new(),
            otel_resource: BTreeMap::new(),
            map_version: None,
            plan_path: None,
            draft: None,
            planning,
        }
    }

    /// ADR 0055: the session knows how it plans and where `plan.ts handoff` reports, with the
    /// same port and token its hooks use. The shell sees exactly these values.
    #[test]
    fn the_script_exports_the_planning_mode_and_the_handoff_endpoint() {
        let ws = Workspace {
            server_url: None,
            map: None,
            maps: vec![],
            repos: vec![],
            otel: None,
            companion: CompanionConfig {
                claude_command: "printenv;".into(),
                ..CompanionConfig::default()
            },
        };
        let ep = HookEndpoint {
            port: 43125,
            token: "abcdefghijklmnopqrstuvwxyz012345".into(),
        };
        let dir = tempfile::tempdir().unwrap();
        for (mode, name) in [(PlanningMode::Git, "git"), (PlanningMode::Rubato, "rubato")] {
            let s = launcher_script(
                &ws,
                &command(mode),
                dir.path(),
                &dir.path().join("hooks.json"),
                &dir.path().join("prompt.txt"),
                &ep,
            );
            std::fs::write(dir.path().join("prompt.txt"), "").unwrap();
            // `claude` is `printenv;` here: `exec printenv` prints the environment the script set
            // up, and the rest of that line never runs.
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(&s)
                .output()
                .unwrap();
            let env = String::from_utf8_lossy(&out.stdout);
            assert!(
                env.lines().any(|l| l == format!("RUBATO_PLANNING={name}")),
                "{env}"
            );
            assert!(
                env.lines()
                    .any(|l| l == "RUBATO_HOOK_URL=http://127.0.0.1:43125"),
                "{env}"
            );
            assert!(
                env.lines()
                    .any(|l| l == format!("RUBATO_HOOK_TOKEN={}", ep.token)),
                "{env}"
            );
            assert!(
                env.lines()
                    .any(|l| l == format!("RUBATO_LAUNCH_ID={}", Uuid::from_u128(7)))
            );
        }
    }
}
