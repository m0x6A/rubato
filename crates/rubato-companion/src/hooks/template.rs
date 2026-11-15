//! The hooks file a launched Claude Code session gets: the skill ships the template, a copy is
//! embedded here, and the endpoint port and the path of the header file that holds the token are
//! filled in per start (ADR 0063). No rendered command carries the token: argv is readable by
//! every local user.

use std::path::Path;

/// Render the hook template for this endpoint. The template ships with the skill; a copy is embedded.
pub const HOOKS_TEMPLATE: &str = include_str!("../../../../skills/rubato/hooks.json");

/// Fill `{port}` and `{header_file}`. The path is shell-quoted (the command runs under `sh -c`)
/// and then JSON-escaped (it sits inside a JSON string).
pub fn render_hooks(template: &str, port: u16, header_file: &Path) -> anyhow::Result<String> {
    let quoted = sh_quote(&header_file.to_string_lossy());
    let json = serde_json::to_string(&quoted)?;
    let out = template
        .replace("{port}", &port.to_string())
        .replace("{header_file}", &json[1..json.len() - 1]);
    serde_json::from_str::<serde_json::Value>(&out)?;
    Ok(out)
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "abcdefghijklmnopqrstuvwxyz012345";

    fn commands(rendered: &str) -> Vec<String> {
        let v: serde_json::Value = serde_json::from_str(rendered).unwrap();
        let mut out = vec![];
        for entries in v["hooks"].as_object().unwrap().values() {
            for e in entries.as_array().unwrap() {
                for h in e["hooks"].as_array().unwrap() {
                    out.push(h["command"].as_str().unwrap().to_owned());
                }
            }
        }
        out
    }

    #[test]
    fn renders_embedded_template_without_the_token() {
        let out = render_hooks(HOOKS_TEMPLATE, 4321, Path::new("/state/hook-auth.header")).unwrap();
        assert!(out.contains("http://127.0.0.1:4321/hook"));
        assert!(
            !out.contains("{token}") && !out.contains("{header_file}") && !out.contains("{port}")
        );
        assert!(!out.contains("Bearer"), "{out}");
        let cmds = commands(&out);
        assert_eq!(cmds.len(), 9, "one command per hook event");
        for c in &cmds {
            assert!(!c.contains(TOKEN), "{c}");
            assert!(c.contains("-H @'/state/hook-auth.header'"), "{c}");
        }
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["hooks"]["Notification"].is_array());
    }

    /// A path with a space, a quote and a backslash reaches curl as one argument, unchanged.
    #[test]
    fn header_path_survives_sh_and_json() {
        let path = r#"/tmp/it's a "dir"\x/hook-auth.header"#;
        let out = render_hooks(HOOKS_TEMPLATE, 1, Path::new(path)).unwrap();
        let cmd = &commands(&out)[0];
        // Replace curl with printf to see the argument the shell hands it.
        let probe = cmd.replacen("curl", "printf '%s\\n'", 1);
        let got = std::process::Command::new("sh")
            .args(["-c", &probe])
            .output()
            .unwrap();
        let args = String::from_utf8(got.stdout).unwrap();
        assert!(args.lines().any(|l| l == format!("@{path}")), "{args}");
    }

    /// Plan 122 §D1: each command sends the launch id from `launch.sh`'s `RUBATO_LAUNCH_ID`.
    /// The shell runs the command, so that header is double-quoted (expanded). The token travels
    /// in the header file (ADR 0063), so curl gets `@<file>` and never the token. A stand-in
    /// `curl` prints its arguments, so this checks what the shell really passes, set and unset.
    #[test]
    fn every_hook_sends_the_launch_id_the_shell_expands() {
        let header = "/state/hook-auth.header";
        let out = render_hooks(HOOKS_TEMPLATE, 4321, Path::new(header)).unwrap();
        let commands = commands(&out);
        assert_eq!(commands.len(), 9, "{commands:?}");
        for c in &commands {
            assert!(
                c.contains("-H \"X-Rubato-Launch: ${RUBATO_LAUNCH_ID:-}\""),
                "{c}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        // Written by a child `sh`, so no fork in a concurrent test holds it open for writing
        // when it runs ("text file busy", see `launch::arm::tests`).
        let written = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg("#!/bin/sh\nfor a in \"$@\"; do echo \"$a\"; done\n")
            .arg(dir.path().join("curl"))
            .status()
            .unwrap();
        assert!(written.success());
        let path = format!(
            "{}:{}",
            dir.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let run = |launch: Option<&str>| {
            let mut sh = std::process::Command::new("sh");
            sh.arg("-c").arg(&commands[0]).env("PATH", &path);
            match launch {
                Some(id) => sh.env("RUBATO_LAUNCH_ID", id),
                None => sh.env_remove("RUBATO_LAUNCH_ID"),
            };
            let o = sh.output().unwrap();
            assert!(o.status.success());
            String::from_utf8(o.stdout).unwrap()
        };
        let id = "6f1c1b5e-1d2a-4c3b-9e8f-7a6b5c4d3e2f";
        let set = run(Some(id));
        assert!(
            set.lines().any(|l| l == format!("X-Rubato-Launch: {id}")),
            "{set}"
        );
        assert!(set.lines().any(|l| l == format!("@{header}")), "{set}");
        assert!(!set.contains("Bearer"), "{set}");
        let unset = run(None);
        assert!(unset.lines().any(|l| l == "X-Rubato-Launch: "), "{unset}");
    }
}
