//! The `launch.sh` a new tmux session runs: OTel environment, resource attributes, then
//! `exec claude --session-id … --settings … --permission-mode …` with the prompt.
//!
//! Every value that reaches the script is single-quoted with [`sh_quote`]: some come from the
//! server (branch, repo, node ids), and the script runs on the developer's machine.

use crate::kernel::config::Workspace;
use anyhow::{Result, bail};
use rubato_protocol::LaunchCommand;
use std::path::Path;

/// `s` as one sh word: single-quoted, each `'` written as `'\''`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Refuse a branch or base ref the companion would not create itself: only letters, digits and `._/-`,
/// no `..` (the worktree path is built from the branch), no leading `-` (read as an option)
/// or `/`, no empty path part, no `.lock` ending.
pub fn check_ref(what: &str, r: &str) -> Result<()> {
    let ok = !r.is_empty()
        && r.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
        && !r.starts_with(['-', '/'])
        && !r.ends_with('/')
        && !r.ends_with(".lock")
        && !r.contains("..")
        && !r.contains("//");
    if !ok {
        bail!("{what} {r:?} is not a branch name the companion accepts");
    }
    Ok(())
}

pub fn launcher_script(
    workspace: &Workspace,
    cmd: &LaunchCommand,
    worktree: &Path,
    hooks: &Path,
    prompt: &Path,
) -> String {
    let q = |s: &str| sh_quote(s);
    let mut s = String::from("#!/bin/sh\n# rubato launch; generated, do not edit\n");
    s.push_str(&format!(
        "export RUBATO_LAUNCH_ID={}\n",
        q(&cmd.id.to_string())
    ));
    if let Some(otel) = &workspace.otel {
        s.push_str("export CLAUDE_CODE_ENABLE_TELEMETRY=1\nexport OTEL_METRICS_EXPORTER=otlp\nexport OTEL_LOGS_EXPORTER=otlp\n");
        s.push_str(&format!(
            "export OTEL_EXPORTER_OTLP_ENDPOINT={}\n",
            q(&otel.endpoint)
        ));
        s.push_str(&format!(
            "export OTEL_EXPORTER_OTLP_PROTOCOL={}\n",
            q(otel.protocol.as_deref().unwrap_or("http/protobuf"))
        ));
    }
    let attrs: Vec<String> = cmd
        .otel_resource
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                k.replace([',', ' ', '='], "_"),
                v.replace([',', ' '], "_")
            )
        })
        .collect();
    if !attrs.is_empty() {
        s.push_str(&format!(
            "export OTEL_RESOURCE_ATTRIBUTES={}\n",
            q(&attrs.join(","))
        ));
    }
    s.push_str(&format!(
        "cd {} || exit 1\n",
        q(&worktree.display().to_string())
    ));
    // `claude_command` is the developer's own config and may carry arguments: left as written.
    s.push_str(&format!(
        "exec {} --session-id {} --settings {} --permission-mode {} \"$(cat {})\"\n",
        workspace.companion.claude_command,
        q(&cmd.session_id.to_string()),
        q(&hooks.display().to_string()),
        q(&workspace.companion.permission_mode),
        q(&prompt.display().to_string())
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_sh() {
        let nasty = ["plain", "it's", "a'b'c", "'", "$(touch /tmp/x)", "a\nb", ""];
        for v in nasty {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("printf '%s' {}", sh_quote(v)))
                .output()
                .unwrap();
            assert_eq!(String::from_utf8(out.stdout).unwrap(), v, "value {v:?}");
        }
    }

    #[test]
    fn refs_the_companion_accepts() {
        for ok in [
            "issue/27-launch-without-lock",
            "origin/main",
            "issue/3-v0.2",
            "issue/9-sök-räkna",
        ] {
            assert!(check_ref("branch", ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "issue/1-x';touch /tmp/p;'",
            "issue/1-../../etc",
            "-b",
            "/abs",
            "issue/",
            "a//b",
            "x.lock",
            "issue/1-$(id)",
            "issue/1 two",
        ] {
            assert!(check_ref("branch", bad).is_err(), "{bad:?}");
        }
    }
}
