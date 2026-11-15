//! The session input task (plan 122 §D1): one message at a time, off the dispatch loop, so
//! hooks keep flowing while a paste runs and two pastes never overlap. For each message, inside
//! a `rubato.session_input.deliver` span: decide under the registry lock → record `typing` →
//! paste through the [`SessionInput`] port (lock released) → record `delivered` and set
//! `input_sent_at` → answer one `SessionMessageResult`. Essential: ends only when the runtime
//! drops the sender.

use super::decide::{Decision, decide, last_hook};
use super::deliveries::{Deliveries, Delivery};
use super::fork::{ForkTurn, Forks};
use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux::SessionInput;
use crate::sessions::Registry;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{
    CompanionMessage, Limits, SessionMessage, SessionMessageKind, SessionMessageOutcome,
    SessionMessageResult, capability,
};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::Instrument;

/// What the task needs, owned, so it can run on its own.
pub struct SessionInputTask {
    pub paired_user: String,
    pub registry: Arc<Mutex<Registry>>,
    pub out: mpsc::UnboundedSender<CompanionMessage>,
    /// Updated by each Welcome.
    pub limits: Arc<std::sync::Mutex<Limits>>,
    /// What the server announced in its last Welcome. A server without `session_messages`
    /// would drop the answer as a schema error, so it gets none.
    pub server_capabilities: Arc<std::sync::Mutex<Vec<String>>>,
    pub input: Arc<dyn SessionInput>,
    pub metrics: Arc<Metrics>,
    pub deliveries: Deliveries,
    /// The installed rubato skill, read for each `plan_approved` (`kernel::skill`), so an upgrade
    /// needs no companion restart.
    pub skill_dir: std::path::PathBuf,
    /// Headless mode (ADR 0062): messages run as turns, `stop` kills the running one.
    pub headless: Option<crate::sessions::Headless>,
    /// For `stop` on an interactive session: its tmux session is killed.
    pub tmux: Option<crate::kernel::tmux::Tmux>,
    /// Passed to every headless turn.
    pub permission_mode: String,
    /// The fork workers for room turns in fork mode (ADR 0057).
    pub forks: Forks,
}

pub async fn run(
    mut messages: mpsc::UnboundedReceiver<SessionMessage>,
    mut task: SessionInputTask,
) -> anyhow::Result<()> {
    while let Some(msg) = messages.recv().await {
        task.on_message(&msg).await;
    }
    Ok(())
}

impl SessionInputTask {
    pub async fn on_message(&mut self, msg: &SessionMessage) {
        let span = tracing::info_span!(
            span::SESSION_INPUT_DELIVER,
            rubato.session_message.id = %msg.id,
            rubato.session_message.kind = msg.kind.name(),
            rubato.launch_id = %msg.launch_id,
            rubato.outcome = tracing::field::Empty,
        );
        self.handle(msg).instrument(span).await
    }

    fn server_takes(&self, cap: &str) -> bool {
        self.server_capabilities
            .lock()
            .unwrap()
            .iter()
            .any(|c| c == cap)
    }

    async fn handle(&mut self, msg: &SessionMessage) {
        let takes_results = self
            .server_capabilities
            .lock()
            .unwrap()
            .iter()
            .any(|c| c == capability::SESSION_MESSAGES);
        if !takes_results {
            tracing::warn!(id = %msg.id, "session message ignored: the server did not announce session_messages");
            return;
        }
        let Some(handled) = self.deliver(msg).await else {
            // A fork turn: its worker answers when the fork starts (`fork.rs`).
            tracing::Span::current().record(attr::OUTCOME, "forked");
            tracing::info!(id = %msg.id, launch_id = %msg.launch_id, "room turn handed to a fork");
            return;
        };
        tracing::Span::current().record(attr::OUTCOME, handled.outcome.name());
        self.report(msg, &handled).await;
        let outcome = handled.outcome;
        let _ = self.out.send(CompanionMessage::SessionMessageResult(
            SessionMessageResult {
                message_id: msg.id,
                outcome,
                at: Utc::now(),
            },
        ));
    }

