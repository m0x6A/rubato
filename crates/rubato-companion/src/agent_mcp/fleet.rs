//! The four fleet tools of a person's lead agent (ADR 0072, plan 134 design §2–§4), offered only
//! to a lead launch, with the companion's orchestrator flag on, a server that announced `fleet`,
//! and the lead secret from the lead's own `mcp.json`. Each call is one `FleetRequest`, sent only
//! while the link is up; the answer is the server's `FleetResult`, waited for up to
//! `RESULT_WAIT_SECS`, else `pending` with the request id to ask again with.
//!
//! The secret lives in the state dir's `leads.json` (0600), in no environment variable and not in
//! `launches.json`. It stops a session that only knows the lead's launch id; one that reads the
//! lead's files as the same OS user can still act as the lead, within the owner's own sessions,
//! which ADR 0072 accepts on the record.

use super::AgentMcp;
use super::tools::{Called, refused};
use crate::sessions::AssignmentView;
use rubato_protocol::fleet::{LEAD_HEADER, limits, reason};
use rubato_protocol::{
    CompanionMessage, FleetAction, FleetRequest, FleetResult, IssueRef, LaunchRole, capability,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;
use uuid::Uuid;

/// The fleet tools, in the order `tools/list` gives them after the three of ADR 0071.
pub const FLEET_TOOLS: &[(&str, &str, bool)] = &[
    (
        "fleet_list",
        "Your owner's own sessions in this workspace: issue, branch, state (or unknown), who started each, waiting messages, the last reply's first line. Nobody else's sessions are listed.",
        true,
    ),
    (
        "fleet_launch",
        "Start a worker on an issue, as your owner, on one of your owner's connected companions. Refused for an issue another person launched or drafted, and past the fleet size.",
        false,
    ),
    (
        "fleet_message",
        "Send a message to a worker you launched. It is typed when the worker waits for input, with a first line naming you. Only your own workers.",
        false,
    ),
    (
        "fleet_stop",
        "Stop a worker you launched. Only your own workers.",
        false,
    ),
];

pub(super) fn is_fleet_tool(name: &str) -> bool {
    FLEET_TOOLS.iter().any(|(n, _, _)| *n == name)
}

/// The `tools/list` entries of the fleet tools.
pub(super) fn list() -> Vec<Value> {
    FLEET_TOOLS
        .iter()
        .map(|(name, description, read_only)| {
            json!({
                "name": name,
                "description": description,
                "inputSchema": schema(name),
                "annotations": {"readOnlyHint": read_only},
            })
        })
        .collect()
}

fn schema(name: &str) -> Value {
    let request_id = json!({"type": "string", "description": "Only to ask again about a call that answered pending: its request_id."});
    match name {
        "fleet_launch" => json!({"type": "object", "properties": {
            "issue": {"type": "string", "description": "owner/name#n"},
            "companion_id": {"type": "string", "description": "Optional: one of your owner's companions."},
            "slug": {"type": "string", "description": "Optional branch slug."},
            "request_id": request_id}, "required": ["issue"], "additionalProperties": false}),
        "fleet_message" => json!({"type": "object", "properties": {
            "launch_id": {"type": "string"}, "text": {"type": "string"}, "request_id": request_id},
            "required": ["launch_id", "text"], "additionalProperties": false}),
        "fleet_stop" => json!({"type": "object", "properties": {
            "launch_id": {"type": "string"}, "request_id": request_id},
            "required": ["launch_id"], "additionalProperties": false}),
        _ => {
            json!({"type": "object", "properties": {"request_id": request_id}, "additionalProperties": false})
        }
    }
}

/// The state dir's lead secrets, by launch id.
pub fn leads_path(state_dir: &Path) -> PathBuf {
    state_dir.join("leads.json")
}

fn read_secrets(state_dir: &Path) -> BTreeMap<Uuid, String> {
    std::fs::read_to_string(leads_path(state_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// A new secret for lead launch `launch_id`, kept in `leads.json` (0600).
pub fn new_lead_secret(state_dir: &Path, launch_id: Uuid) -> anyhow::Result<String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let mut all = read_secrets(state_dir);
    all.insert(launch_id, secret.clone());
    let tmp = leads_path(state_dir).with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(serde_json::to_string(&all)?.as_bytes())?;
    std::fs::rename(&tmp, leads_path(state_dir))?;
    Ok(secret)
}

/// Whether `given` is lead `launch_id`'s secret, compared in constant time.
pub fn secret_matches(state_dir: &Path, launch_id: Uuid, given: Option<&str>) -> bool {
    let (Some(given), Some(want)) = (given, read_secrets(state_dir).remove(&launch_id)) else {
        return false;
    };
    let (a, b) = (given.as_bytes(), want.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Waiters for a `FleetResult`, by request id.
#[derive(Default)]
pub(super) struct Waiters(Mutex<HashMap<Uuid, oneshot::Sender<FleetResult>>>);

/// Whether this launch may use the fleet tools now: the flag, the role, the server, the secret.
pub(super) fn lead_ok(
    st: &AgentMcp,
    view: &AssignmentView,
    headers: &axum::http::HeaderMap,
) -> Result<(), &'static str> {
    if !st.orchestrator {
        return Err(reason::ORCHESTRATOR_OFF);
    }
    if view.assignment.role != LaunchRole::Lead {
        return Err(reason::NOT_A_LEAD);
    }
    if !st
        .server_capabilities
        .lock()
        .unwrap()
        .iter()
        .any(|c| c == capability::FLEET)
    {
        return Err(reason::ORCHESTRATOR_OFF);
    }
    let given = headers.get(LEAD_HEADER).and_then(|v| v.to_str().ok());
    if !secret_matches(&st.state_dir, view.launch_id, given) {
        return Err("lead_secret");
    }
    Ok(())
}

fn text(v: &Value, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": v.to_string()}], "isError": is_error})
}

fn uuid_arg(args: &Value, key: &str) -> Option<Uuid> {
    args.get(key)?.as_str()?.parse().ok()
}

/// The action a call asks for, or why its arguments do not read.
fn action(name: &str, args: &Value) -> Result<FleetAction, &'static str> {
    Ok(match name {
        "fleet_list" => FleetAction::List,
        "fleet_launch" => FleetAction::Launch {
            issue: args
                .get("issue")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<IssueRef>().ok())
                .ok_or("issue as owner/name#n is required")?,
            companion_id: uuid_arg(args, "companion_id"),
            slug: args.get("slug").and_then(Value::as_str).map(str::to_owned),
        },
        "fleet_message" => FleetAction::Message {
            launch_id: uuid_arg(args, "launch_id").ok_or("launch_id is required")?,
            text: args
                .get("text")
                .and_then(Value::as_str)
                .ok_or("text is required")?
                .to_owned(),
        },
        "fleet_stop" => FleetAction::Stop {
            launch_id: uuid_arg(args, "launch_id").ok_or("launch_id is required")?,
        },
        _ => return Err("unknown fleet tool"),
    })
}

/// Run fleet tool `name` for lead `view`.
pub(super) async fn call(
    st: &AgentMcp,
    view: &AssignmentView,
    name: &str,
    args: &Value,
    headers: &axum::http::HeaderMap,
) -> Called {
    match lead_ok(st, view, headers) {
        Ok(()) => {}
        Err("lead_secret") => {
            return refused("lead_secret", "this session is not the lead it names");
        }
        Err(why) => {
            return refused(
                why,
                "the fleet tools are for a lead launch with the orchestrator on",
            );
        }
    }
    let action = match action(name, args) {
        Ok(a) => a,
        Err(why) => {
            return Called {
                result: text(&json!({"error": why}), true),
                outcome: "error",
                reason: "arguments",
            };
        }
    };
    if !*st.link.borrow() {
        return refused(
            reason::LINK_DOWN,
            "the companion is not connected to Rubato; try again later",
        );
    }
    let request_id = uuid_arg(args, "request_id").unwrap_or_else(Uuid::new_v4);
    let (tx, rx) = oneshot::channel();
    st.fleet_waiters.0.lock().unwrap().insert(request_id, tx);
    let sent = st
        .out
        .send(CompanionMessage::FleetRequest(FleetRequest {
            request_id,
            lead_launch_id: view.launch_id,
            action,
        }))
        .is_ok();
    if !sent {
        st.fleet_waiters.0.lock().unwrap().remove(&request_id);
        return refused(reason::LINK_DOWN, "the companion's link is closing");
    }
    match tokio::time::timeout(Duration::from_secs(limits::RESULT_WAIT_SECS), rx).await {
        Ok(Ok(r)) => answer(r),
        _ => {
            st.fleet_waiters.0.lock().unwrap().remove(&request_id);
            Called {
                result: text(
                    &json!({"status": "pending", "request_id": request_id, "message": "no answer yet; call again with this request_id to get it, without acting twice"}),
                    false,
                ),
                outcome: "pending",
                reason: "",
            }
        }
    }
}

fn answer(r: FleetResult) -> Called {
    let refused_reason = r.reason.clone();
    let v = serde_json::to_value(&r).unwrap_or_default();
    match r.outcome {
        rubato_protocol::FleetOutcome::Refused => Called {
            result: text(&v, true),
            outcome: "refused",
            reason: static_reason(refused_reason.as_deref()),
        },
        _ => Called {
            result: text(&v, false),
            outcome: "ok",
            reason: "",
        },
    }
}

/// `rubato.reason` takes only the protocol's own names, never a string the server sent.
fn static_reason(r: Option<&str>) -> &'static str {
    const KNOWN: &[&str] = &[
        reason::ORCHESTRATOR_OFF,
        reason::IDENTITY,
        reason::NOT_A_LEAD,
        reason::OTHER_OWNER,
        reason::NOT_LED,
        reason::UNKNOWN_LAUNCH,
        reason::FLEET_FULL,
        reason::SOMEONE_ELSES_ISSUE,
        reason::REQUEST_ID_REUSED,
        reason::EMPTY,
        reason::TOO_LONG,
        reason::LAUNCH_REFUSED,
    ];
    r.and_then(|r| KNOWN.iter().find(|k| **k == r).copied())
        .unwrap_or("other")
}

/// A `FleetResult` from the server: the call waiting for it answers.
pub(super) fn on_result(st: &AgentMcp, r: FleetResult) {
    if let Some(tx) = st.fleet_waiters.0.lock().unwrap().remove(&r.request_id) {
        let _ = tx.send(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_matches_only_its_lead() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let sa = new_lead_secret(d.path(), a).unwrap();
        let sb = new_lead_secret(d.path(), b).unwrap();
        assert!(secret_matches(d.path(), a, Some(&sa)));
        assert!(
            !secret_matches(d.path(), a, Some(&sb)),
            "another lead's secret"
        );
        assert!(!secret_matches(d.path(), a, None));
        assert!(!secret_matches(d.path(), Uuid::new_v4(), Some(&sa)));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(leads_path(d.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn arguments_read_into_actions() {
        assert_eq!(action("fleet_list", &json!({})).unwrap(), FleetAction::List);
        let l = action("fleet_launch", &json!({"issue": "m0x6A/rubato#12"})).unwrap();
        assert!(matches!(l, FleetAction::Launch { ref issue, .. } if issue.number == 12));
        assert!(action("fleet_launch", &json!({"issue": "nope"})).is_err());
        assert!(
            action(
                "fleet_message",
                &json!({"launch_id": Uuid::nil().to_string()})
            )
            .is_err()
        );
        assert_eq!(static_reason(Some("other_owner")), "other_owner");
        assert_eq!(static_reason(Some("<script>")), "other");
    }
}
