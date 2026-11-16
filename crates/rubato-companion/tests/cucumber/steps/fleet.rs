//! Steps for `features/fleet.feature` (ADR 0072, plan 134).

use crate::world::*;
use cucumber::{given, then, when};
use rubato_protocol::fleet::LEAD_HEADER;
use rubato_protocol::{
    CompanionMessage, FleetOutcome, FleetRequest, FleetResult, LaunchRole, ServerMessage,
};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

#[given("the companion's orchestrator flag is on")]
async fn given_orchestrator(w: &mut RubatoWorld) {
    w.workspace.companion.orchestrator = true;
}

#[when(
    expr = "the server sends a lead launch for user {string} on issue {int} of {string} with slug {string}"
)]
async fn when_lead_launch(
    w: &mut RubatoWorld,
    user: String,
    issue: u64,
    repo: String,
    slug: String,
) {
    let mut cmd = w.launch_cmd(&user, &repo, issue, &slug);
    cmd.role = LaunchRole::Lead;
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd));
}

/// The launch's `mcp.json`, from the launch dir the companion wrote.
fn mcp_config(w: &RubatoWorld) -> Value {
    let id = w.last_launch.as_ref().unwrap().id;
    let p = w
        .state_dir
        .join("launches")
        .join(id.to_string())
        .join("mcp.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[then("the lead's MCP config carries its lead secret, and a worker's would not")]
async fn then_secret(w: &mut RubatoWorld) {
    let c = mcp_config(w);
    let headers = &c["mcpServers"]["rubato-companion"]["headers"];
    let secret = headers[LEAD_HEADER].as_str().expect("a lead secret");
    assert!(secret.len() >= 32);
    // The secret is in leads.json (0600), not in launches.json.
    let launches = std::fs::read_to_string(w.state_dir.join("launches.json")).unwrap();
    assert!(!launches.contains(secret), "never in launches.json");
    use std::os::unix::fs::PermissionsExt;
    let leads = w.state_dir.join("leads.json");
    assert_eq!(
        std::fs::metadata(&leads).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[when(
    expr = "the lead asks fleet_message to a worker through its MCP config and the server answers {string}"
)]
async fn when_fleet_message(w: &mut RubatoWorld, outcome: String) {
    assert_eq!(outcome, "ok");
    let c = mcp_config(w);
    let server = c["mcpServers"]["rubato-companion"].clone();
    let worker = Uuid::new_v4();
    let call = tokio::spawn(async move {
        let mut req = reqwest::Client::new().post(server["url"].as_str().unwrap());
        for (k, v) in server["headers"].as_object().unwrap() {
            req = req.header(k, v.as_str().unwrap());
        }
        req.json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "fleet_message", "arguments": {"launch_id": worker.to_string(), "text": "run the tests"}}}))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    });
    let got: FleetRequest = w
        .server()
        .wait_for(Duration::from_secs(15), |m| match m {
            CompanionMessage::FleetRequest(r) => Some(r.clone()),
            _ => None,
        })
        .await
        .expect("a fleet request");
    w.server()
        .push(ServerMessage::FleetResult(Box::new(FleetResult {
            request_id: got.request_id,
            lead_launch_id: got.lead_launch_id,
            outcome: FleetOutcome::Ok,
            reason: None,
            message: None,
            fleet: None,
            launch_id: None,
            message_id: Some(Uuid::from_u128(9)),
        })));
    let res = call.await.unwrap();
    let body: Value =
        serde_json::from_str(res["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["outcome"], "ok", "{body}");
    assert_eq!(
        got.lead_launch_id,
        w.last_launch.as_ref().unwrap().id,
        "the lead's own launch"
    );
    w.fleet_request_checked = true;
}

#[then("the fleet request named the lead's own launch")]
async fn then_named(w: &mut RubatoWorld) {
    assert!(
        w.fleet_request_checked,
        "checked when the server received it"
    );
}
