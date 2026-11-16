//! The plan mirror task (ADR 0053, plan 122 §D4). The dispatch loop hands it the session id of
//! each settle hook (`Stop`, `Notification(idle_prompt)`, `SessionEnd`) and goes on; the task
//! reads what it needs from the registry, releases the lock, reads the bundle off the runtime
//! (`spawn_blocking`, with a deadline for a slow disk), and sends `CompanionMessage::Draft` when
//! the bundle hash differs from the last one sent for that launch, to a server that announced
//! `drafts`. Essential: ends only when the runtime drops the sender.

use super::bundle::{Read, read_bundle};
use crate::sessions::Registry;
use chrono::Utc;
use rubato_protocol::{
    CompanionMessage, DraftBundle, DraftFile, DraftRefusal, HookEvent, HookEventKind, IssueRef,
    Limits, bundle_hash, capability,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

/// The hook is a settle point: the agent finished a version, so the files say what it wrote.
/// Nothing is mirrored mid-turn.
pub fn is_settle_point(ev: &HookEvent) -> bool {
    match ev.hook_event_name {
        HookEventKind::Stop | HookEventKind::SessionEnd => true,
        HookEventKind::Notification => ev.notification_type.as_deref() == Some("idle_prompt"),
        _ => false,
    }
}

/// The dispatch arm: queue the session of a settle hook. Never waits.
pub fn on_hook(ev: &HookEvent, mirror: &mpsc::UnboundedSender<String>) {
    if is_settle_point(ev) {
        let _ = mirror.send(ev.session_id.clone());
    }
}

/// What a launched session's mirror needs, copied out under the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub launch_id: Uuid,
    pub session_id: String,
    pub issue: IssueRef,
    pub branch: String,
    pub worktree: PathBuf,
    pub plan_path: String,
    /// The bundle a restored draft put in the worktree: the first settle compares with it.
    pub restored_hash: Option<String>,
}

impl Registry {
    /// The mirror target of a session: only one launched from Rubato with a plan path.
    pub fn mirror_target(&self, session_id: &str) -> Option<Target> {
        let e = self.by_session(session_id)?;
        Some(Target {
            launch_id: e.launch_id?,
            session_id: session_id.to_owned(),
            issue: e.issue.clone()?,
            branch: e.branch.clone()?,
            worktree: e.worktree.clone().unwrap_or_else(|| e.cwd.clone()),
            plan_path: e.plan_path.clone()?,
            restored_hash: e.restored_hash.clone(),
        })
    }
}

pub struct PlanMirrorTask {
    pub registry: Arc<Mutex<Registry>>,
    pub out: mpsc::UnboundedSender<CompanionMessage>,
    /// `draft_max_bytes`, updated by each Welcome.
    pub limits: tokio::sync::watch::Receiver<Limits>,
    /// What the last Welcome announced: no `drafts`, no bundle.
    pub server_capabilities: Arc<std::sync::Mutex<Vec<String>>>,
    /// How long one bundle read may take before this settle point is skipped.
    pub read_timeout: Duration,
    /// The last bundle hash sent per launch. In memory: after a restart one bundle is sent
    /// again, which the server stores once.
    pub last_sent: HashMap<Uuid, String>,
}

pub async fn run(
    mut settles: mpsc::UnboundedReceiver<String>,
    mut task: PlanMirrorTask,
) -> anyhow::Result<()> {
    while let Some(session_id) = settles.recv().await {
        task.on_settle(&session_id).await;
    }
    Ok(())
}

