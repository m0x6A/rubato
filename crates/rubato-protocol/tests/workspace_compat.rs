//! An overview and a web event from a server without workspaces still read (ADRs 0044, 0049):
//! the web keeps working against an older server, and an older web against a newer one.

use rubato_protocol::{Overview, WebEvent};

/// What today's server sends, cut to the fields every overview has.
const OLD_OVERVIEW: &str = r#"{
  "map": { "map": { "version": 1, "name": "acme", "nodes": [] }, "version": "abc1234", "loaded_at": "2026-11-14T08:00:00Z" },
  "issues": [{ "issue": { "repo": "acme/auth", "number": 7 }, "pins": [], "progress": { "issue": { "repo": "acme/auth", "number": 7 }, "state": "planned", "last_seen": "2026-11-14T08:00:00Z" } }],
  "findings": [],
  "companions": [],
  "backlog": { "issues": [{ "issue": { "repo": "acme/auth", "number": 8 }, "title": "t", "url": "u", "updated_at": "2026-11-14T08:00:00Z", "map_line": { "kind": "node", "id": "auth-api" } }] },
  "generated_at": "2026-11-14T08:00:00Z"
}"#;

#[test]
fn an_old_overview_reads_as_the_default_workspace() {
    let ov: Overview = serde_json::from_str(OLD_OVERVIEW).unwrap();
    assert_eq!(ov.workspace, "default");
    assert!(ov.workspaces.is_empty());
    assert!(ov.issues[0].fits_in.is_empty() && ov.issues[0].pinned_in.is_empty());
    let line = ov.backlog.issues[0].map_line.as_ref().unwrap();
    assert_eq!(line.target.id(), "auth-api");
    assert_eq!(line.workspace, None);
    // Nothing new is written for a single unqualified workspace, beyond the workspace id.
    let back = serde_json::to_value(&ov).unwrap();
    assert!(back.get("workspaces").is_none());
    assert!(back["issues"][0].get("fits_in").is_none());
    assert_eq!(
        back["backlog"]["issues"][0]["map_line"],
        serde_json::json!({ "kind": "node", "id": "auth-api" })
    );
}

#[test]
fn an_old_changed_event_concerns_every_workspace() {
    let ev: WebEvent =
        serde_json::from_str(r#"{"type":"changed","at":"2026-11-14T08:00:00Z","reason":"pin"}"#)
            .unwrap();
    let WebEvent::Changed {
        workspace, reason, ..
    } = ev;
    assert_eq!((workspace, reason.as_str()), (None, "pin"));
}
