//! Plan 220 §4: every test goes through the receiver's merged router (`serve_with`), so the token
//! layer, the launch header and the body limit are the real ones.

use super::*;
use crate::hooks::{HookServer, LAUNCH_HEADER};
use crate::kernel::telemetry::metric_reader;
use chrono::{DateTime, Duration as Days, TimeZone};
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::{
    BranchStatus, IssueRef, LaunchCommand, PlanningMode, SessionStatus, capability,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

fn command(n: u128) -> LaunchCommand {
    LaunchCommand {
        id: Uuid::from_u128(n),
        user: "m0x6A".into(),
        issue: IssueRef::new("m0x6A/rubato", n as u64),
        repo_url: "https://github.com/m0x6A/rubato".into(),
        branch: format!("issue/{n}-agent"),
        base_ref: "origin/main".into(),
        node_ids: vec!["rubato-companion".into()],
        spend_cap_usd: 25.0,
        attempt_limit: 3,
        attempt: 1,
        session_id: Uuid::from_u128(1000 + n),
        prompt: "plan it".into(),
        otel_resource: BTreeMap::new(),
        map_version: Some("abc1234".into()),
        plan_path: Some(format!("docs/plans/{n}.md")),
        draft: None,
        planning: PlanningMode::Git,
        role: Default::default(),
        lead_launch_id: None,
    }
}

struct World {
    st: Arc<AgentMcp>,
    reader: metric_reader::Reader,
    base: String,
    token: String,
    out: mpsc::UnboundedReceiver<CompanionMessage>,
    hooks: mpsc::UnboundedReceiver<rubato_protocol::HookEvent>,
    caps: Arc<std::sync::Mutex<Vec<String>>>,
    link: tokio_watch::Sender<bool>,
}

/// A registry with launch 7 started (and launch 8 still pending when `pending`), the receiver
/// serving this slice's routers.
async fn world(now: DateTime<Utc>, pending: bool) -> World {
    let mut reg = Registry::new(Uuid::from_u128(42), "m0x6A");
    let cmd = command(7);
    reg.begin_launch(&cmd, now);
    reg.end_launch();
    reg.apply_launch(&cmd, "%7", "rubato-7", Path::new("/w/7"), now);
    let registry = Arc::new(Mutex::new(Registry::new(Uuid::from_u128(42), "m0x6A")));
    let (reader, metrics) = metric_reader::start();
    let metrics = Arc::new(metrics);
    let caps = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (out_tx, out) = mpsc::unbounded_channel();
    // Built on an empty registry, so launch 7 does not count as restored.
    let (link, link_rx) = tokio_watch::channel(true);
    let st = AgentMcp::new(
        registry.clone(),
        metrics,
        caps.clone(),
        out_tx,
        link_rx,
        false,
        std::env::temp_dir(),
    )
    .await;
    *registry.lock().await = reg;
    if pending {
        registry.lock().await.begin_launch(&command(8), now);
    }
    let (htx, hooks) = mpsc::unbounded_channel();
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
        out,
        hooks,
        caps,
        link,
    }
}

impl World {
    async fn rpc(&self, launch: Option<u128>, msg: Value) -> reqwest::Response {
        let mut req = reqwest::Client::new()
            .post(format!("{}/mcp", self.base))
            .bearer_auth(&self.token)
            .json(&msg);
        if let Some(n) = launch {
            req = req.header(LAUNCH_HEADER, Uuid::from_u128(n).to_string());
        }
        req.send().await.unwrap()
    }

    async fn call(&self, launch: u128, tool: &str, args: Value) -> Value {
        let res = self
            .rpc(Some(launch), json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}))
            .await;
        assert_eq!(res.status(), 200);
        res.json::<Value>().await.unwrap()["result"].clone()
    }

    fn calls(&self, tool: &str, outcome: &str, reason: &str) -> u64 {
        self.reader
            .points(metric::MCP_CALLS)
            .iter()
            .filter(|p| {
                p.attrs[attr::MCP_TOOL] == tool
                    && p.attrs[attr::OUTCOME] == outcome
                    && p.attrs[attr::DROP_REASON] == reason
            })
            .map(|p| p.value)
            .sum()
    }
}

/// The JSON a tool result's text item holds.
fn body(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap()
}

