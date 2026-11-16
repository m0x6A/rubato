//! The fleet tools on the receiver's real router (ADR 0072, plan 134 design §3–§4): offered only
//! to a lead launch with the companion's flag on, a server that announced `fleet` and the lead's
//! own secret; a call without the secret is refused `lead_secret`; a call is one `FleetRequest`
//! for the calling launch, answered by the server's `FleetResult`; with the link down nothing is
//! sent.

use super::*;
use crate::hooks::{HookServer, LAUNCH_HEADER};
use crate::kernel::telemetry::metric_reader;
use chrono::{DateTime, TimeZone};
use rubato_protocol::fleet::LEAD_HEADER;
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::{
    FleetAction, FleetOutcome, FleetResult, IssueRef, LaunchCommand, LaunchRole, PlanningMode,
    capability,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

const LEAD: u128 = 70;
const WORKER: u128 = 71;

fn command(n: u128, role: LaunchRole) -> LaunchCommand {
    LaunchCommand {
        id: Uuid::from_u128(n),
        user: "m0x6A".into(),
        issue: IssueRef::new("m0x6A/rubato", n as u64),
        repo_url: "https://github.com/m0x6A/rubato".into(),
        branch: format!("issue/{n}-x"),
        base_ref: "origin/main".into(),
        node_ids: vec![],
        spend_cap_usd: 25.0,
        attempt_limit: 3,
        attempt: 1,
        session_id: Uuid::from_u128(1000 + n),
        prompt: "lead".into(),
        otel_resource: BTreeMap::new(),
        map_version: None,
        plan_path: None,
        draft: None,
        planning: PlanningMode::Git,
        role,
        lead_launch_id: None,
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap()
}

struct World {
    st: Arc<AgentMcp>,
    reader: metric_reader::Reader,
    base: String,
    token: String,
    secret: String,
    out: mpsc::UnboundedReceiver<CompanionMessage>,
    caps: Arc<std::sync::Mutex<Vec<String>>>,
    link: tokio_watch::Sender<bool>,
    _dir: tempfile::TempDir,
}

async fn world(orchestrator: bool) -> World {
    let dir = tempfile::tempdir().unwrap();
    let mut reg = Registry::new(Uuid::from_u128(42), "m0x6A");
    for (n, role) in [(LEAD, LaunchRole::Lead), (WORKER, LaunchRole::Worker)] {
        let cmd = command(n, role);
        reg.begin_launch(&cmd, now());
        reg.end_launch();
        reg.apply_launch(
            &cmd,
            &format!("%{n}"),
            &format!("rubato-{n}"),
            Path::new("/w/7"),
            now(),
        );
    }
    let secret = new_lead_secret(dir.path(), Uuid::from_u128(LEAD)).unwrap();
    let registry = Arc::new(Mutex::new(reg));
    let (reader, metrics) = metric_reader::start();
    let caps = Arc::new(std::sync::Mutex::new(vec![capability::FLEET.to_owned()]));
    let (out_tx, out) = mpsc::unbounded_channel();
    let (link, link_rx) = tokio_watch::channel(true);
    let st = AgentMcp::new(
        registry,
        Arc::new(metrics),
        caps.clone(),
        out_tx,
        link_rx,
        orchestrator,
        dir.path().to_owned(),
    )
    .await;
    let (htx, _hooks) = mpsc::unbounded_channel();
    let (dtx, _drx) = mpsc::unbounded_channel();
    let server = HookServer::bind(htx, dtx).await.unwrap();
    let base = format!("http://127.0.0.1:{}", server.endpoint.port);
    let token = server.endpoint.token.clone();
    let (open, guarded) = routers(st.clone());
    tokio::spawn(server.serve_with(open, guarded));
    World {
        st,
        reader,
        base,
        token,
        secret,
        out,
        caps,
        link,
        _dir: dir,
    }
}

impl World {
    async fn rpc(&self, launch: u128, secret: Option<&str>, msg: Value) -> Value {
        let mut req = reqwest::Client::new()
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&self.token)
            .header(LAUNCH_HEADER, Uuid::from_u128(launch).to_string())
            .json(&msg);
        if let Some(s) = secret {
            req = req.header(LEAD_HEADER, s);
        }
        let res = req.send().await.unwrap();
        assert_eq!(res.status(), 200);
        res.json::<Value>().await.unwrap()
    }

    async fn tools(&self, launch: u128, secret: Option<&str>) -> Vec<String> {
        let v = self
            .rpc(
                launch,
                secret,
                json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            )
            .await;
        v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn call(&self, launch: u128, secret: Option<&str>, tool: &str, args: Value) -> Value {
        let v = self
            .rpc(launch, secret, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": tool, "arguments": args}}))
            .await;
        let r = &v["result"];
        serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn calls(&self, outcome: &str, reason: &str) -> u64 {
        self.reader
            .points(metric::MCP_CALLS)
            .iter()
            .filter(|p| p.attrs[attr::OUTCOME] == outcome && p.attrs[attr::DROP_REASON] == reason)
            .map(|p| p.value)
            .sum()
    }
}

fn fleet_names() -> Vec<String> {
    FLEET_TOOLS
        .iter()
        .map(|(n, _, _)| (*n).to_owned())
        .collect()
}

#[tokio::test]
async fn only_a_lead_with_the_flag_the_server_and_its_secret_sees_the_fleet_tools() {
    let w = world(true).await;
    let lead = w.tools(LEAD, Some(&w.secret)).await;
    for t in fleet_names() {
        assert!(lead.contains(&t), "{t} in {lead:?}");
    }
    assert_eq!(lead.len(), 3 + 4);
    let no_secret = w.tools(LEAD, None).await;
    assert_eq!(no_secret.len(), 3, "without its secret: {no_secret:?}");
    assert_eq!(w.tools(LEAD, Some("wrong")).await.len(), 3);
    assert_eq!(
        w.tools(WORKER, Some(&w.secret)).await.len(),
        3,
        "a worker never"
    );
    // The server did not announce `fleet`.
    w.caps.lock().unwrap().clear();
    assert_eq!(w.tools(LEAD, Some(&w.secret)).await.len(), 3);
    // The companion's own flag off.
    let off = world(false).await;
    assert_eq!(off.tools(LEAD, Some(&off.secret)).await.len(), 3);
    println!("checked 6 tool lists");
}

#[tokio::test]
async fn a_call_without_the_lead_secret_is_refused_and_counted() {
    let mut w = world(true).await;
    // A worker's own launch header naming itself, and the lead's id without its secret.
    let r = w.call(WORKER, None, "fleet_list", json!({})).await;
    assert_eq!(r["refused"], "not_a_lead", "{r}");
    let r = w.call(LEAD, None, "fleet_list", json!({})).await;
    assert_eq!(r["refused"], "lead_secret", "{r}");
    let r = w
        .call(
            LEAD,
            Some("not-it"),
            "fleet_stop",
            json!({"launch_id": Uuid::nil().to_string()}),
        )
        .await;
    assert_eq!(r["refused"], "lead_secret", "{r}");
    assert_eq!(w.calls("refused", "lead_secret"), 2);
    assert!(w.out.try_recv().is_err(), "nothing was sent to the server");
}

#[tokio::test]
async fn the_server_answers_the_waiting_call() {
    let w = world(true).await;
    let secret = w.secret.clone();
    let base = w.base.clone();
    let token = w.token.clone();
    let st = w.st.clone();
    let mut out = w.out;
    let answer = tokio::spawn(async move {
        let Some(CompanionMessage::FleetRequest(r)) = out.recv().await else {
            panic!("a fleet request");
        };
        assert_eq!(
            r.lead_launch_id,
            Uuid::from_u128(LEAD),
            "the calling launch"
        );
        assert!(matches!(r.action, FleetAction::Message { ref text, .. } if text == "go"));
        st.on_fleet_result(FleetResult {
            request_id: r.request_id,
            lead_launch_id: r.lead_launch_id,
            outcome: FleetOutcome::Ok,
            reason: None,
            message: None,
            fleet: None,
            launch_id: None,
            message_id: Some(Uuid::from_u128(5)),
        });
    });
    let res = reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .bearer_auth(&token)
        .header(LAUNCH_HEADER, Uuid::from_u128(LEAD).to_string())
        .header(LEAD_HEADER, &secret)
        .json(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "fleet_message", "arguments": {"launch_id": Uuid::from_u128(WORKER).to_string(), "text": "go"}}}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    answer.await.unwrap();
    let body: Value =
        serde_json::from_str(res["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["outcome"], "ok", "{body}");
    assert_eq!(body["message_id"], Uuid::from_u128(5).to_string());
    assert_eq!(res["result"]["isError"], false);
}

#[tokio::test]
async fn with_the_link_down_nothing_is_sent() {
    let mut w = world(true).await;
    w.link.send(false).unwrap();
    let secret = w.secret.clone();
    let r = w.call(LEAD, Some(&secret), "fleet_list", json!({})).await;
    assert_eq!(r["refused"], "link_down", "{r}");
    assert!(w.out.try_recv().is_err());
}