    /// The outcome to answer now, or None when a fork worker answers it.
    async fn deliver(&mut self, msg: &SessionMessage) -> Option<Handled> {
        if msg.kind.is_stop() {
            return Some(self.stop(msg).await);
        }
        let max_chars = self.limits.lock().unwrap().session_message_max_chars;
        // A room turn's answer is a `RoomReply`, which an older server would drop.
        if matches!(msg.kind, SessionMessageKind::RoomTurn { .. })
            && !self.server_takes(capability::ROOM_REPLIES)
        {
            return Some(Handled::failed(
                SessionMessageOutcome::Failed {
                    error: "the server did not announce room_replies".into(),
                },
                "no_room_replies",
            ));
        }
        // Only a `plan_approved` needs the skill: read it then, as it is installed now.
        let skill = match msg.kind {
            SessionMessageKind::PlanApproved { .. } => {
                crate::kernel::skill::installed_version(&self.skill_dir)
            }
            _ => None,
        };
        let decision = {
            let reg = self.registry.lock().await;
            decide(
                msg,
                &self.paired_user,
                &reg,
                max_chars,
                &self.deliveries,
                skill.as_deref(),
            )
        };
        let (target, text) = match decision {
            Decision::Answer(o) => {
                let repeat = matches!(
                    o,
                    SessionMessageOutcome::Delivered | SessionMessageOutcome::Interrupted
                );
                let failure =
                    matches!(o, SessionMessageOutcome::Interrupted).then_some("interrupted");
                return Some(Handled {
                    outcome: o,
                    repeat,
                    failure,
                });
            }
            Decision::Fork {
                session_id,
                worktree,
            } => {
                // Recorded before it runs: a repeated push is answered from the record and
                // never runs (or pays for) a second fork.
                if let Err(e) = self
                    .deliveries
                    .record(msg.id, Delivery::Delivered, Utc::now())
                {
                    return Some(Handled::failed(
                        SessionMessageOutcome::Failed {
                            error: format!("recording the delivery: {e:#}"),
                        },
                        "state_write",
                    ));
                }
                self.forks.submit(ForkTurn {
                    msg: msg.clone(),
                    session_id,
                    worktree,
                });
                return None;
            }
            Decision::Paste { target, text, .. } => (target, text),
            Decision::Turn {
                session_id,
                worktree,
                text,
            } => return Some(self.run_turn(msg, &session_id, &worktree, text).await),
        };
        // Before anything is typed: a record that cannot be written means no paste.
        if let Err(e) = self.deliveries.record(msg.id, Delivery::Typing, Utc::now()) {
            return Some(Handled::failed(
                SessionMessageOutcome::Failed {
                    error: format!("recording the delivery: {e:#}"),
                },
                "state_write",
            ));
        }
        let pasted = tokio::task::spawn_blocking({
            let input = self.input.clone();
            let key = msg.id.to_string();
            let text = text.clone();
            move || input.paste(&target, &key, &text)
        })
        .await;
        Some(match pasted {
            Ok(Ok(())) => {
                let at = Utc::now();
                if let Err(e) = self.deliveries.record(msg.id, Delivery::Delivered, at) {
                    // It was typed. The record stays `typing`, so a repeat after a restart is
                    // answered `interrupted`: never typed twice.
                    tracing::error!(id = %msg.id, error = %format!("{e:#}"), "delivered, but not recorded as delivered");
                }
                self.registry
                    .lock()
                    .await
                    .mark_input_sent(msg.launch_id, at, msg.id, &text);
                Handled {
                    outcome: SessionMessageOutcome::Delivered,
                    repeat: false,
                    failure: None,
                }
            }
            Ok(Err(e)) => {
                // Nothing was typed: forget the id so it is not `typing`.
                if let Err(r) = self.deliveries.remove(msg.id) {
                    tracing::error!(id = %msg.id, error = %format!("{r:#}"), "a failed paste stays recorded as typing");
                }
                Handled::failed(
                    SessionMessageOutcome::Failed {
                        error: format!("{e:#}"),
                    },
                    "paste",
                )
            }
            // The paste panicked or was cancelled part way: whether it arrived is unknown.
            Err(e) => {
                tracing::error!(id = %msg.id, error = %e, "paste task did not finish");
                Handled::failed(SessionMessageOutcome::Interrupted, "interrupted")
            }
        })
    }

