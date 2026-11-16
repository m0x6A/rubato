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

/// One `tools/call` through the launch's `mcp.json`, with every header it names.
async fn call(w: &RubatoWorld, tool: &str, args: Value) -> Value {
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(mcp_config_path(w).await).unwrap()).unwrap();
    let server = &config["mcpServers"]["rubato-companion"];
    let mut req = reqwest::Client::new().post(server["url"].as_str().unwrap());
    for (k, v) in server["headers"].as_object().unwrap() {
        req = req.header(k, v.as_str().unwrap());
    }
    let res = req
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let result = res.json::<Value>().await.unwrap()["result"].clone();
    assert_eq!(result["isError"], false, "{result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[cucumber::given(expr = "the test server also announces {string}")]
async fn given_announces(w: &mut RubatoWorld, cap: String) {
    w.server
        .as_ref()
        .unwrap()
        .capabilities
        .lock()
        .unwrap()
        .push(cap);
}

#[cucumber::when(expr = "the launched session reports {string} on {string} through its MCP config")]
async fn when_reports(w: &mut RubatoWorld, doing: String, part: String) {
    let a = call(w, "report_status", json!({"doing": doing, "parts": [part]})).await;
    assert_eq!(a["accepted"], true, "{a}");
}

#[then(expr = "the server receives an agent report {string} for that launch")]
async fn then_report(w: &mut RubatoWorld, doing: String) {
    let launch = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server
        .as_ref()
        .unwrap()
        .wait_for(std::time::Duration::from_secs(15), |m| match m {
            rubato_protocol::CompanionMessage::AgentReport(r) if r.launch_id == launch => {
                Some(r.clone())
            }
            _ => None,
        })
        .await
        .expect("an agent report within the drain interval");
    assert_eq!(got.doing, doing);
    assert_eq!(got.parts, vec!["rubato-companion".to_owned()]);
}

#[then("the server has received no agent report")]
async fn then_no_report(w: &mut RubatoWorld) {
    let n = w
        .server
        .as_ref()
        .unwrap()
        .count(|m| matches!(m, rubato_protocol::CompanionMessage::AgentReport(_)))
        .await;
    assert_eq!(n, 0);
}

#[cucumber::when(
    expr = "the server sends a plan status for that launch whose branch plan is {string}"
)]
async fn when_plan_status(w: &mut RubatoWorld, sha: String) {
    let launch = w.last_launch.as_ref().unwrap().id;
    w.server
        .as_ref()
        .unwrap()
        .push(rubato_protocol::ServerMessage::PlanStatus(Box::new(
            rubato_protocol::PlanStatus {
                launch_id: launch,
                as_of: chrono::Utc::now(),
                draft: None,
                review: None,
                approval: None,
                branch: Some(rubato_protocol::BranchStatus {
                    sha,
                    carries_approval: false,
                }),
            },
        )));
}

#[then(
    expr = "asked through that config, plan_status answers the branch plan {string} with the link up"
)]
async fn then_plan_status(w: &mut RubatoWorld, sha: String) {
    for _ in 0..50 {
        let a = call(w, "plan_status", json!({})).await;
        if a["status"] == "known" {
            assert_eq!(a["plan_status"]["branch"]["sha"], sha.as_str(), "{a}");
            assert_eq!(a["link_up"], true, "{a}");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("plan_status never became known");
}
