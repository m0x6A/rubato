//! Whether to type a session message, checked in the order plan 122 §D1 gives: the paired user,
//! a session launched here, the length, the delivery record, then idle. Only a hook proves
//! idle, and only a hook newer than the companion's own last input. A `plan_approved` is typed
//! as the handoff command (`handoff_prompt`, ADR 0055), and only when the installed skill has
//! `plan.ts handoff`; the length check is on the text that would be typed.

use super::deliveries::{Deliveries, Delivery};
use crate::sessions::{PASTED_KEPT, Registry};
use chrono::{DateTime, Utc};
use rubato_protocol::session_message::handoff_prompt;
use rubato_protocol::{
    HookEventKind, MessageRefusal, RoomTurnMode, SessionMessage, SessionMessageKind,
    SessionMessageOutcome, skill_has_handoff,
};
use uuid::Uuid;

/// What to do with one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Answer without typing.
    Answer(SessionMessageOutcome),
    /// Type `text` into `target`, a pane id or a tmux session name.
    Paste {
        target: String,
        session_id: String,
        text: String,
    },
    /// A room turn in fork mode (ADR 0057): nothing is typed; a read-only fork of `session_id`
    /// runs in `worktree` and its answer comes back as `RoomReply`.
    Fork {
        session_id: String,
        worktree: std::path::PathBuf,
    },
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
    skill_version: Option<&str>,
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
    let text = match handoff_prompt(&msg.issue, &msg.kind) {
        Some(prompt) => {
            if !skill_version.is_some_and(skill_has_handoff) {
                return refused(MessageRefusal::SkillOutdated);
            }
            prompt
        }
        None => msg.kind.text(),
    };
    if text.chars().count() as u64 > max_chars {
        return refused(MessageRefusal::TooLong);
    }
    match deliveries.get(msg.id) {
        Some(Delivery::Delivered) => return Answer(SessionMessageOutcome::Delivered),
        Some(Delivery::Typing) => return Answer(SessionMessageOutcome::Interrupted),
        None => {}
    }
    // A fork does not need the session idle or a pane: it runs beside it.
    if let SessionMessageKind::RoomTurn {
        mode: RoomTurnMode::Fork,
        ..
    } = &msg.kind
    {
        return Decision::Fork {
            session_id,
            worktree: e.worktree.clone().unwrap_or_else(|| e.cwd.clone()),
        };
    }
    if !e.waits_for_input() {
        return Answer(SessionMessageOutcome::NotIdle);
    }
    let Some(target) = e.pane_id.clone().or_else(|| e.tmux_session.clone()) else {
        return refused(MessageRefusal::NoPane);
    };
    Decision::Paste {
        target,
        session_id,
        text,
    }
}

impl Registry {
    /// The companion typed `text` (message `message_id`) into the launch's session at `at`: the
    /// next message waits for an idle hook newer than this, and the next prompt ending with
    /// `text` is tagged with `message_id`.
    pub fn mark_input_sent(
        &mut self,
        launch_id: Uuid,
        at: DateTime<Utc>,
        message_id: Uuid,
        text: &str,
    ) {
        if let Some(e) = self.find_by_launch(launch_id) {
            e.input_sent_at = Some(at);
            e.pasted.push((message_id, text.to_owned()));
            let over = e.pasted.len().saturating_sub(PASTED_KEPT);
            e.pasted.drain(..over);
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
                plan_path: None,
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
            Decision::Paste { .. } | Decision::Fork { .. } => {
                panic!("expected an answer, got {d:?}")
            }
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
            decide(&message(1, "hi"), USER, &r, 8000, &d, None),
            Decision::Paste {
                target: "%7".into(),
                session_id: "s7".into(),
                text: "hi".into(),
            }
        );
    }

    fn plan_approved(id: u128) -> SessionMessage {
        let mut m = message(id, "");
        m.kind = rubato_protocol::SessionMessageKind::PlanApproved {
            approval_id: Uuid::from_u128(5),
            approved_by: USER.into(),
            approved_at: t(0),
            approved_sha: "22e89bdc981e97d34dc7057747332a8e23cdf0e9".into(),
            bundle_hash: "0123abcd".into(),
            branch: "issue/7-restock".into(),
            plan_path: "docs/plans/7.md".into(),
            self_approved: false,
            url: "https://rubato.example/approvals/5".into(),
        };
        m
    }

