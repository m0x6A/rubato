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

/// Refuse a launch whose branch is not the issue's own `issue/<n>-<slug>` (ADR 0006, issue #155).
/// The branch names the worktree folder and goes to tmux-ide and into the script.
pub fn check_branch(cmd: &LaunchCommand) -> Result<()> {
    if !cmd.issue.is_branch_for(&cmd.branch) {
        bail!(
            "branch {:?} is not issue/{}-<slug> for {}",
            cmd.branch,
            cmd.issue.number,
            cmd.issue
        );
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
            // A key keeps only `[A-Za-z0-9._-]`, so it can never break the `k=v,k=v` format.
            let key: String = k
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("{key}={}", v.replace([',', ' '], "_"))
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
    // `claude_command` is the developer's own `local.yml`, never a server value, and may carry
    // arguments (`claude --model …`): written as is, so it is the one unquoted word here.
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

    fn command(otel: &[(&str, &str)]) -> LaunchCommand {
        LaunchCommand {
            id: uuid::Uuid::from_u128(155),
            user: "m0x6A".into(),
            issue: rubato_protocol::IssueRef::new("acme/pantry", 7),
            repo_url: String::new(),
            branch: "issue/7-restock".into(),
            base_ref: "origin/main".into(),
            node_ids: vec![],
            spend_cap_usd: 1.0,
            attempt_limit: 1,
            attempt: 1,
            session_id: uuid::Uuid::from_u128(1),
            prompt: String::new(),
            otel_resource: otel
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            map_version: None,
        }
    }

    /// The #155 regression: a node id that ends the quoted word and runs `touch` if the quoting
    /// is wrong. The script must parse (`sh -n`) and, run with `claude_command` stubbed to print
    /// the attributes, print the id as text and leave no marker file.
    #[test]
    fn a_node_id_with_a_quote_stays_text() {
        let d = tempfile::tempdir().unwrap();
        let marker = d.path().join("p");
        let node = format!("a'$(touch${{IFS}}{})'", marker.display());
        let worktree = d.path().join("wt");
        std::fs::create_dir(&worktree).unwrap();
        let prompt = d.path().join("prompt.txt");
        std::fs::write(&prompt, "hi").unwrap();
        let mut ws = Workspace::parse("{}").unwrap();
        ws.companion.claude_command = "sh -c 'printf %s \"$OTEL_RESOURCE_ATTRIBUTES\"' stub".into();
        let cmd = command(&[("rubato.node", node.as_str())]);
        let script = launcher_script(&ws, &cmd, &worktree, &d.path().join("hooks.json"), &prompt);
        let path = d.path().join("launch.sh");
        std::fs::write(&path, &script).unwrap();

        let parsed = std::process::Command::new("sh")
            .arg("-n")
            .arg(&path)
            .output()
            .unwrap();
        assert!(parsed.status.success(), "sh -n: {parsed:?}\n{script}");
        let out = std::process::Command::new("sh")
            .arg(&path)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}\n{script}");
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            format!("rubato.node={node}")
        );
        assert!(!marker.exists(), "the id ran as a command:\n{script}");
    }

    #[test]
    fn only_the_issues_own_branch_is_launched() {
        let mut cmd = command(&[]);
        assert!(check_branch(&cmd).is_ok());
        for bad in [
            "issue/9-other",
            "main",
            "issue/7-../../escape",
            "issue/7-a/b",
        ] {
            cmd.branch = bad.into();
            let e = check_branch(&cmd).unwrap_err().to_string();
            assert!(e.contains("is not issue/7-<slug> for acme/pantry#7"), "{e}");
        }
    }

    #[test]
    fn attribute_keys_keep_only_safe_characters() {
        let ws = Workspace::parse("{}").unwrap();
        let cmd = command(&[("a b,c=d'e", "v"), ("rubato.issue", "acme/pantry#7")]);
        let p = Path::new("/x");
        let script = launcher_script(&ws, &cmd, p, p, p);
        assert!(
            script.contains(
                "export OTEL_RESOURCE_ATTRIBUTES='a_b_c_d_e=v,rubato.issue=acme/pantry#7'\n"
            ),
            "{script}"
        );
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
