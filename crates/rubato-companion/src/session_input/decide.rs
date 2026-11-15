//! Whether to type a session message, checked in the order plan 122 §D1 gives: the paired user,
//! a session launched here, the length, the delivery record, then idle. Only a hook proves
//! idle, and only a hook newer than the companion's own last input.

use super::deliveries::{Deliveries, Delivery};
use crate::sessions::Registry;
use chrono::{DateTime, Utc};
use rubato_protocol::{HookEventKind, MessageRefusal, SessionMessage, SessionMessageOutcome};
use uuid::Uuid;

/// What to do with one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Answer without typing.
    Answer(SessionMessageOutcome),
    /// Type it into `target`, a pane id or a tmux session name.
    Paste { target: String, session_id: String },
}

/// Why a message waits, for the `not idle` log line: the last hook, if any.
pub fn last_hook(reg: &Registry, launch_id: Uuid) -> Option<HookEventKind> {
    reg.by_launch(launch_id).and_then(|e| e.last_hook)
}

pub fn decide(
    msg: &SessionMessage,
    paired_user: &str,
    reg: &Registry,
    max_chars: u64,
    deliveries: &Deliveries,
) -> Decision {
    use Decision::Answer;
    let refused = |reason| Answer(SessionMessageOutcome::Refused { reason });
    if msg.user != paired_user {
        return refused(MessageRefusal::NotPairedUser);
    }
    let Some(e) = reg.by_launch(msg.launch_id) else {
        return refused(MessageRefusal::NotLaunchedHere);
    };
    let Some(session_id) = e.session_id.clone().filter(|_| e.is_live()) else {
        return refused(MessageRefusal::UnknownSession);
    };
    if msg.kind.text().chars().count() as u64 > max_chars {
        return refused(MessageRefusal::TooLong);
    }
    match deliveries.get(msg.id) {
        Some(Delivery::Delivered) => return Answer(SessionMessageOutcome::Delivered),
        Some(Delivery::Typing) => return Answer(SessionMessageOutcome::Interrupted),
        None => {}
    }
    if !e.waits_for_input() {
        return Answer(SessionMessageOutcome::NotIdle);
    }
    let Some(target) = e.pane_id.clone().or_else(|| e.tmux_session.clone()) else {
        return refused(MessageRefusal::NoPane);
    };
    Decision::Paste { target, session_id }
}

impl Registry {
    /// The companion typed into the launch's session at `at`: the next message waits for an
    /// idle hook newer than this.
    pub fn mark_input_sent(&mut self, launch_id: Uuid, at: DateTime<Utc>) {
        if let Some(e) = self.find_by_launch(launch_id) {
            e.input_sent_at = Some(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::LaunchRecord;
    use rubato_protocol::{HookEvent, IssueRef, SessionMessageKind};
    use std::path::PathBuf;

    const USER: &str = "m0x6A";

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_794_000_000 + s, 0).unwrap()
    }

    fn message(id: u128, text: &str) -> SessionMessage {
        SessionMessage {
            id: Uuid::from_u128(id),
            user: USER.into(),
            launch_id: Uuid::from_u128(7),
            session_id: Some("s7".into()),
            issue: IssueRef::new("acme/pantry", 7),
            kind: SessionMessageKind::Chat {
                text: text.into(),
                author: USER.into(),
            },
            created_at: t(0),
        }
    }

    fn registry(pane: Option<&str>) -> Registry {
        let mut r = Registry::new(Uuid::nil(), USER);
        r.restore_launches(
            vec![LaunchRecord {
                launch_id: Uuid::from_u128(7),
                session_id: "s7".into(),
                worktree: PathBuf::from("/w"),
                pane_id: pane.map(str::to_owned),
                tmux_session: pane.map(|_| "pantry@issue-7-restock".to_owned()),
                issue: IssueRef::new("acme/pantry", 7),
                branch: Some("issue/7-restock".into()),
                spend_cap_usd: None,
            }],
            None,
            false,
            t(0),
        );
        r
    }

    fn hook(r: &mut Registry, name: &str, at: DateTime<Utc>) {
        let ev = HookEvent::from_payload(
            serde_json::json!({"session_id": "s7", "hook_event_name": name, "cwd": "/w"}),
            at,
        )
        .unwrap();
        r.apply_hook(&ev);
    }

    fn outcome(d: Decision) -> SessionMessageOutcome {
        match d {
            Decision::Answer(o) => o,
            Decision::Paste { .. } => panic!("expected an answer, got {d:?}"),
        }
    }

    fn reason(d: Decision) -> MessageRefusal {
        match outcome(d) {
            SessionMessageOutcome::Refused { reason } => reason,
            o => panic!("expected a refusal, got {o:?}"),
        }
    }

    #[test]
    fn a_stop_hook_makes_it_paste_into_the_pane() {
        let dir = tempfile::tempdir().unwrap();
        let d = Deliveries::open(dir.path()).unwrap();
        let mut r = registry(Some("%7"));
        hook(&mut r, "Stop", t(1));
        assert_eq!(
            decide(&message(1, "hi"), USER, &r, 8000, &d),
            Decision::Paste {
                target: "%7".into(),
                session_id: "s7".into()
            }
        );
    }

    /// Each check in its place: every message below fails exactly one, and the later checks
    /// would pass, so a reordering shows up as the wrong reason.
    #[test]
    fn the_checks_run_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = Deliveries::open(dir.path()).unwrap();
        let mut r = registry(Some("%7"));
        hook(&mut r, "Stop", t(1));

        let mut other = message(1, "hi");
        other.user = "someone-else".into();
        assert_eq!(
            reason(decide(&other, USER, &r, 8000, &d)),
            MessageRefusal::NotPairedUser
        );

        let mut elsewhere = message(1, "hi");
        elsewhere.launch_id = Uuid::from_u128(99);
        assert_eq!(
            reason(decide(&elsewhere, USER, &r, 8000, &d)),
            MessageRefusal::NotLaunchedHere
        );

        assert_eq!(
            reason(decide(&message(1, "héllo"), USER, &r, 4, &d)),
            MessageRefusal::TooLong,
        );
        // Counted in characters, not bytes.
        assert!(matches!(
            decide(&message(1, "héllo"), USER, &r, 5, &d),
            Decision::Paste { .. }
        ));

        d.record(Uuid::from_u128(2), Delivery::Delivered, t(1))
            .unwrap();
        d.record(Uuid::from_u128(3), Delivery::Typing, t(1))
            .unwrap();
        // A delivered or interrupted id is answered from the record even when not idle.
        hook(&mut r, "UserPromptSubmit", t(2));
        assert_eq!(
            outcome(decide(&message(2, "hi"), USER, &r, 8000, &d)),
            SessionMessageOutcome::Delivered
        );
        assert_eq!(
            outcome(decide(&message(3, "hi"), USER, &r, 8000, &d)),
            SessionMessageOutcome::Interrupted
        );
        assert_eq!(
            outcome(decide(&message(4, "hi"), USER, &r, 8000, &d)),
            SessionMessageOutcome::NotIdle
        );
    }

