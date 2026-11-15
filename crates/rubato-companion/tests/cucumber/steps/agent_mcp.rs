//! Steps for `features/agent-mcp.feature` (ADR 0071, plan 220 §5).

use crate::world::*;
use cucumber::then;
use serde_json::{Value, json};

/// The `--mcp-config` path the stand-in claude was started with.
async fn mcp_config_path(w: &RubatoWorld) -> String {
    let log = standin_log(w).await;
    let args = log.lines().find(|l| l.starts_with("ARGS:")).unwrap();
    args.split_whitespace()
        .skip_while(|a| *a != "--mcp-config")
        .nth(1)
        .expect("--mcp-config <file>")
        .to_owned()
}

/// Call `tools/call get_assignment` as the config says, without the launch header when
/// `with_launch` is false.
async fn get_assignment(w: &RubatoWorld, with_launch: bool) -> Value {
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(mcp_config_path(w).await).unwrap()).unwrap();
    let server = &config["mcpServers"]["rubato-companion"];
    assert_eq!(server["type"], "http");
    let mut req = reqwest::Client::new().post(server["url"].as_str().unwrap());
    for (k, v) in server["headers"].as_object().unwrap() {
        if with_launch || k != "X-Rubato-Launch" {
            req = req.header(k, v.as_str().unwrap());
        }
    }
    let res = req
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "get_assignment", "arguments": {}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "a known token is never 401");
    res.json::<Value>().await.unwrap()["result"].clone()
}

#[then("the stand-in claude received an MCP config, mode 0600, beside its settings file")]
async fn then_mcp_config(w: &mut RubatoWorld) {
    use std::os::unix::fs::PermissionsExt;
    let path = mcp_config_path(w).await;
    let p = std::path::Path::new(&path);
    assert_eq!(p.file_name().unwrap(), "mcp.json");
    assert!(p.with_file_name("hooks.json").exists(), "beside hooks.json");
    assert_eq!(
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777,
        0o600,
        "it holds the token"
    );
    let log = standin_log(w).await;
    let token = &w.handles.as_ref().unwrap().hook_endpoint.token;
    let args = log.lines().find(|l| l.starts_with("ARGS:")).unwrap();
    assert!(
        !args.contains(token.as_str()),
        "no token on the command line"
    );
    assert!(
        args.contains("--allowedTools mcp__rubato-companion"),
        "{args}"
    );
}

#[then(
    expr = "asked through that config, the MCP server answers get_assignment with issue {int} of {string}"
)]
async fn then_assignment(w: &mut RubatoWorld, issue: u64, repo: String) {
    let result = get_assignment(w, true).await;
    assert_eq!(result["isError"], false, "{result}");
    let a: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(a["issue"], json!({"repo": repo, "number": issue}), "{a}");
    assert_eq!(
        a["launch_id"],
        w.last_launch.as_ref().unwrap().id.to_string()
    );
}

#[then("asked without the launch header, the MCP server refuses get_assignment")]
async fn then_refused(w: &mut RubatoWorld) {
    let result = get_assignment(w, false).await;
    assert_eq!(result["isError"], true, "{result}");
}
