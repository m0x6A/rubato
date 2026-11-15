//! Steps for `features/handoff.feature`.

use super::session_messages::{docstring, launched_session, send};
use crate::world::*;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use rubato_protocol::session_message::handoff_prompt;
use rubato_protocol::{
    CompanionMessage, LaunchResult, PlanningMode, ServerMessage, SessionMessage, SessionMessageKind,
};
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;

fn skill_dir(w: &RubatoWorld) -> std::path::PathBuf {
    w.workspace
        .companion
        .skill_dir
        .clone()
        .expect("a test skill dir")
}

#[given(expr = "the installed rubato skill is version {string}")]
async fn given_skill(w: &mut RubatoWorld, version: String) {
    let dir = skill_dir(w);
    std::fs::create_dir_all(&dir).unwrap();
    let pkg = serde_json::json!({"name": "rubato-skill", "version": version, "private": true});
    std::fs::write(dir.join("package.json"), pkg.to_string()).unwrap();
}

#[given("the installed rubato skill names no version")]
async fn given_skill_unversioned(w: &mut RubatoWorld) {
    let dir = skill_dir(w);
    std::fs::create_dir_all(&dir).unwrap();
    // What install.ts writes today.
    std::fs::write(
        dir.join("package.json"),
        r#"{"name": "rubato-skill", "private": true}"#,
    )
    .unwrap();
}

#[given(expr = "the server does not announce {string}")]
async fn given_not_announced(w: &mut RubatoWorld, cap: String) {
    w.server()
        .capabilities
        .lock()
        .unwrap()
        .retain(|c| *c != cap);
}

async fn hello_capabilities(w: &RubatoWorld) -> Vec<String> {
    w.server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Hello { capabilities, .. } => Some(capabilities.clone()),
            _ => None,
        })
        .await
        .expect("a hello")
}

#[then(expr = "the server receives a hello announcing {string}")]
async fn then_hello_announces(w: &mut RubatoWorld, cap: String) {
    let caps = hello_capabilities(w).await;
    assert!(caps.contains(&cap), "{caps:?}");
}

#[then(expr = "the server receives a hello announcing nothing that starts with {string}")]
async fn then_hello_lacks(w: &mut RubatoWorld, prefix: String) {
    let caps = hello_capabilities(w).await;
    assert!(caps.iter().all(|c| !c.starts_with(&prefix)), "{caps:?}");
}

#[when("the server sends the launched session the approval of its plan")]
async fn when_send_approval(w: &mut RubatoWorld) {
    let cmd = w.last_launch.clone().expect("a launch");
    let m = SessionMessage {
        id: Uuid::new_v4(),
        user: cmd.user.clone(),
        launch_id: cmd.id,
        session_id: Some(launched_session(w)),
        issue: cmd.issue.clone(),
        kind: SessionMessageKind::PlanApproved {
            approval_id: Uuid::new_v4(),
            approved_by: cmd.user.clone(),
            approved_at: chrono::Utc::now(),
            approved_sha: "22e89bdc981e97d34dc7057747332a8e23cdf0e9".into(),
            bundle_hash: "77ab0f3c9d2e41b5a6c8e0f1d2b3a4c5e6f70812".into(),
            branch: cmd.branch.clone(),
            plan_path: "docs/plans/7.md".into(),
            self_approved: true,
            url: "http://rubato.test/approvals/1".into(),
        },
        created_at: chrono::Utc::now(),
    };
    send(w, m);
}

#[then("the stand-in claude was typed the handoff command for that approval")]
async fn then_typed_handoff(w: &mut RubatoWorld) {
    let m = w.last_message.clone().unwrap();
    let want = format!("{}\n", handoff_prompt(&m.issue, &m.kind).unwrap());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while typed(w) != want && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let got = typed(w);
    assert_eq!(got, want);
    assert!(got.contains("plan.ts handoff --approval"), "{got}");
}

#[given(
    expr = "the server has launched issue {int} of {string} with slug {string} for user {string}, planning in {string}"
)]
async fn given_launched_planning(
    w: &mut RubatoWorld,
    issue: u64,
    repo: String,
    slug: String,
    user: String,
    planning: String,
) {
    let mut cmd = w.launch_cmd(&user, &repo, issue, &slug);
    cmd.planning = serde_json::from_value(serde_json::json!(planning)).unwrap();
    assert_eq!(cmd.planning, PlanningMode::Rubato);
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd.clone()));
    let got = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started { command_id, .. })
                if *command_id == cmd.id =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(got.is_some(), "the launch started");
}