    /// ADR 0055: `plan_approved` is typed as the handoff command, never as its summary text, and
    /// only when the installed skill has `plan.ts handoff`. The owner and launch checks come first,
    /// as for chat, and the length check measures the command.
    #[test]
    fn plan_approved_types_the_handoff_command_with_a_skill_that_has_it() {
        let dir = tempfile::tempdir().unwrap();
        let d = Deliveries::open(dir.path()).unwrap();
        let mut r = registry(Some("%7"));
        hook(&mut r, "Stop", t(1));
        let m = plan_approved(1);
        let prompt = handoff_prompt(&m.issue, &m.kind).unwrap();
        match decide(&m, USER, &r, 8000, &d, Some("0.2.0")) {
            Decision::Paste { text, .. } => {
                assert_eq!(text, prompt);
                assert!(text.contains("plan.ts handoff --approval"), "{text}");
                assert_ne!(text, m.kind.text());
            }
            o => panic!("expected a paste, got {o:?}"),
        }
        for skill in [None, Some("0.1.9"), Some("dev")] {
            assert_eq!(
                reason(decide(&m, USER, &r, 8000, &d, skill)),
                MessageRefusal::SkillOutdated,
                "{skill:?}"
            );
        }
        // Chat does not need the skill.
        assert!(matches!(
            decide(&message(2, "hi"), USER, &r, 8000, &d, None),
            Decision::Paste { .. }
        ));
        // The owner check comes first.
        let mut other = plan_approved(3);
        other.user = "someone-else".into();
        assert_eq!(
            reason(decide(&other, USER, &r, 8000, &d, None)),
            MessageRefusal::NotPairedUser
        );
        // The length is the typed command's, not the short summary's.
        let max = prompt.chars().count() as u64;
        assert!(m.kind.text().chars().count() < 60);
        assert!(matches!(
            decide(&m, USER, &r, max, &d, Some("0.2.0")),
            Decision::Paste { .. }
        ));
        assert_eq!(
            reason(decide(&m, USER, &r, max - 1, &d, Some("0.2.0"))),
            MessageRefusal::TooLong
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
            reason(decide(&other, USER, &r, 8000, &d, None)),
            MessageRefusal::NotPairedUser
        );

        let mut elsewhere = message(1, "hi");
        elsewhere.launch_id = Uuid::from_u128(99);
        assert_eq!(
            reason(decide(&elsewhere, USER, &r, 8000, &d, None)),
            MessageRefusal::NotLaunchedHere
        );

        assert_eq!(
            reason(decide(&message(1, "héllo"), USER, &r, 4, &d, None)),
            MessageRefusal::TooLong,
        );
        // Counted in characters, not bytes.
        assert!(matches!(
            decide(&message(1, "héllo"), USER, &r, 5, &d, None),
            Decision::Paste { .. }
        ));

        d.record(Uuid::from_u128(2), Delivery::Delivered, t(1))
            .unwrap();
        d.record(Uuid::from_u128(3), Delivery::Typing, t(1))
            .unwrap();
        // A delivered or interrupted id is answered from the record even when not idle.
        hook(&mut r, "UserPromptSubmit", t(2));
        assert_eq!(
            outcome(decide(&message(2, "hi"), USER, &r, 8000, &d, None)),
            SessionMessageOutcome::Delivered
        );
        assert_eq!(
            outcome(decide(&message(3, "hi"), USER, &r, 8000, &d, None)),
            SessionMessageOutcome::Interrupted
        );
        assert_eq!(
            outcome(decide(&message(4, "hi"), USER, &r, 8000, &d, None)),
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
            outcome(decide(&m, USER, &r, 8000, &d, None)),
            SessionMessageOutcome::NotIdle
        );
        hook(&mut r, "Stop", t(1));
        assert!(matches!(
            decide(&m, USER, &r, 8000, &d, None),
            Decision::Paste { .. }
        ));
        r.mark_input_sent(Uuid::from_u128(7), t(2), Uuid::from_u128(1), "hi");
        assert_eq!(
            outcome(decide(&m, USER, &r, 8000, &d, None)),
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
            decide(&m, USER, &r, 8000, &d, None),
            Decision::Paste { .. }
        ));
        idle.notification_type = Some("permission_prompt".into());
        idle.received_at = t(4);
        r.apply_hook(&idle);
        assert_eq!(
            outcome(decide(&m, USER, &r, 8000, &d, None)),
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
            reason(decide(&message(1, "hi"), USER, &r, 8000, &d, None)),
            MessageRefusal::NoPane
        );
        let mut r = registry(Some("%7"));
        hook(&mut r, "Stop", t(1));
        r.apply_panes(&[], t(2));
        assert_eq!(
            reason(decide(&message(1, "hi"), USER, &r, 8000, &d, None)),
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
            outcome(decide(&message(1, "hi"), USER, &r, 8000, &d, None)),
            SessionMessageOutcome::NotIdle
        );
        assert_eq!(
            last_hook(&r, Uuid::from_u128(7)),
            Some(HookEventKind::SessionEnd)
        );
    }
}