    /// A message to a headless session is one resume turn (ADR 0062). `typing` is recorded before
    /// the spawn and `delivered` once the child runs, so it is never typed twice.
    async fn run_turn(
        &mut self,
        msg: &SessionMessage,
        session_id: &str,
        worktree: &std::path::Path,
        text: String,
    ) -> Handled {
        let Some(headless) = self.headless.clone() else {
            return Handled::failed(
                SessionMessageOutcome::Failed {
                    error: "a headless session on a companion not in headless mode".into(),
                },
                "not_headless",
            );
        };
        if headless.is_running(msg.launch_id) {
            return Handled {
                outcome: SessionMessageOutcome::NotIdle,
                repeat: false,
                failure: None,
            };
        }
        if let Err(e) = self.deliveries.record(msg.id, Delivery::Typing, Utc::now()) {
            return Handled::failed(
                SessionMessageOutcome::Failed {
                    error: format!("recording the delivery: {e:#}"),
                },
                "state_write",
            );
        }
        let at = Utc::now();
        // Before the turn starts, so the prompt it reports is tagged with this message's id.
        self.registry
            .lock()
            .await
            .mark_input_sent(msg.launch_id, at, msg.id, &text);
        let args = crate::kernel::claude::resume_args(session_id, &text, &self.permission_mode);
        match headless
            .start(msg.launch_id, session_id, worktree, args, text)
            .await
        {
            Ok(()) => {
                if let Err(e) = self
                    .deliveries
                    .record(msg.id, Delivery::Delivered, Utc::now())
                {
                    tracing::error!(id = %msg.id, error = %format!("{e:#}"), "delivered, but not recorded as delivered");
                }
                Handled {
                    outcome: SessionMessageOutcome::Delivered,
                    repeat: false,
                    failure: None,
                }
            }
            Err(e) => {
                let _ = self.deliveries.remove(msg.id);
                Handled::failed(
                    SessionMessageOutcome::Failed {
                        error: format!("{e:#}"),
                    },
                    "turn",
                )
            }
        }
    }

    /// `stop` (ADR 0062): checks the paired user and the launch, skips only the idle check, kills
    /// the turn's process group (headless) or the tmux session (interactive), and ends the
    /// session. A repeat is answered from the delivery record without killing again.
    async fn stop(&mut self, msg: &SessionMessage) -> Handled {
        let refused = |reason| Handled {
            outcome: SessionMessageOutcome::Refused { reason },
            repeat: false,
            failure: None,
        };
        if msg.user != self.paired_user {
            return refused(rubato_protocol::MessageRefusal::NotPairedUser);
        }
        if matches!(self.deliveries.get(msg.id), Some(Delivery::Delivered)) {
            return Handled {
                outcome: SessionMessageOutcome::Delivered,
                repeat: true,
                failure: None,
            };
        }
        let entry = self.registry.lock().await.by_launch(msg.launch_id).cloned();
        let Some(entry) = entry.filter(|e| !e.ended) else {
            return refused(rubato_protocol::MessageRefusal::NotLaunchedHere);
        };
        // A fork turn of the session goes with it (ADR 0057); it is not the session's own turn.
        if self.forks.stop(msg.launch_id) {
            tracing::info!(launch_id = %msg.launch_id, "room fork turn killed by stop");
        }
        let killed_turn = self
            .headless
            .as_ref()
            .is_some_and(|h| h.kill(msg.launch_id, crate::kernel::claude::KillReason::Stop));
        if !killed_turn {
            if let (Some(t), Some(name)) = (self.tmux.clone(), entry.tmux_session.clone()) {
                let _ = tokio::task::spawn_blocking(move || t.kill_session(&name)).await;
            }
            // An idle headless session, or a tmux one: nothing reports the end, so say it here.
            let mut reg = self.registry.lock().await;
            if let Some(e) = reg.find_by_launch(msg.launch_id) {
                e.status = rubato_protocol::SessionStatus::Ended;
                e.ended = true;
            }
            let snap = reg.snapshot();
            let _ = self.out.send(CompanionMessage::Sessions { sessions: snap });
        }
        let _ = self
            .deliveries
            .record(msg.id, Delivery::Delivered, Utc::now());
        tracing::info!(id = %msg.id, launch_id = %msg.launch_id, killed_turn, "session stopped by its owner");
        Handled {
            outcome: SessionMessageOutcome::Delivered,
            repeat: false,
            failure: None,
        }
    }

    /// The log line and, for a failure, `rubato.session_input.failed{rubato.reason}`.
    async fn report(&self, msg: &SessionMessage, h: &Handled) {
        if let Some(reason) = h.failure {
            self.metrics
                .session_input_failed
                .add(1, &[KeyValue::new(attr::DROP_REASON, reason)]);
        }
        let repeat = h.repeat;
        match &h.outcome {
            SessionMessageOutcome::Delivered => {
                let chars = msg.kind.text().chars().count();
                tracing::info!(id = %msg.id, launch_id = %msg.launch_id, chars, repeat, "session message delivered");
            }
            SessionMessageOutcome::NotIdle => {
                let last = last_hook(&*self.registry.lock().await, msg.launch_id);
                tracing::info!(id = %msg.id, launch_id = %msg.launch_id, last_hook = ?last, "session message not idle");
            }
            SessionMessageOutcome::Refused { reason } => {
                tracing::warn!(id = %msg.id, launch_id = %msg.launch_id, user = %msg.user, reason = reason.name(), "session message refused");
            }
            SessionMessageOutcome::Interrupted => {
                tracing::warn!(id = %msg.id, launch_id = %msg.launch_id, repeat, "session message interrupted: delivery unknown");
            }
            SessionMessageOutcome::Failed { error } => {
                tracing::warn!(id = %msg.id, launch_id = %msg.launch_id, reason = h.failure, error = %error, "session message failed");
            }
        }
    }
}

