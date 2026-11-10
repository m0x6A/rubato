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
}