/// The value of `NAME=` in the stand-in's environment.
async fn standin_env(w: &RubatoWorld, name: &str) -> String {
    let log = standin_log(w).await;
    log.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} in {log}"))
        .to_owned()
}

#[then(expr = "the stand-in claude received {string} in its environment")]
async fn then_env(w: &mut RubatoWorld, pair: String) {
    let (name, value) = pair.split_once('=').unwrap();
    assert_eq!(standin_env(w, name).await, value);
}

#[then("the stand-in claude's environment points the handoff at the companion's hook endpoint")]
async fn then_env_endpoint(w: &mut RubatoWorld) {
    let ep = w.handles.as_ref().unwrap().hook_endpoint.clone();
    assert_eq!(
        standin_env(w, "RUBATO_HOOK_URL").await,
        format!("http://127.0.0.1:{}", ep.port)
    );
    assert_eq!(standin_env(w, "RUBATO_HOOK_TOKEN").await, ep.token);
    let launch = w.last_launch.as_ref().unwrap().id.to_string();
    assert_eq!(standin_env(w, "RUBATO_LAUNCH_ID").await, launch);
}

/// POST `body` to `$RUBATO_HOOK_URL/handoff` as `plan.ts handoff` does, with the stand-in's
/// own environment: its token and its launch id unless overridden.
async fn post_handoff(w: &mut RubatoWorld, body: &str, token: Option<&str>, with_launch: bool) {
    let url = standin_env(w, "RUBATO_HOOK_URL").await;
    let token = match token {
        Some(t) => t.to_owned(),
        None => standin_env(w, "RUBATO_HOOK_TOKEN").await,
    };
    let launch = standin_env(w, "RUBATO_LAUNCH_ID").await;
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-m", "2", "-o", "/dev/null", "-w", "%{http_code}"])
        .args(["-H", &format!("Authorization: Bearer {token}")])
        .args(["-H", "Content-Type: application/json"]);
    if with_launch {
        cmd.args(["-H", &format!("X-Rubato-Launch: {launch}")]);
    }
    let out = cmd
        .args(["--data-binary", body, &format!("{url}/handoff")])
        .output()
        .unwrap();
    w.last_status = String::from_utf8_lossy(&out.stdout).trim().parse().ok();
}

fn line(outcome: &str) -> String {
    serde_json::json!({
        "event": "rubato.plan.handoff",
        "outcome": outcome,
        "approval": "5d3b1f0e-2c4a-4b8e-9f1d-7a6c5e4b3a21",
        "commit": "abc1234",
        "adrs_accepted": ["0050"],
        "adrs_left": [],
    })
    .to_string()
}

#[when(expr = "the launched session's skill reports a {string} handoff to the companion")]
async fn when_report(w: &mut RubatoWorld, outcome: String) {
    post_handoff(w, &line(&outcome), None, true).await;
}

#[when(expr = "the launched session's skill reports a {string} handoff with the token {string}")]
async fn when_report_token(w: &mut RubatoWorld, outcome: String, token: String) {
    post_handoff(w, &line(&outcome), Some(&token), true).await;
}

#[when(expr = "the launched session's skill reports a {string} handoff without its launch header")]
async fn when_report_no_launch(w: &mut RubatoWorld, outcome: String) {
    post_handoff(w, &line(&outcome), None, false).await;
}

#[when("the launched session's skill posts this body to /handoff:")]
async fn when_post_body(w: &mut RubatoWorld, step: &Step) {
    post_handoff(w, &docstring(step), None, true).await;
}

#[then(expr = "the server receives a handoff report {string} for the launch")]
async fn then_report(w: &mut RubatoWorld, outcome: String) {
    let id = w.last_launch.as_ref().unwrap().id;
    let r = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::HandoffReport(r) => Some(r.clone()),
            _ => None,
        })
        .await
        .expect("a handoff report");
    assert_eq!(r.launch_id, id, "the launch from the header");
    assert_eq!(r.line.outcome.name(), outcome);
    assert_eq!(r.line.commit.as_deref(), Some("abc1234"));
    assert_eq!(r.line.adrs_accepted, ["0050"]);
}

#[then(expr = "no handoff report arrives within {int} second(s)")]
async fn then_no_report(w: &mut RubatoWorld, secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let n = w
        .server()
        .count(|m| matches!(m, CompanionMessage::HandoffReport(_)))
        .await;
    assert_eq!(n, 0);
}