impl PlanMirrorTask {
    pub async fn on_settle(&mut self, session_id: &str) {
        let Some(t) = self.registry.lock().await.mirror_target(session_id) else {
            return;
        };
        let takes_drafts = self
            .server_capabilities
            .lock()
            .unwrap()
            .iter()
            .any(|c| c == capability::DRAFTS);
        if !takes_drafts {
            tracing::debug!(rubato.issue = %t.issue, "draft bundle not sent: the server did not announce drafts");
            return;
        }
        let max_bytes = self.limits.borrow().draft_max_bytes;
        let read = tokio::time::timeout(
            self.read_timeout,
            tokio::task::spawn_blocking({
                let (worktree, plan_path) = (t.worktree.clone(), t.plan_path.clone());
                move || read_bundle(&worktree, &plan_path, max_bytes)
            }),
        )
        .await;
        let read = match read {
            Ok(Ok(Ok(r))) => r,
            Ok(Ok(Err(e))) => {
                tracing::warn!(rubato.issue = %t.issue, error = %format!("{e:#}"), "draft bundle not read");
                return;
            }
            Ok(Err(e)) => {
                tracing::warn!(rubato.issue = %t.issue, error = %e, "draft bundle read did not finish");
                return;
            }
            Err(_) => {
                tracing::warn!(rubato.issue = %t.issue, timeout_secs = self.read_timeout.as_secs_f64(), "draft bundle read timed out");
                return;
            }
        };
        let (bundle, bytes) = match read {
            Read::NoPlan => return,
            Read::Bundle {
                plan_path,
                files,
                missing,
                bytes,
            } => {
                let hash = bundle_hash(&plan_path, &files);
                let files = files
                    .into_iter()
                    .map(|(path, text)| DraftFile { path, text })
                    .collect();
                (self.bundle(&t, files, missing, hash, None), bytes)
            }
            Read::TooLarge { bytes, missing } => (
                self.bundle(
                    &t,
                    Vec::new(),
                    missing,
                    String::new(),
                    Some(DraftRefusal::TooLarge),
                ),
                bytes,
            ),
        };
        // Before anything was sent for this launch, the bundle a restored draft wrote counts as
        // sent: the server already holds it as the draft's newest revision.
        let last = self
            .last_sent
            .get(&t.launch_id)
            .or(t.restored_hash.as_ref());
        if last == Some(&bundle.bundle_hash) {
            tracing::debug!(rubato.issue = %t.issue, "draft bundle unchanged");
            return;
        }
        self.last_sent
            .insert(t.launch_id, bundle.bundle_hash.clone());
        if bundle.refused.is_some() {
            tracing::warn!(rubato.issue = %t.issue, rubato.branch = %t.branch, bytes, max_bytes, "draft bundle too large");
        } else {
            tracing::info!(rubato.issue = %t.issue, rubato.branch = %t.branch, files = bundle.files.len(), missing = bundle.missing.len(), bytes, "draft bundle sent");
        }
        let _ = self.out.send(CompanionMessage::Draft(bundle));
    }

