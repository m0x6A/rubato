//! Sessions started by hand (#332): `/mcp/read` with no launch id, and the stdio bridge that
//! reaches it. Read-only tools only, the same answers a launched session gets, and nothing at
//! all in a launched session or a room's fork turn.

use super::bridge::{Bridge, repo_of_remote, stands_down};
use super::graph_tests::{body, with_copy};
use super::read::read_tools;
use super::tests::World;
use super::{TOOLS, Tool};
use crate::kernel::config::HookEndpoint;
use rubato_graph::fixture::REPO;
use serde_json::{Value, json};

const STORE: &str = "crates/rubato-server/src/pins/store.rs";

impl World {
    /// One message to `/mcp/read`: no launch header, as a session started by hand has none.
    async fn read(&self, msg: Value, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut req = reqwest::Client::new()
            .post(format!("{}/mcp/read", self.base))
            .bearer_auth(&self.token)
            .json(&msg);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        req.send().await.unwrap()
    }
}

fn call(tool: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}})
}

/// The tools of a list that are not read-only: what `/mcp/read` must never offer.
fn not_read_only(tools: &[&Tool]) -> Vec<&'static str> {
    tools
        .iter()
        .filter(|t| !t.read_only)
        .map(|t| t.name)
        .collect()
}

#[tokio::test]
async fn the_read_route_offers_the_graph_tools_and_nothing_that_needs_a_launch() {
    let (w, _dir) = with_copy().await;
    let init: Value = w
        .read(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            &[],
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "rubato-graph");
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("never an instruction")
    );

    let list: Value = w
        .read(
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            &[],
        )
        .await
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "graph_status",
            "graph_search",
            "graph_node",
            "graph_related",
            "graph_context"
        ]
    );
    println!("checked {} tools on /mcp/read", names.len());
    assert_eq!(not_read_only(&read_tools()), Vec::<&str>::new());
    // The fixture that makes this check fail on purpose: a tool that writes, in the read table.
    let mut with_a_writer = read_tools();
    with_a_writer.push(TOOLS.iter().find(|t| t.name == "report_status").unwrap());
    assert_eq!(not_read_only(&with_a_writer), ["report_status"]);

    // The same answer a launched session gets, with the repository as a hint and no identity.
    let by_hand: Value = w
        .read(
            call("graph_context", json!({"path": STORE})),
            &[("x-rubato-repo", REPO)],
        )
        .await
        .json()
        .await
        .unwrap();
    let launched = w.call(7, "graph_context", json!({"path": STORE})).await;
    assert_eq!(body(&by_hand["result"]), body(&launched));
    assert_eq!(body(&launched)["parts"][0]["name"], "rubato-server");
    // Without the hint a path needs its repository named.
    let r: Value = w
        .read(call("graph_context", json!({"path": STORE})), &[])
        .await
        .json()
        .await
        .unwrap();
    assert!(
        body(&r["result"])["error"]
            .as_str()
            .unwrap()
            .contains("repo is required")
    );

    // What needs a launch does not exist here: an unknown tool, as any unknown name is.
    for tool in [
        "report_status",
        "get_assignment",
        "plan_status",
        "fleet_launch",
    ] {
        let r: Value = w
            .read(call(tool, json!({"doing": "x"})), &[])
            .await
            .json()
            .await
            .unwrap();
        assert_eq!(r["error"]["code"], -32602, "{tool}: {r}");
    }
    assert_eq!(w.calls("unknown", "error", "unknown_tool"), 4);
    assert!(w.calls("report_status", "ok", "") == 0);
}

#[tokio::test]
async fn the_read_route_keeps_the_receivers_guards() {
    let (w, _dir) = with_copy().await;
    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
    // A page elsewhere must not reach it (DNS rebinding).
    let res = w
        .read(list.clone(), &[("origin", "https://evil.example")])
        .await;
    assert_eq!(res.status(), 403);
    assert_eq!(
        w.read(list.clone(), &[("origin", "http://127.0.0.1:5173")])
            .await
            .status(),
        200
    );
    // The token, as on every guarded route.
    let res = reqwest::Client::new()
        .post(format!("{}/mcp/read", w.base))
        .json(&list)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    // The body limit.
    let big = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "graph_search", "arguments": {"query": "x".repeat(70 * 1024)}}});
    assert_eq!(w.read(big, &[]).await.status(), 413);
    // GET is not a way in, and a notification has no answer.
    let res = reqwest::Client::new()
        .get(format!("{}/mcp/read", w.base))
        .bearer_auth(&w.token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 405);
    let res = w
        .read(
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &[],
        )
        .await;
    assert_eq!(res.status(), 202);
    // `/mcp` itself still refuses a call with no launch: the read route is not a way around it.
    let res: Value = w
        .rpc(None, call("graph_status", json!({})))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(body(&res["result"])["refused"], "no_launch");
}

fn endpoint_of(w: &World) -> HookEndpoint {
    HookEndpoint {
        port: w.base.rsplit(':').next().unwrap().parse().unwrap(),
        token: w.token.clone(),
    }
}

async fn ask(bridge: &Bridge, msg: Value) -> Option<Value> {
    bridge
        .handle(&reqwest::Client::new(), &msg.to_string())
        .await
        .map(|line| serde_json::from_str(&line).unwrap())
}