#[tokio::test]
async fn get_assignment_answers_from_the_launch() {
    let w = world(Utc::now(), false).await;
    let r = w.call(7, "get_assignment", json!({})).await;
    assert_eq!(r["isError"], false);
    let a = body(&r);
    assert_eq!(a["issue"], json!({"repo": "m0x6A/rubato", "number": 7}));
    assert_eq!(a["branch"], "issue/7-agent");
    assert_eq!(a["node_ids"], json!(["rubato-companion"]));
    assert_eq!(a["spend_cap_usd"], 25.0);
    assert_eq!(a["attempt_limit"], 3);
    assert_eq!(w.calls("get_assignment", "ok", ""), 1);
}

#[tokio::test]
async fn a_pending_launch_can_initialize_before_the_launcher_returns() {
    let w = world(Utc::now(), true).await;
    let res = w
        .rpc(Some(8), json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}))
        .await;
    assert_eq!(res.status(), 200);
    let v: Value = res.json().await.unwrap();
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
}

#[tokio::test]
async fn an_unknown_launch_is_refused_without_a_401() {
    let w = world(Utc::now(), false).await;
    for launch in [None, Some(99)] {
        let res = w
            .rpc(
                launch,
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            )
            .await;
        assert_eq!(res.status(), 200, "never 401 for a known token");
        let v: Value = res.json().await.unwrap();
        assert_eq!(v["error"]["code"], rpc::NO_LAUNCH);
        let res = w
            .rpc(launch, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "get_assignment", "arguments": {}}}))
            .await;
        assert_eq!(res.status(), 200);
        let v: Value = res.json().await.unwrap();
        assert_eq!(v["result"]["isError"], true);
    }
    assert_eq!(w.calls("initialize", "refused", "no_launch"), 2);
    assert_eq!(w.calls("get_assignment", "refused", "no_launch"), 2);
}

#[tokio::test]
async fn a_call_naming_another_issue_is_refused_and_counted() {
    let w = world(Utc::now(), false).await;
    let r = w
        .call(
            7,
            "report_status",
            json!({"doing": "x", "issue": "m0x6A/rubato#8"}),
        )
        .await;
    assert_eq!(r["isError"], true);
    assert_eq!(w.calls("report_status", "refused", "identity"), 1);
    assert!(w.st.reports.peek().is_empty(), "nothing kept");
    let r = w
        .call(7, "get_assignment", json!({"issue": "M0x6A/Rubato#7"}))
        .await;
    assert_eq!(
        r["isError"], false,
        "its own issue, spelled differently, is fine"
    );
}

#[tokio::test]
async fn plan_status_is_unknown_until_pushed_and_keeps_the_newest() {
    let w = world(Utc::now(), false).await;
    assert_eq!(
        body(&w.call(7, "plan_status", json!({})).await)["status"],
        "unknown"
    );
    let status = |at: DateTime<Utc>, sha: &str| PlanStatus {
        launch_id: Uuid::from_u128(7),
        as_of: at,
        draft: None,
        review: None,
        approval: None,
        branch: Some(BranchStatus {
            sha: sha.into(),
            carries_approval: false,
        }),
    };
    w.st.on_plan_status(status(now(), "new"));
    w.st.on_plan_status(status(now() - Days::minutes(1), "old"));
    let b = body(&w.call(7, "plan_status", json!({})).await);
    assert_eq!(b["status"], "known");
    assert_eq!(b["plan_status"]["branch"]["sha"], "new");
    assert_eq!(b["link_up"], true);
}

/// Plan 220 §6: with the link down the answer says so; the agent reads it as "not known".
#[tokio::test]
async fn plan_status_says_when_the_link_is_down() {
    let w = world(Utc::now(), false).await;
    w.link.send(false).unwrap();
    let b = body(&w.call(7, "plan_status", json!({})).await);
    assert_eq!(b["link_up"], false);
}

