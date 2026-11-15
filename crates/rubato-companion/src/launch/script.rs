//! The `launch.sh` a new tmux session runs: OTel environment, resource attributes, then
//! `exec claude --session-id … --settings … --permission-mode …` with the prompt.
//!
//! Claude Code does not pass `OTEL_*` variables to the processes it spawns (its Bash tool,
//! hooks), so the script also exports each one as `RUBATO_OTEL_*` for the skill's
//! `otel-emit.ts` (ADR 0061). The mirror is expanded when the script runs, so a header from the
//! developer's environment is never written into this file.
//!
//! Every value that reaches the script is single-quoted with [`sh_quote`]: some come from the
//! server (branch, repo, node ids), and the script runs on the developer's machine.

use crate::kernel::config::{HookEndpoint, Workspace};
use anyhow::{Result, bail};
use rubato_protocol::LaunchCommand;
use std::path::Path;

/// The variables mirrored as `RUBATO_<name>`, in the order they are exported.
pub const MIRRORED: [&str; 5] = [
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_PROTOCOL",
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_RESOURCE_ATTRIBUTES",
];

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
    hook: &HookEndpoint,
) -> String {
    let q = |s: &str| sh_quote(s);
    let mut s = String::from("#!/bin/sh\n# rubato launch; generated, do not edit\n");
    s.push_str(&format!(
        "export RUBATO_LAUNCH_ID={}\n",
        q(&cmd.id.to_string())
    ));
    // For `plan.ts handoff` (ADR 0055): how the session plans, and where it reports its
    // outcome (`/handoff` on the loopback receiver, the hooks' bearer token). The file is
    // written 0600.
    s.push_str(&format!(
        "export RUBATO_PLANNING={}\n",
        q(cmd.planning.name())
    ));
    s.push_str(&format!(
        "export RUBATO_HOOK_URL={}\n",
        q(&format!("http://127.0.0.1:{}", hook.port))
    ));
    s.push_str(&format!("export RUBATO_HOOK_TOKEN={}\n", q(&hook.token)));
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
        // Merged, not replaced: a `service.namespace` the developer's environment sets is kept.
        // Rubato itself never sets one (#140).
        s.push_str(&format!(
            "export OTEL_RESOURCE_ATTRIBUTES=\"${{OTEL_RESOURCE_ATTRIBUTES:+$OTEL_RESOURCE_ATTRIBUTES,}}\"{}\n",
            q(&attrs.join(","))
        ));
    }
    for name in MIRRORED {
        s.push_str(&format!("export RUBATO_{name}=\"${{{name}-}}\"\n"));
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
    use crate::kernel::config::{CompanionConfig, OtelConfig};
    use rubato_protocol::{IssueRef, PlanningMode};
    use std::collections::{BTreeMap, HashMap};
    use std::process::Command;
    use uuid::Uuid;

    fn workspace(otel: bool) -> Workspace {
        Workspace {
            server_url: None,
            map: None,
            maps: vec![],
            repos: vec![],
            otel: otel.then(|| OtelConfig {
                endpoint: "http://collector:4318".into(),
                protocol: None,
            }),
            companion: CompanionConfig {
                // Print the environment instead of starting Claude Code.
                claude_command: "env #".into(),
                ..CompanionConfig::default()
            },
        }
    }

    fn command() -> LaunchCommand {
        LaunchCommand {
            id: Uuid::nil(),
            user: "m0x6A".into(),
            issue: IssueRef {
                repo: "acme/pantry".into(),
                number: 7,
            },
            repo_url: "https://github.com/acme/pantry.git".into(),
            branch: "issue/7-restock".into(),
            base_ref: "origin/main".into(),
            node_ids: vec![],
            spend_cap_usd: 5.0,
            attempt_limit: 1,
            attempt: 1,
            session_id: Uuid::nil(),
            prompt: "go".into(),
            otel_resource: BTreeMap::from([
                ("rubato.issue".into(), "acme/pantry#7".into()),
                ("rubato.branch".into(), "issue/7-it's".into()),
            ]),
            map_version: None,
            plan_path: None,
            planning: PlanningMode::Git,
        }
    }

    fn endpoint() -> HookEndpoint {
        HookEndpoint {
            port: 43125,
            token: "abcdefghijklmnopqrstuvwxyz012345".into(),
        }
    }

    /// Run the script under `sh` with `env` as the environment, and return what "claude" saw.
    fn run(script: &str, env: &[(&str, &str)]) -> HashMap<String, String> {
        let dir = tempfile::tempdir().unwrap();
        let prompt = dir.path().join("prompt");
        std::fs::write(&prompt, "go").unwrap();
        let script = script.replace("__PROMPT__", &prompt.display().to_string());
        let out = Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    fn script(otel: bool) -> String {
        launcher_script(
            &workspace(otel),
            &command(),
            &std::env::temp_dir(),
            Path::new("/tmp/hooks.json"),
            Path::new("__PROMPT__"),
            &endpoint(),
        )
    }

    /// ADR 0055: the session knows how it plans and where `plan.ts handoff` reports, with the
    /// same port and token its hooks use. The shell sees exactly these values.
    #[test]
    fn the_script_exports_the_planning_mode_and_the_handoff_endpoint() {
        for (mode, name) in [(PlanningMode::Git, "git"), (PlanningMode::Rubato, "rubato")] {
            let cmd = LaunchCommand {
                planning: mode,
                ..command()
            };
            let s = launcher_script(
                &workspace(false),
                &cmd,
                &std::env::temp_dir(),
                Path::new("/tmp/hooks.json"),
                Path::new("__PROMPT__"),
                &endpoint(),
            );
            let env = run(&s, &[]);
            assert_eq!(env.get("RUBATO_PLANNING").map(String::as_str), Some(name));
            assert_eq!(
                env.get("RUBATO_HOOK_URL").map(String::as_str),
                Some("http://127.0.0.1:43125")
            );
            assert_eq!(env.get("RUBATO_HOOK_TOKEN"), Some(&endpoint().token));
            assert_eq!(env.get("RUBATO_LAUNCH_ID"), Some(&Uuid::nil().to_string()));
        }
    }

    #[test]
    fn the_resource_attributes_are_merged_and_mirrored() {
        let env = run(
            &script(true),
            &[("OTEL_RESOURCE_ATTRIBUTES", "service.namespace=team")],
        );
        let attrs = &env["OTEL_RESOURCE_ATTRIBUTES"];
        assert!(attrs.starts_with("service.namespace=team,"), "{attrs}");
        assert!(attrs.contains("rubato.issue=acme/pantry#7"), "{attrs}");
        assert!(attrs.contains("rubato.branch=issue/7-it's"), "{attrs}");
        assert_eq!(&env["RUBATO_OTEL_RESOURCE_ATTRIBUTES"], attrs);
        assert_eq!(
            env["RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT"],
            "http://collector:4318"
        );
        assert_eq!(env["RUBATO_OTEL_EXPORTER_OTLP_PROTOCOL"], "http/protobuf");
    }

    #[test]
    fn without_inherited_attributes_only_rubato_ones_are_set() {
        let env = run(&script(true), &[]);
        assert!(
            env["OTEL_RESOURCE_ATTRIBUTES"].starts_with("rubato."),
            "{:?}",
            env["OTEL_RESOURCE_ATTRIBUTES"]
        );
    }

    #[test]
    fn headers_reach_the_mirror_but_never_the_script_file() {
        let secret = "authorization=Bearer s3cret";
        let text = script(true);
        assert!(!text.contains("s3cret"));
        let env = run(&text, &[("OTEL_EXPORTER_OTLP_HEADERS", secret)]);
        assert_eq!(env["RUBATO_OTEL_EXPORTER_OTLP_HEADERS"], secret);
    }

    #[test]
    fn without_otel_config_the_developers_own_endpoint_is_mirrored() {
        let env = run(
            &script(false),
            &[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://own:4318")],
        );
        assert_eq!(env["RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT"], "http://own:4318");
        assert!(!env.contains_key("CLAUDE_CODE_ENABLE_TELEMETRY"));
        assert_eq!(env["RUBATO_OTEL_EXPORTER_OTLP_HEADERS"], "");
    }

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
