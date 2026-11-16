//! `report_status`'s throttle and its drain (plan 220 §4). A call puts the newest report for its
//! launch in a map; the drain empties the map every 5 s. Nothing here touches the registry.

use super::AgentMcp;
use opentelemetry::KeyValue;
use rubato_protocol::limits::{AGENT_DOING_MAX_CHARS, AGENT_PARTS_MAX};
use rubato_protocol::otel::attr;
use rubato_protocol::{AgentReport, CompanionMessage, capability, is_map_id};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Pending(Mutex<HashMap<Uuid, AgentReport>>);

impl Pending {
    /// Keep `r` as its launch's newest. Says whether it replaced one not yet sent.
    pub fn put(&self, r: AgentReport) -> bool {
        self.0.lock().unwrap().insert(r.launch_id, r).is_some()
    }

    fn take(&self) -> Vec<AgentReport> {
        self.0.lock().unwrap().drain().map(|(_, r)| r).collect()
    }

    #[cfg(test)]
    pub fn peek(&self) -> Vec<AgentReport> {
        self.0.lock().unwrap().values().cloned().collect()
    }
}

/// One line, at most [`AGENT_DOING_MAX_CHARS`] characters, the last one `…` when cut.
pub(super) fn one_line(doing: &str) -> String {
    let line = doing
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() <= AGENT_DOING_MAX_CHARS {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(AGENT_DOING_MAX_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// The first [`AGENT_PARTS_MAX`] entries that are map ids, and the ones dropped. Whether an id
/// exists in the map is the server's check.
pub(super) fn parts(given: &[String]) -> (Vec<String>, Vec<String>) {
    let (ok, bad): (Vec<_>, Vec<_>) = given.iter().cloned().partition(|p| is_map_id(p));
    let mut kept = ok;
    let mut dropped = bad;
    if kept.len() > AGENT_PARTS_MAX {
        dropped.extend(kept.split_off(AGENT_PARTS_MAX));
    }
    (kept, dropped)
}

/// Send every pending report when the server takes `agent_mcp`; drop them otherwise, so nothing
/// leaves later on a Welcome that does announce it.
pub(super) fn drain_once(st: &AgentMcp) {
    let reports = st.reports.take();
    if reports.is_empty() {
        return;
    }
    let takes = st
        .server_capabilities
        .lock()
        .unwrap()
        .iter()
        .any(|c| c == capability::AGENT_MCP);
    for r in reports {
        if takes
            && st
                .out
                .send(CompanionMessage::AgentReport(r.clone()))
                .is_ok()
        {
            st.metrics
                .mcp_reports
                .add(1, &[KeyValue::new(attr::OUTCOME, "sent")]);
        } else {
            let reason = if takes {
                "outbox_closed"
            } else {
                "server_too_old"
            };
            tracing::info!(rubato.launch_id = %r.launch_id, rubato.reason = reason, "agent report not sent: server does not take agent_mcp");
            st.metrics.mcp_reports.add(
                1,
                &[
                    KeyValue::new(attr::OUTCOME, "not_sent"),
                    KeyValue::new(attr::DROP_REASON, reason),
                ],
            );
        }
    }
}