#[tokio::test]
async fn the_transport_keeps_the_streamable_http_rules() {
    let w = world(Utc::now(), false).await;
    let c = reqwest::Client::new();
    let res = w
        .rpc(
            Some(7),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .await;
    assert_eq!(res.status(), 202);
    assert!(res.text().await.unwrap().is_empty());
    let res = w
        .rpc(
            Some(7),
            json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}),
        )
        .await;
    let v: Value = res.json().await.unwrap();
    assert_eq!(v["error"]["code"], -32601);
    assert_eq!(w.calls("unknown", "error", "unknown_method"), 1);
    let res = c
        .get(format!("{}/mcp", w.base))
        .bearer_auth(&w.token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 405);
    let res = c
        .post(format!("{}/mcp", w.base))
        .bearer_auth(&w.token)
        .header(LAUNCH_HEADER, Uuid::from_u128(7).to_string())
        .header("origin", "https://evil.example")
        .json(&json!({"jsonrpc": "2.0", "id": 4, "method": "ping"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
    let res = c
        .get(format!("{}/.well-known/oauth-authorization-server", w.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
    assert_eq!(w.calls("discovery", "refused", "discovery"), 1);
    for (asked, answered) in [("2025-11-25", "2025-11-25"), ("1999-01-01", "2025-11-25")] {
        let res = w
            .rpc(Some(7), json!({"jsonrpc": "2.0", "id": 5, "method": "initialize", "params": {"protocolVersion": asked}}))
            .await;
        assert!(res.headers().get("mcp-session-id").is_none());
        let v: Value = res.json().await.unwrap();
        assert_eq!(v["result"]["protocolVersion"], answered);
    }
    let res = w
        .rpc(
            Some(7),
            json!([{"jsonrpc": "2.0", "id": 6, "method": "ping"}]),
        )
        .await;
    let v: Value = res.json().await.unwrap();
    assert_eq!(v["error"]["code"], -32600, "no batches");
    let v: Value = w
        .rpc(
            Some(7),
            json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list"}),
        )
        .await
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["get_assignment", "report_status", "plan_status"]);
}

#[tokio::test]
async fn reports_are_throttled_bounded_and_sent_only_to_a_server_that_takes_them() {
    let mut w = world(Utc::now(), false).await;
    for line in ["one", "two", "three"] {
        w.call(
            7,
            "report_status",
            json!({"doing": line, "parts": ["rubato-companion", "Bad_Id"]}),
        )
        .await;
    }
    assert_eq!(w.calls("report_status", "coalesced", ""), 2);
    let pending = w.st.reports.peek();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].doing, "three");
    assert_eq!(pending[0].parts, vec!["rubato-companion".to_owned()]);

    // The server did not announce agent_mcp: nothing leaves, ever.
    report::drain_once(&w.st);
    assert!(w.out.try_recv().is_err());
    assert!(w.st.reports.peek().is_empty());
    let not_sent: u64 = w
        .reader
        .points(metric::MCP_REPORTS)
        .iter()
        .filter(|p| {
            p.attrs[attr::OUTCOME] == "not_sent" && p.attrs[attr::DROP_REASON] == "server_too_old"
        })
        .map(|p| p.value)
        .sum();
    assert_eq!(not_sent, 1);

    w.caps
        .lock()
        .unwrap()
        .push(capability::AGENT_MCP.to_owned());
    let long = "y".repeat(300);
    w.call(7, "report_status", json!({"doing": long})).await;
    report::drain_once(&w.st);
    match w.out.try_recv() {
        Ok(CompanionMessage::AgentReport(r)) => {
            assert_eq!(r.launch_id, Uuid::from_u128(7));
            assert_eq!(r.doing.chars().count(), 200);
        }
        other => panic!("{other:?}"),
    }
}

/// ADR 0071: a report touches nothing in the registry, so a session that stops hooking still
/// turns *unknown* on time.
#[tokio::test]
async fn a_report_never_keeps_a_session_working() {
    let w = world(now(), false).await;
    {
        let mut reg = w.st.registry.lock().await;
        reg.find_by_launch(Uuid::from_u128(7)).unwrap().status = SessionStatus::Working;
    }
    let before = serde_json::to_string(&w.st.registry.lock().await.snapshot()).unwrap();
    w.call(7, "report_status", json!({"doing": "still here"}))
        .await;
    let after = w.st.registry.lock().await.snapshot();
    assert_eq!(
        serde_json::to_string(&after).unwrap(),
        before,
        "snapshot byte-equal"
    );
    let stale = rubato_protocol::Limits::default().session_stale_secs;
    let later = now() + Days::seconds(stale as i64 + 1);
    assert_eq!(
        after[0].effective_status(later, stale),
        SessionStatus::Unknown
    );
}

#[tokio::test]
async fn a_launch_that_never_asks_is_counted_once_and_one_that_asked_never() {
    let w = world(now() - Days::minutes(6), false).await;
    let count = |w: &World| -> u64 {
        w.reader
            .points(metric::MCP_NOT_INITIALIZED)
            .iter()
            .map(|p| p.value)
            .sum()
    };
    watch::tick(&w.st, now()).await;
    watch::tick(&w.st, now()).await;
    assert_eq!(count(&w), 1, "once over two ticks");

    let w = world(now() - Days::minutes(6), false).await;
    w.call(7, "get_assignment", json!({})).await;
    watch::tick(&w.st, now()).await;
    assert_eq!(count(&w), 0, "any request counts as asking");
}

#[tokio::test]
async fn a_launch_restored_at_start_is_not_counted() {
    let mut reg = Registry::new(Uuid::from_u128(42), "m0x6A");
    let cmd = command(7);
    reg.begin_launch(&cmd, now() - Days::minutes(30));
    reg.end_launch();
    reg.apply_launch(
        &cmd,
        "%7",
        "rubato-7",
        Path::new("/w/7"),
        now() - Days::minutes(30),
    );
    let (reader, metrics) = metric_reader::start();
    let (out, _rx) = mpsc::unbounded_channel();
    let st = AgentMcp::new(
        Arc::new(Mutex::new(reg)),
        Arc::new(metrics),
        Default::default(),
        out,
        tokio_watch::channel(true).1,
        false,
        std::env::temp_dir(),
    )
    .await;
    watch::tick(&st, now()).await;
    assert!(
        reader
            .points(metric::MCP_NOT_INITIALIZED)
            .iter()
            .all(|p| p.value == 0)
    );
}

/// Plan 220 §3 again, with the real routes: `/mcp` handlers take the registry lock the hook arm
/// takes, and hooks keep flowing under load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hooks_keep_flowing_while_tools_are_called_in_a_flood() {
    let mut w = world(Utc::now(), false).await;
    let c = reqwest::Client::new();
    let mut jobs = tokio::task::JoinSet::new();
    for i in 0..500u32 {
        let (c, base, token) = (c.clone(), w.base.clone(), w.token.clone());
        jobs.spawn(async move {
            c.post(format!("{base}/mcp"))
                .bearer_auth(token)
                .header(LAUNCH_HEADER, Uuid::from_u128(7).to_string())
                .json(&json!({"jsonrpc": "2.0", "id": i, "method": "tools/call", "params": {"name": "get_assignment", "arguments": {}}}))
                .send()
                .await
                .map(|r| r.status().as_u16())
        });
    }
    for i in 0..100 {
        let (c, base, token) = (c.clone(), w.base.clone(), w.token.clone());
        jobs.spawn(async move {
            c.post(format!("{base}/hook"))
                .bearer_auth(token)
                .json(
                    &json!({"session_id": format!("s{i}"), "hook_event_name": "Stop", "cwd": "/w"}),
                )
                .send()
                .await
                .map(|r| r.status().as_u16())
        });
    }
    let mut ok = 0;
    while let Some(r) = jobs.join_next().await {
        let s = r.unwrap().expect("answered");
        assert!(s == 200 || s == 202, "{s}");
        ok += 1;
    }
    assert_eq!(ok, 600);
    let mut hooks = 0;
    while w.hooks.try_recv().is_ok() {
        hooks += 1;
    }
    assert_eq!(hooks, 100);
}

