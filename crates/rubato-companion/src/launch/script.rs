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

/// The MCP server a launched session gets (ADR 0071, plan 220 §5): its name in `mcp.json`.
pub const MCP_SERVER: &str = "rubato-companion";
/// `--allowedTools`: every tool of that server, so a headless turn (which cannot answer a
/// permission prompt) can call them; no other server's.
pub const ALLOWED_TOOLS: &str = "mcp__rubato-companion";
/// Next to `hooks.json` in the launch dir, mode 0600: it holds the receiver's token.
pub const MCP_CONFIG_FILE: &str = "mcp.json";

/// `mcp.json` for one launch: the receiver's `/mcp` on loopback, its token and the launch id as
/// headers. Claude Code reads it through `--mcp-config <path>`, so the token is on no command
/// line (ADR 0063).
pub fn render_mcp_config(hook: &HookEndpoint, launch_id: uuid::Uuid) -> String {
    serde_json::json!({"mcpServers": {MCP_SERVER: {
        "type": "http",
        "url": format!("http://127.0.0.1:{}/mcp", hook.port),
        "headers": {
            "Authorization": format!("Bearer {}", hook.token),
            "X-Rubato-Launch": launch_id.to_string(),
        },
    }}})
    .to_string()
}

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

/// Refuse a launch whose branch is not the issue's own `issue/<n>-<slug>` (ADR 0006, issue #155).
/// The branch names the worktree folder and goes to git and into the script.
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
    // `claude_command` is the developer's own `local.yml`, never a server value, and may carry
    // arguments (`claude --model …`): written as is, so it is the one unquoted word here. The MCP
    // flags take lists of values, so they come right after it and `--session-id` ends them; the
    // prompt stays the last word (plan 220 §5). `mcp.json` sits next to `hooks.json`.
    s.push_str(&format!(
        "exec {} --mcp-config {} --allowedTools {} --session-id {} --settings {} --permission-mode {} \"$(cat {})\"\n",
        workspace.companion.claude_command,
        q(&hooks.with_file_name(MCP_CONFIG_FILE).display().to_string()),
        ALLOWED_TOOLS,
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
            draft: None,
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

    fn command_with_otel(otel: &[(&str, &str)]) -> LaunchCommand {
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
            plan_path: None,
            draft: None,
            planning: PlanningMode::Git,
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
        let cmd = command_with_otel(&[("rubato.node", node.as_str())]);
        let script = launcher_script(
            &ws,
            &cmd,
            &worktree,
            &d.path().join("hooks.json"),
            &prompt,
            &endpoint(),
        );
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

    /// Plan 220 §5: `--mcp-config` and `--allowedTools` take lists, so they go right after the
    /// claude command; `--session-id` ends their lists and the prompt is still the last word.
    #[test]
    fn the_mcp_flags_come_first_and_the_prompt_stays_last() {
        let d = tempfile::tempdir().unwrap();
        let stand = d.path().join("claude.sh");
        std::fs::write(&stand, "printf '%s\\n' \"$@\"\n").unwrap();
        let prompt = d.path().join("prompt.txt");
        std::fs::write(&prompt, "hi there").unwrap();
        let mut ws = Workspace::parse("{}").unwrap();
        ws.companion.claude_command = format!("sh {} --model x", stand.display());
        let hooks = d.path().join("hooks.json");
        let script = launcher_script(&ws, &command(), d.path(), &hooks, &prompt, &endpoint());
        let path = d.path().join("launch.sh");
        std::fs::write(&path, &script).unwrap();
        let out = Command::new("sh").arg(&path).output().unwrap();
        assert!(out.status.success(), "{out:?}\n{script}");
        let argv: Vec<String> = String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        let mcp = d.path().join(MCP_CONFIG_FILE).display().to_string();
        assert_eq!(
            argv[..7],
            [
                "--model",
                "x",
                "--mcp-config",
                mcp.as_str(),
                "--allowedTools",
                ALLOWED_TOOLS,
                "--session-id"
            ],
            "{argv:?}"
        );
        assert_eq!(
            argv.last().map(String::as_str),
            Some("hi there"),
            "{argv:?}"
        );
        assert!(
            argv.iter().all(|a| !a.contains(&endpoint().token)),
            "no token on the command line"
        );
    }

    /// Plan 220 §5: the MCP config names the receiver's `/mcp`, its token and the launch.
    #[test]
    fn the_mcp_config_names_this_receiver_and_this_launch() {
        let id = Uuid::from_u128(7);
        let v: serde_json::Value =
            serde_json::from_str(&render_mcp_config(&endpoint(), id)).unwrap();
        let s = &v["mcpServers"][MCP_SERVER];
        assert_eq!(s["type"], "http");
        assert_eq!(s["url"], "http://127.0.0.1:43125/mcp");
        assert_eq!(
            s["headers"]["Authorization"],
            format!("Bearer {}", endpoint().token)
        );
        assert_eq!(s["headers"]["X-Rubato-Launch"], id.to_string());
        assert_eq!(ALLOWED_TOOLS, format!("mcp__{MCP_SERVER}"));
    }

    #[test]
    fn only_the_issues_own_branch_is_launched() {
        let mut cmd = command_with_otel(&[]);
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
        let cmd = command_with_otel(&[("a b,c=d'e", "v"), ("rubato.issue", "acme/pantry#7")]);
        let p = Path::new("/x");
        let script = launcher_script(&ws, &cmd, p, p, p, &endpoint());
        assert!(
            script.contains(
                "export OTEL_RESOURCE_ATTRIBUTES=\"${OTEL_RESOURCE_ATTRIBUTES:+$OTEL_RESOURCE_ATTRIBUTES,}\"'a_b_c_d_e=v,rubato.issue=acme/pantry#7'\n"
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