/// One message handled.
struct Handled {
    outcome: SessionMessageOutcome,
    /// Answered from `deliveries.json`, without typing.
    repeat: bool,
    /// The `rubato.reason` it counts under on `rubato.session_input.failed`, if it failed.
    failure: Option<&'static str>,
}

impl Handled {
    fn failed(outcome: SessionMessageOutcome, reason: &'static str) -> Self {
        Self {
            outcome,
            repeat: false,
            failure: Some(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::{capture, metric_reader};
    use crate::kernel::tmux::Tmux;
    use crate::sessions::LaunchRecord;
    use rubato_protocol::otel::metric;
    use rubato_protocol::{HookEvent, IssueRef, SessionMessageKind};
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    const USER: &str = "m0x6A";

    /// A stand-in `tmux` that records each call and what `load-buffer` read, and fails
    /// `paste-buffer` while `<dir>/fail` exists. Written by a child `sh` (see
    /// `launch::arm::tests`).
    fn standin(dir: &Path) -> Tmux {
        let script = dir.join("tmux");
        let body = format!(
            "#!/bin/sh\necho \"$*\" >> '{d}/calls'\ncase \"$1\" in\n  load-buffer) cat >> '{d}/typed' ;;\n  paste-buffer) [ -e '{d}/fail' ] && {{ echo 'no such pane' >&2; exit 1; }} ;;\nesac\nexit 0\n",
            d = dir.display()
        );
        let ok = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg(&body)
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        Tmux::new(script.to_string_lossy(), None)
    }

    fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    struct Fixture {
        task: SessionInputTask,
        out: mpsc::UnboundedReceiver<CompanionMessage>,
        dir: tempfile::TempDir,
        reader: metric_reader::Reader,
    }

    fn fixture(server_caps: &[&str]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::new(Uuid::nil(), USER);
        reg.restore_launches(
            vec![LaunchRecord {
                launch_id: Uuid::from_u128(7),
                session_id: "s7".into(),
                worktree: PathBuf::from("/w"),
                pane_id: Some("%7".into()),
                tmux_session: Some("pantry@issue-7-restock".into()),
                issue: IssueRef::new("acme/pantry", 7),
                branch: None,
                spend_cap_usd: None,
                plan_path: None,
            }],
            None,
            false,
            Utc::now(),
        );
        reg.apply_hook(
            &HookEvent::from_payload(
                serde_json::json!({"session_id": "s7", "hook_event_name": "Stop", "cwd": "/w"}),
                Utc::now(),
            )
            .unwrap(),
        );
        let (out_tx, out) = mpsc::unbounded_channel();
        let (reader, metrics) = metric_reader::start();
        let registry = Arc::new(Mutex::new(reg));
        let task = SessionInputTask {
            paired_user: USER.into(),
            registry: registry.clone(),
            out: out_tx.clone(),
            limits: Arc::new(std::sync::Mutex::new(Limits::default())),
            server_capabilities: Arc::new(std::sync::Mutex::new(
                server_caps.iter().map(|c| (*c).to_owned()).collect(),
            )),
            input: Arc::new(standin(dir.path())),
            metrics: Arc::new(metrics),
            deliveries: Deliveries::open(dir.path()).unwrap(),
            skill_dir: dir.path().join("skill"),
            headless: None,
            tmux: None,
            permission_mode: "acceptEdits".into(),
            forks: crate::session_input::Forks::new(crate::session_input::ForkCtx {
                claude: crate::kernel::claude::Claude {
                    command: "false".into(),
                    timeout: std::time::Duration::from_secs(5),
                },
                registry: registry.clone(),
                out: out_tx.clone(),
                limits: Arc::new(std::sync::Mutex::new(Limits::default())),
                spent: Default::default(),
                running: Default::default(),
            }),
        };
        Fixture {
            task,
            out,
            dir,
            reader,
        }
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
            created_at: Utc::now(),
        }
    }

    fn result(out: &mut mpsc::UnboundedReceiver<CompanionMessage>) -> SessionMessageResult {
        match out.try_recv() {
            Ok(CompanionMessage::SessionMessageResult(r)) => r,
            other => panic!("expected a session message result, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_message_is_typed_once_and_a_repeat_is_answered_from_the_record() {
        let (spans, _g) = capture::spans();
        let mut f = fixture(&[capability::SESSION_MESSAGES]);
        let m = message(1, "line one\nline two\nline three");
        f.task.on_message(&m).await;
        let r = result(&mut f.out);
        assert_eq!(
            (r.message_id, r.outcome),
            (m.id, SessionMessageOutcome::Delivered)
        );
        let key = format!("rubato-{}", m.id);
        assert_eq!(
            calls(f.dir.path()),
            [
                format!("load-buffer -b {key} -"),
                format!("paste-buffer -p -d -b {key} -t %7"),
                "send-keys -t %7 Enter".to_owned(),
            ]
        );
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("typed")).unwrap(),
            "line one\nline two\nline three"
        );
        assert_eq!(f.task.deliveries.get(m.id), Some(Delivery::Delivered));
        assert!(
            f.task
                .registry
                .lock()
                .await
                .by_launch(m.launch_id)
                .unwrap()
                .input_sent_at
                .is_some()
        );

        // Pushed again (a Hello after a half-open socket): answered, not typed.
        f.task.on_message(&m).await;
        assert_eq!(result(&mut f.out).outcome, SessionMessageOutcome::Delivered);
        assert_eq!(calls(f.dir.path()).len(), 3, "typed once");

        let spans = spans.lock().unwrap();
        let deliver: Vec<_> = spans
            .iter()
            .filter(|s| s.message == span::SESSION_INPUT_DELIVER)
            .collect();
        assert_eq!(deliver.len(), 2, "{spans:?}");
        for s in deliver {
            assert_eq!(
                s.fields.get(attr::SESSION_MESSAGE_ID),
                Some(&m.id.to_string())
            );
            assert_eq!(
                s.fields.get(attr::SESSION_MESSAGE_KIND).map(String::as_str),
                Some("chat")
            );
            assert_eq!(
                s.fields.get(attr::OUTCOME).map(String::as_str),
                Some("delivered")
            );
        }
    }

    /// An older server drops an envelope it cannot decode: it gets no answer, and nothing is
    /// typed for it.
    #[tokio::test(flavor = "current_thread")]
    async fn a_server_without_session_messages_gets_no_answer() {
        let mut f = fixture(&[]);
        f.task.on_message(&message(1, "hi")).await;
        assert!(f.out.try_recv().is_err());
        assert!(calls(f.dir.path()).is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_paste_is_failed_counted_and_not_left_typing() {
        let mut f = fixture(&[capability::SESSION_MESSAGES]);
        std::fs::write(f.dir.path().join("fail"), "").unwrap();
        let m = message(1, "hi");
        f.task.on_message(&m).await;
        match result(&mut f.out).outcome {
            SessionMessageOutcome::Failed { error } => {
                assert!(error.contains("paste-buffer"), "{error}");
            }
            o => panic!("expected failed, got {o:?}"),
        }
        assert_eq!(f.task.deliveries.get(m.id), None);
        assert!(
            !calls(f.dir.path())
                .iter()
                .any(|c| c.starts_with("send-keys")),
            "no Enter after a failed paste"
        );
        let points = f.reader.points(metric::SESSION_INPUT_FAILED);
        assert_eq!(points.len(), 1, "{points:?}");
        assert_eq!(points[0].attrs[attr::DROP_REASON], "paste");
        assert_eq!(points[0].value, 1);

        // The paste works again: the same id is typed, since nothing was typed before.
        std::fs::remove_file(f.dir.path().join("fail")).unwrap();
        f.task.on_message(&m).await;
        assert_eq!(result(&mut f.out).outcome, SessionMessageOutcome::Delivered);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_id_left_typing_is_interrupted_and_counted() {
        let mut f = fixture(&[capability::SESSION_MESSAGES]);
        let m = message(1, "hi");
        f.task
            .deliveries
            .record(m.id, Delivery::Typing, Utc::now())
            .unwrap();
        f.task.on_message(&m).await;
        assert_eq!(
            result(&mut f.out).outcome,
            SessionMessageOutcome::Interrupted
        );
        assert!(calls(f.dir.path()).is_empty(), "nothing typed");
        let points = f.reader.points(metric::SESSION_INPUT_FAILED);
        assert_eq!(points[0].attrs[attr::DROP_REASON], "interrupted");
    }
}
