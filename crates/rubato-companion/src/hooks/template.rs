//! The hooks file a launched Claude Code session gets: the skill ships the template, a copy is
//! embedded here, and the endpoint port and token are filled in per start.

use crate::kernel::config::HookEndpoint;

/// Render the hook template for this endpoint. The template ships with the skill; a copy is embedded.
pub const HOOKS_TEMPLATE: &str = include_str!("../../../../skills/rubato/hooks.json");

pub fn render_hooks(template: &str, ep: &HookEndpoint) -> anyhow::Result<String> {
    let out = template
        .replace("{port}", &ep.port.to_string())
        .replace("{token}", &ep.token);
    serde_json::from_str::<serde_json::Value>(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_embedded_template() {
        let out = render_hooks(
            HOOKS_TEMPLATE,
            &HookEndpoint {
                port: 4321,
                token: "abcdefghijklmnopqrstuvwxyz012345".into(),
            },
        )
        .unwrap();
        assert!(out.contains("http://127.0.0.1:4321/hook"));
        assert!(!out.contains("{token}"));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["hooks"]["Notification"].is_array());
    }

    /// Every hook command, as the rendered file holds it.
    fn commands(rendered: &str) -> Vec<String> {
        let v: serde_json::Value = serde_json::from_str(rendered).unwrap();
        v["hooks"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|groups| groups.as_array().unwrap().iter())
            .flat_map(|g| g["hooks"].as_array().unwrap().iter())
            .map(|h| h["command"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Plan 122 §D1: each command sends the launch id from `launch.sh`'s `RUBATO_LAUNCH_ID`.
    /// The shell runs the command, so the header is double-quoted (expanded) and the bearer
    /// stays single-quoted. A stand-in `curl` prints its arguments, so this checks what the
    /// shell really passes, set and unset.
    #[test]
    fn every_hook_sends_the_launch_id_the_shell_expands() {
        let ep = HookEndpoint {
            port: 4321,
            token: "abcdefghijklmnopqrstuvwxyz012345".into(),
        };
        let out = render_hooks(HOOKS_TEMPLATE, &ep).unwrap();
        let commands = commands(&out);
        assert_eq!(commands.len(), 9, "{commands:?}");
        for c in &commands {
            assert!(
                c.contains("-H \"X-Rubato-Launch: ${RUBATO_LAUNCH_ID:-}\""),
                "{c}"
            );
            assert!(
                c.contains(&format!("-H 'Authorization: Bearer {}'", ep.token)),
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
        assert!(
            set.lines()
                .any(|l| l == format!("Authorization: Bearer {}", ep.token)),
            "{set}"
        );
        let unset = run(None);
        assert!(unset.lines().any(|l| l == "X-Rubato-Launch: "), "{unset}");
    }
}