    #[test]
    fn only_an_idle_hook_newer_than_the_last_input_counts() {
        let dir = tempfile::tempdir().unwrap();
        let d = Deliveries::open(dir.path()).unwrap();
        let mut r = registry(Some("%7"));
        let m = message(1, "hi");
        // Restored, no hook yet: nothing proves the turn ended.
        assert_eq!(
            outcome(decide(&m, USER, &r, 8000, &d)),
            SessionMessageOutcome::NotIdle
        );
        hook(&mut r, "Stop", t(1));
        assert!(matches!(
            decide(&m, USER, &r, 8000, &d),
            Decision::Paste { .. }
        ));
        r.mark_input_sent(Uuid::from_u128(7), t(2));
        assert_eq!(
            outcome(decide(&m, USER, &r, 8000, &d)),
            SessionMessageOutcome::NotIdle,
            "a second message waits for the turn the first began"
        );
        let mut idle = HookEvent::from_payload(
            serde_json::json!({"session_id": "s7", "hook_event_name": "Notification",
                "notification_type": "idle_prompt", "cwd": "/w"}),
            t(3),
        )
        .unwrap();
        r.apply_hook(&idle);
        assert!(matches!(
            decide(&m, USER, &r, 8000, &d),
            Decision::Paste { .. }
        ));
        idle.notification_type = Some("permission_prompt".into());
        idle.received_at = t(4);
        r.apply_hook(&idle);
        assert_eq!(
            outcome(decide(&m, USER, &r, 8000, &d)),
            SessionMessageOutcome::NotIdle,
            "typing into a permission prompt would answer it"
        );
    }

    #[test]
    fn a_launch_with_no_live_session_or_no_pane_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let d = Deliveries::open(dir.path()).unwrap();
        let mut r = registry(None);
        hook(&mut r, "Stop", t(1));
        assert_eq!(
            reason(decide(&message(1, "hi"), USER, &r, 8000, &d)),
            MessageRefusal::NoPane
        );
        let mut r = registry(Some("%7"));
        hook(&mut r, "Stop", t(1));
        r.apply_panes(&[], t(2));
        assert_eq!(
            reason(decide(&message(1, "hi"), USER, &r, 8000, &d)),
            MessageRefusal::UnknownSession,
            "the pane vanished"
        );
    }

    /// A `SessionEnd` with the pane still there is a `/clear` under way: the message waits for
    /// the new session's idle hook instead of being refused.
    #[test]
    fn a_session_end_in_a_live_pane_waits() {
        let dir = tempfile::tempdir().unwrap();
        let d = Deliveries::open(dir.path()).unwrap();
        let mut r = registry(Some("%7"));
        hook(&mut r, "Stop", t(1));
        hook(&mut r, "SessionEnd", t(2));
        assert_eq!(
            outcome(decide(&message(1, "hi"), USER, &r, 8000, &d)),
            SessionMessageOutcome::NotIdle
        );
        assert_eq!(
            last_hook(&r, Uuid::from_u128(7)),
            Some(HookEventKind::SessionEnd)
        );
    }
}