/// Manual check (plan 220 §5), not run by CI: serve the real receiver with this slice for launch
/// 7 and write its `mcp.json` to `$RUBATO_MANUAL_MCP_JSON`, then wait 90 s so real Claude Code can
/// connect (`claude mcp list` in a directory whose `.mcp.json` is that file).
/// `RUBATO_MANUAL_MCP_JSON=/tmp/x/.mcp.json cargo test -p rubato-companion --lib manual_real_claude -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn manual_real_claude_connects() {
    let path = std::env::var("RUBATO_MANUAL_MCP_JSON").expect("RUBATO_MANUAL_MCP_JSON");
    let w = world(Utc::now(), false).await;
    // The shape `launch::render_mcp_config` writes, built here so this slice names no `launch`.
    let v = json!({"mcpServers": {"rubato-companion": {
        "type": "http",
        "url": format!("{}/mcp", w.base),
        "headers": {"Authorization": format!("Bearer {}", w.token), "X-Rubato-Launch": Uuid::from_u128(7).to_string()},
    }}});
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    println!("serving {} for launch 7; config at {path}", w.base);
    tokio::time::sleep(std::time::Duration::from_secs(90)).await;
    for tool in ["initialize", "tools/list", "get_assignment"] {
        println!(
            "{tool}: ok={} refused={}",
            w.calls(tool, "ok", ""),
            w.calls(tool, "refused", "no_launch")
        );
    }
}
