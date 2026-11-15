//! The spend task: spend is summed here, off the runtime's dispatch loop, so hooks and server
//! messages keep flowing while a transcript is read or a capped session is stopped (#172).
//!
//! The dispatch loop sends one `SpendJob` per hook. Each time the task wakes it takes every job
//! already queued and keeps only the newest per `session_id`: a burst of tool calls costs one
//! sum per session, not one per hook, and the sum it does run sees the newest transcript.

use anyhow::Result;
use rubato_protocol::{HookEvent, IssueRef};
use std::future::Future;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// What one hook leaves for the spend step: the hook, and the session's issue, transcript and
/// spend cap as the hook arm read them under the registry lock.
#[derive(Debug, Clone)]
pub struct SpendJob {
    pub ev: HookEvent,
    pub outcome: (Option<IssueRef>, Option<PathBuf>, Option<f64>),
}

/// Run `step` for each job, coalesced per wake. Ends when every sender is gone.
pub async fn run<F, Fut>(mut rx: mpsc::UnboundedReceiver<SpendJob>, mut step: F) -> Result<()>
where
    F: FnMut(SpendJob) -> Fut,
    Fut: Future<Output = ()>,
{
    while let Some(first) = rx.recv().await {
        for job in coalesce(first, &mut rx) {
            step(job).await;
        }
    }
    Ok(())
}

/// `first` and everything already queued, newest job per session, in first-seen order.
fn coalesce(first: SpendJob, rx: &mut mpsc::UnboundedReceiver<SpendJob>) -> Vec<SpendJob> {
    let mut jobs = vec![first];
    while let Ok(job) = rx.try_recv() {
        match jobs
            .iter_mut()
            .find(|j| j.ev.session_id == job.ev.session_id)
        {
            Some(slot) => *slot = job,
            None => jobs.push(job),
        }
    }
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn job(session: &str, transcript: &str) -> SpendJob {
        SpendJob {
            ev: HookEvent::from_payload(
                serde_json::json!({"session_id": session, "hook_event_name": "PostToolUse", "cwd": "/x"}),
                chrono::Utc::now(),
            )
            .unwrap(),
            outcome: (None, Some(PathBuf::from(transcript)), None),
        }
    }

    #[tokio::test]
    async fn pending_jobs_coalesce_by_session() {
        let (tx, rx) = mpsc::unbounded_channel();
        for (s, t) in [("s1", "a"), ("s2", "x"), ("s1", "b"), ("s1", "c")] {
            tx.send(job(s, t)).unwrap();
        }
        drop(tx);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        run(rx, move |j: SpendJob| {
            s.lock()
                .unwrap()
                .push((j.ev.session_id, j.outcome.1.unwrap()));
            async {}
        })
        .await
        .unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                ("s1".to_owned(), PathBuf::from("c")),
                ("s2".to_owned(), PathBuf::from("x")),
            ],
            "one step per session, the newest job for each"
        );
    }
}