    fn bundle(
        &self,
        t: &Target,
        files: Vec<DraftFile>,
        missing: Vec<String>,
        bundle_hash: String,
        refused: Option<DraftRefusal>,
    ) -> DraftBundle {
        DraftBundle {
            launch_id: t.launch_id,
            session_id: Some(t.session_id.clone()),
            issue: t.issue.clone(),
            branch: t.branch.clone(),
            files,
            missing,
            bundle_hash,
            refused,
            at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::LaunchRecord;

    fn write(root: &std::path::Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn task(
        worktree: &std::path::Path,
        caps: &[&str],
        plan_path: Option<&str>,
    ) -> (PlanMirrorTask, mpsc::UnboundedReceiver<CompanionMessage>) {
        let mut reg = Registry::new(Uuid::nil(), "m0x6A");
        reg.restore_launches(
            vec![LaunchRecord {
                launch_id: Uuid::from_u128(7),
                session_id: "s7".into(),
                worktree: worktree.to_path_buf(),
                pane_id: None,
                tmux_session: None,
                issue: IssueRef::new("acme/pantry", 7),
                branch: Some("issue/7-restock".into()),
                spend_cap_usd: None,
                plan_path: plan_path.map(str::to_owned),
                assignment: None,
            }],
            None,
            false,
            Utc::now(),
        );
        let (out, rx) = mpsc::unbounded_channel();
        (
            PlanMirrorTask {
                registry: Arc::new(Mutex::new(reg)),
                out,
                limits: tokio::sync::watch::channel(Limits::default()).1,
                server_capabilities: Arc::new(std::sync::Mutex::new(
                    caps.iter().map(|c| (*c).to_owned()).collect(),
                )),
                read_timeout: Duration::from_secs(5),
                last_sent: HashMap::new(),
            },
            rx,
        )
    }

    fn drafts(rx: &mut mpsc::UnboundedReceiver<CompanionMessage>) -> Vec<DraftBundle> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|m| match m {
                CompanionMessage::Draft(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_settle_hooks_mirror() {
        let ev = |name: &str, nt: Option<&str>| {
            let mut e = HookEvent::from_payload(
                serde_json::json!({"session_id": "s", "hook_event_name": name, "cwd": "/w"}),
                Utc::now(),
            )
            .unwrap();
            e.notification_type = nt.map(str::to_owned);
            e
        };
        assert!(is_settle_point(&ev("Stop", None)));
        assert!(is_settle_point(&ev("SessionEnd", None)));
        assert!(is_settle_point(&ev("Notification", Some("idle_prompt"))));
        for (name, nt) in [
            ("Notification", Some("permission_prompt")),
            ("Notification", None),
            ("UserPromptSubmit", None),
            ("PostToolUse", None),
            ("StopFailure", None),
        ] {
            assert!(!is_settle_point(&ev(name, nt)), "{name} {nt:?}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_changed_bundle_is_sent_once_per_hash() {
        let dir = tempfile::tempdir().unwrap();
        let (mut t, mut rx) = task(dir.path(), &[capability::DRAFTS], Some("docs/plans/7.md"));
        // No plan yet: nothing.
        t.on_settle("s7").await;
        assert!(drafts(&mut rx).is_empty());

        write(
            dir.path(),
            "docs/plans/7.md",
            "# Plan\n\n[design](7-restock/design.md)\n",
        );
        write(dir.path(), "docs/plans/7-restock/design.md", "one\n");
        t.on_settle("s7").await;
        t.on_settle("s7").await;
        let sent = drafts(&mut rx);
        assert_eq!(sent.len(), 1, "unchanged bundles are not sent again");
        let b = &sent[0];
        assert_eq!(
            (b.launch_id, b.branch.as_str()),
            (Uuid::from_u128(7), "issue/7-restock")
        );
        assert_eq!(b.session_id.as_deref(), Some("s7"));
        assert_eq!(
            b.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["docs/plans/7.md", "docs/plans/7-restock/design.md"]
        );
        let files: Vec<(String, String)> = b
            .files
            .iter()
            .map(|f| (f.path.clone(), f.text.clone()))
            .collect();
        assert_eq!(b.bundle_hash, bundle_hash("docs/plans/7.md", &files));

        write(dir.path(), "docs/plans/7-restock/design.md", "two\n");
        t.on_settle("s7").await;
        let sent = drafts(&mut rx);
        assert_eq!(sent.len(), 1);
        assert_ne!(sent[0].bundle_hash, b.bundle_hash);

        // Too large: sent once with no files and no hash, then not again.
        t.limits = tokio::sync::watch::channel(Limits {
            draft_max_bytes: 10,
            ..Limits::default()
        })
        .1;
        t.on_settle("s7").await;
        t.on_settle("s7").await;
        let sent = drafts(&mut rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].refused, Some(DraftRefusal::TooLarge));
        assert!(sent[0].files.is_empty() && sent[0].bundle_hash.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn nothing_without_drafts_a_launch_or_a_plan_path() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "docs/plans/7.md", "# Plan\n");
        let (mut t, mut rx) = task(dir.path(), &[], Some("docs/plans/7.md"));
        t.on_settle("s7").await;
        assert!(drafts(&mut rx).is_empty(), "a server without drafts");
        assert!(
            t.last_sent.is_empty(),
            "so an upgraded server still gets it"
        );

        let (mut t, mut rx) = task(dir.path(), &[capability::DRAFTS], None);
        t.on_settle("s7").await;
        assert!(
            drafts(&mut rx).is_empty(),
            "an older server sent no plan path"
        );

        let (mut t, mut rx) = task(dir.path(), &[capability::DRAFTS], Some("docs/plans/7.md"));
        t.on_settle("not-launched").await;
        assert!(drafts(&mut rx).is_empty(), "a session not launched here");
    }
}