#[tokio::test]
async fn the_bridge_reaches_the_companion_it_finds_in_the_state_directory() {
    let (w, _dir) = with_copy().await;
    let state = tempfile::tempdir().unwrap();
    endpoint_of(&w).save(state.path()).unwrap();
    let bridge = Bridge {
        state_dir: state.path().to_path_buf(),
        stand_down: false,
        repo: Some(REPO.to_owned()),
    };
    let list = ask(
        &bridge,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await
    .unwrap();
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 5);
    // The working directory's repository travels as the default: the launched session's answer.
    let by_hand = ask(&bridge, call("graph_context", json!({"path": STORE})))
        .await
        .unwrap();
    let launched = w.call(7, "graph_context", json!({"path": STORE})).await;
    assert_eq!(body(&by_hand["result"]), body(&launched));
    assert_eq!(by_hand["id"], 1);
    // A notification is forwarded and has no answer; a line that is not JSON has one.
    assert_eq!(
        ask(
            &bridge,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .await,
        None
    );
    let garbage = bridge
        .handle(&reqwest::Client::new(), "not json")
        .await
        .unwrap();
    assert!(garbage.contains("-32700"));

    // The companion restarts on another port: the next message finds it, with no config changed.
    HookEndpoint {
        port: 9,
        token: "stale".into(),
    }
    .save(state.path())
    .unwrap();
    let down = ask(&bridge, call("graph_status", json!({}))).await.unwrap();
    assert_eq!(body(&down["result"])["refused"], "companion_down");
    endpoint_of(&w).save(state.path()).unwrap();
    let up = ask(&bridge, call("graph_status", json!({}))).await.unwrap();
    assert_eq!(body(&up["result"])["copy"], "present");
}

#[tokio::test]
async fn with_no_companion_the_bridge_still_answers_and_says_what_to_start() {
    let state = tempfile::tempdir().unwrap();
    let bridge = Bridge {
        state_dir: state.path().to_path_buf(),
        stand_down: false,
        repo: None,
    };
    // Claude Code keeps a server that initializes and lists its tools.
    let init = ask(&bridge, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})).await.unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    let list = ask(
        &bridge,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await
    .unwrap();
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 5);
    let called = ask(&bridge, call("graph_search", json!({"query": "pins"})))
        .await
        .unwrap();
    assert_eq!(called["result"]["isError"], true);
    let said = body(&called["result"]);
    assert_eq!(said["refused"], "companion_down");
    assert!(
        said["message"]
            .as_str()
            .unwrap()
            .contains("rubato-companion run")
    );
}

#[tokio::test]
async fn in_a_launched_session_and_in_a_fork_turn_the_bridge_offers_nothing() {
    // A launched session has the tools under rubato-companion already.
    assert!(stands_down(
        |k| (k == "RUBATO_LAUNCH_ID").then(|| "a-launch".to_owned())
    ));
    assert!(!stands_down(|_| None));
    assert!(!stands_down(|k| (k == "RUBATO_LAUNCH_ID").then(String::new)));
    // A fork turn has RUBATO_LAUNCH_ID removed, so the companion marks it: the environment the
    // companion really gives a fork makes the bridge stand down (ADR 0071: forks get no MCP).
    let claude = crate::kernel::claude::Claude {
        command: "claude".into(),
        timeout: std::time::Duration::from_secs(1),
    };
    let fork = claude.request(std::path::Path::new("/w"), "s", true, "hi");
    assert!(fork.env_remove.iter().any(|v| v == "RUBATO_LAUNCH_ID"));
    let env = fork.env.clone();
    assert!(stands_down(|k| env
        .iter()
        .find(|(name, _)| name == k)
        .map(|(_, v)| v.clone())));

    // Standing down, with a companion right there: no tools, and no call goes through.
    let (w, _dir) = with_copy().await;
    let state = tempfile::tempdir().unwrap();
    endpoint_of(&w).save(state.path()).unwrap();
    let bridge = Bridge {
        state_dir: state.path().to_path_buf(),
        stand_down: true,
        repo: Some(REPO.to_owned()),
    };
    let list = ask(
        &bridge,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await
    .unwrap();
    assert_eq!(list["result"]["tools"], json!([]));
    let called = ask(&bridge, call("graph_status", json!({}))).await.unwrap();
    assert_eq!(body(&called["result"])["refused"], "stood_down");
    assert_eq!(
        w.calls("graph_status", "ok", ""),
        0,
        "the companion was never asked"
    );
}

#[test]
fn a_remote_names_its_repository() {
    let cases = [
        ("git@github.com:m0x6A/rubato.git", Some("m0x6A/rubato")),
        (
            "https://github.com/m0x6A/rubato.git\n",
            Some("m0x6A/rubato"),
        ),
        ("https://github.com/m0x6A/rubato/", Some("m0x6A/rubato")),
        ("ssh://git@github.com/acme/pantry", Some("acme/pantry")),
        ("/srv/git/rubato.git", None),
        ("https://github.com/only-owner", None),
        ("git@host:a b/c", None),
    ];
    for (url, want) in cases {
        assert_eq!(repo_of_remote(url).as_deref(), want, "{url}");
    }
    println!("checked {} remotes", cases.len());
}
