//! `rubato-companion mcp` (ADR 0086): an MCP server over stdio for a session a person started
//! by hand. Claude Code starts it; for each message it reads the loopback receiver's port and
//! token from the state directory and posts to `/mcp/read`. So no config file holds a token or
//! a port, and a companion that restarted on another port is found again at the next message.
//!
//! It holds no state and offers nothing of its own: the tools are the companion's read-only
//! graph tools. In a session Rubato launched, and in a room's fork turn, it lists no tools: the
//! first already has them under `rubato-companion`, and a fork gets no Rubato MCP (ADR 0071).

use super::read::{INSTRUCTIONS, REPO_HEADER, list, read_tools};
use super::rpc::PROTOCOL_VERSIONS;
use crate::kernel::config::HookEndpoint;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use crate::kernel::claude::FORK_TURN_ENV;

/// What the bridge was started with.
#[derive(Debug, Clone, Default)]
pub struct Bridge {
    pub state_dir: PathBuf,
    /// A launched session or a fork turn: answer, and offer nothing.
    pub stand_down: bool,
    /// `owner/name` of the working directory's repository, when it has an `origin`.
    pub repo: Option<String>,
}

/// `owner/name` from a git remote URL, for GitHub's two spellings and anything shaped like them.
pub fn repo_of_remote(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let mut parts = path.rsplit('/');
    let (name, owner) = (parts.next()?, parts.next()?);
    let ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (ok(owner) && ok(name)).then(|| format!("{owner}/{name}"))
}

/// Whether this environment is a session Rubato launched, or a room's fork turn: the bridge
/// then offers nothing.
pub fn stands_down(get: impl Fn(&str) -> Option<String>) -> bool {
    let set = |k: &str| get(k).is_some_and(|v| !v.is_empty());
    set("RUBATO_LAUNCH_ID") || set(FORK_TURN_ENV)
}

impl Bridge {
    /// From the process's environment and working directory.
    pub fn from_env(state_dir: &Path) -> Self {
        let repo = std::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| repo_of_remote(&String::from_utf8_lossy(&o.stdout)));
        Self {
            state_dir: state_dir.to_path_buf(),
            stand_down: stands_down(|k| std::env::var(k).ok()),
            repo,
        }
    }

    /// What the bridge says when it cannot, or must not, ask the companion.
    fn local(&self, msg: &Value, why_no_call: (&str, &str)) -> Option<Value> {
        let id = msg.get("id")?.clone();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => {
                let asked = msg
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| **v == asked)
                    .unwrap_or(&PROTOCOL_VERSIONS[0]);
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "rubato-graph", "version": crate::kernel::VERSION},
                    "instructions": INSTRUCTIONS,
                })
            }
            "ping" => json!({}),
            "tools/list" if self.stand_down => json!({"tools": []}),
            // The companion is not running: the tools are still named, so Claude Code keeps
            // the server and a later call finds the companion.
            "tools/list" => list(&read_tools()),
            "tools/call" => {
                let (reason, message) = why_no_call;
                json!({"content": [{"type": "text", "text": json!({"refused": reason, "message": message}).to_string()}], "isError": true})
            }
            _ => {
                return Some(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
                );
            }
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    /// One message in, at most one message out. A notification has no answer.
    pub async fn handle(&self, client: &reqwest::Client, line: &str) -> Option<String> {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}).to_string());
        };
        if self.stand_down {
            return self
                .local(&msg, ("stood_down", "this session has Rubato's tools from its own companion already, or is a room's fork turn"))
                .map(|v| v.to_string());
        }
        let down = (
            "companion_down",
            "no rubato-companion is running on this machine: start it with `rubato-companion run`",
        );
        let Some(endpoint) = HookEndpoint::load(&self.state_dir) else {
            return self.local(&msg, down).map(|v| v.to_string());
        };
        let mut req = client
            .post(format!("http://127.0.0.1:{}/mcp/read", endpoint.port))
            .bearer_auth(&endpoint.token)
            .header("content-type", "application/json")
            .body(line.to_owned());
        if let Some(repo) = &self.repo {
            req = req.header(REPO_HEADER, repo);
        }
        match req.send().await {
            Ok(res) if res.status().as_u16() == 202 => None,
            Ok(res) if res.status().is_success() => {
                res.text().await.ok().map(|t| t.trim().to_owned())
            }
            // Not reached, or not a companion of this version: answer as when none runs.
            _ => self.local(&msg, down).map(|v| v.to_string()),
        }
    }

    /// Serve stdin to stdout, one JSON-RPC message a line, until stdin closes.
    pub async fn run(self) -> anyhow::Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        let mut out = tokio::io::stdout();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            if let Some(answer) = self.handle(&client, &line).await {
                out.write_all(answer.as_bytes()).await?;
                out.write_all(b"\n").await?;
                out.flush().await?;
            }
        }
        Ok(())
    }
}
