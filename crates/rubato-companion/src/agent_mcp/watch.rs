//! Launches that never asked (plan 220 §4). If Claude Code cannot start the MCP server it goes on
//! without it and the companion sees no request at all, so silence has to be counted: once per
//! launch with no authenticated request five minutes after it started.

use super::AgentMcp;
use chrono::{DateTime, Duration, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use std::collections::HashSet;
use std::sync::Mutex;
use uuid::Uuid;

/// How long a launch may go without any MCP request before it is counted.
pub const GRACE_MINUTES: i64 = 5;

pub(super) struct Initialized {
    seen: Mutex<HashSet<Uuid>>,
    counted: Mutex<HashSet<Uuid>>,
}

impl Initialized {
    pub fn new(restored: HashSet<Uuid>) -> Self {
        Self {
            seen: Mutex::new(restored),
            counted: Mutex::new(HashSet::new()),
        }
    }

    /// Any authenticated request with a known launch. Says whether it is the launch's first.
    pub fn mark(&self, launch_id: Uuid) -> bool {
        self.seen.lock().unwrap().insert(launch_id)
    }
}

pub(super) async fn tick(st: &AgentMcp, now: DateTime<Utc>) {
    let started: Vec<(Uuid, DateTime<Utc>)> = {
        let reg = st.registry.lock().await;
        reg.entries()
            .iter()
            .filter(|e| !e.ended)
            .filter_map(|e| Some((e.launch_id?, e.assignment.as_ref()?.launched_at)))
            .collect()
    };
    let seen = st.initialized.seen.lock().unwrap().clone();
    let mut counted = st.initialized.counted.lock().unwrap();
    for (id, at) in started {
        if seen.contains(&id)
            || counted.contains(&id)
            || now - at < Duration::minutes(GRACE_MINUTES)
        {
            continue;
        }
        counted.insert(id);
        tracing::warn!(rubato.launch_id = %id, "launched session has not initialized mcp");
        st.metrics.mcp_not_initialized.add(
            1,
            &[KeyValue::new(
                attr::COMPANION_ID,
                st.companion_id.to_string(),
            )],
        );
    }
}
