//! How Claude Code is authenticated on this machine (ADR 0076): the kind only, worked out from
//! which variables are set and whether the user's Claude settings name an `apiKeyHelper`. No
//! value is ever read into a string that leaves this module, logged, or sent. The label describes
//! the companion's own environment; an interactive pane inherits the tmux server's, which can
//! differ.

use std::path::Path;

/// The kinds, as `claude_auth:<kind>` and `rubato.claude.auth` name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeAuth {
    /// `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`, or an `apiKeyHelper`: billed per token.
    ApiKey,
    Bedrock,
    Vertex,
    /// The developer's own Claude login.
    Login,
}

impl ClaudeAuth {
    pub fn name(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Bedrock => "bedrock",
            Self::Vertex => "vertex",
            Self::Login => "login",
        }
    }
}

fn set(get: &dyn Fn(&str) -> Option<String>, name: &str) -> bool {
    get(name).is_some_and(|v| !v.trim().is_empty())
}

fn truthy(get: &dyn Fn(&str) -> Option<String>, name: &str) -> bool {
    get(name).is_some_and(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes"))
}

/// The kind from an environment getter and the user's Claude settings file.
pub fn detect_with(get: &dyn Fn(&str) -> Option<String>, settings: &Path) -> ClaudeAuth {
    if truthy(get, "CLAUDE_CODE_USE_BEDROCK") {
        return ClaudeAuth::Bedrock;
    }
    if truthy(get, "CLAUDE_CODE_USE_VERTEX") {
        return ClaudeAuth::Vertex;
    }
    if set(get, "ANTHROPIC_API_KEY") || set(get, "ANTHROPIC_AUTH_TOKEN") {
        return ClaudeAuth::ApiKey;
    }
    // Only whether the key exists; its value (a command) is not kept.
    let helper = std::fs::read_to_string(settings)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v.get("apiKeyHelper").is_some());
    if helper {
        ClaudeAuth::ApiKey
    } else {
        ClaudeAuth::Login
    }
}

/// The kind for this process: its environment and `~/.claude/settings.json`.
pub fn detect() -> ClaudeAuth {
    let settings = std::env::var("HOME")
        .map(|h| Path::new(&h).join(".claude/settings.json"))
        .unwrap_or_default();
    detect_with(&|k| std::env::var(k).ok(), &settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const MARKER: &str = "sk-ant-MARKER-must-never-appear";

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn each_variable_gives_its_kind_and_no_value_leaks() {
        let d = tempfile::tempdir().unwrap();
        let none = d.path().join("absent.json");
        let cases: Vec<(Vec<(&str, &str)>, ClaudeAuth)> = vec![
            (vec![], ClaudeAuth::Login),
            (vec![("ANTHROPIC_API_KEY", MARKER)], ClaudeAuth::ApiKey),
            (vec![("ANTHROPIC_AUTH_TOKEN", MARKER)], ClaudeAuth::ApiKey),
            (vec![("ANTHROPIC_API_KEY", " ")], ClaudeAuth::Login),
            (
                vec![
                    ("CLAUDE_CODE_USE_BEDROCK", "1"),
                    ("ANTHROPIC_API_KEY", MARKER),
                ],
                ClaudeAuth::Bedrock,
            ),
            (vec![("CLAUDE_CODE_USE_VERTEX", "true")], ClaudeAuth::Vertex),
            (vec![("CLAUDE_CODE_USE_VERTEX", "0")], ClaudeAuth::Login),
        ];
        for (vars, want) in &cases {
            let got = detect_with(&env(vars), &none);
            assert_eq!(got, *want, "{vars:?}");
            // What leaves the module is the kind's name, never the value.
            assert!(!got.name().contains("MARKER"));
            assert!(!format!("{got:?}").contains("MARKER"));
        }
        println!("checked {} environments", cases.len());
    }

    #[test]
    fn an_api_key_helper_in_the_settings_is_an_api_key() {
        let d = tempfile::tempdir().unwrap();
        let s = d.path().join("settings.json");
        std::fs::write(&s, format!(r#"{{"apiKeyHelper": "echo {MARKER}"}}"#)).unwrap();
        let got = detect_with(&env(&[]), &s);
        assert_eq!(got, ClaudeAuth::ApiKey);
        assert_eq!(got.name(), "api_key");
        std::fs::write(&s, r#"{"model": "x"}"#).unwrap();
        assert_eq!(detect_with(&env(&[]), &s), ClaudeAuth::Login);
    }
}
