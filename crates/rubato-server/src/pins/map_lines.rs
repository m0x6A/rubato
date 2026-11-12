//! Pins from `Map:` lines (ADR 0026). The listing hands over the lines of trusted authors; each
//! (issue, target) is applied once. An issue that is pinned anywhere already is left as it is,
//! and after a person unpins an issue the same line does not pin it again.

use super::store;
use crate::kernel::state::App;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{IssuePin, IssueRef, MapRef, SystemMap};

/// Why a target cannot be pinned: `unknown_id` or `kind_mismatch`, with a message naming it.
pub type Refusal = (&'static str, String);

/// Whether `target` is an id of `map` of the right kind. A node id pinned as a seam would never
/// show on the map. Shared by the pin route and `Map:` lines.
pub fn check_target(map: &SystemMap, version: &str, target: &MapRef) -> Result<(), Refusal> {
    let id = target.id();
    if !map.ids().contains(id) {
        return Err(("unknown_id", format!("{id} is not an id in map {version}")));
    }
    let (kind, of_kind) = if target.is_node() {
        ("part", map.node_ids().contains(id))
    } else {
        ("seam", map.seam_ids().contains(id))
    };
    if !of_kind {
        return Err((
            "kind_mismatch",
            format!("{id} is not a {kind} in map {version}"),
        ));
    }
    Ok(())
}

/// One `Map:` line to apply.
#[derive(Debug, Clone)]
pub struct Line {
    pub issue: IssueRef,
    pub target: MapRef,
    pub author: String,
}

/// Apply this tick's lines. Never fails the poll: a store error is logged and the line is tried
/// again next tick. Counts `rubato.backlog.map_lines` by outcome; returns how many pinned.
pub async fn apply_map_lines(app: &App, map: &SystemMap, version: &str, lines: &[Line]) -> usize {
    let mut pinned = 0;
    for line in lines {
        let outcome = match apply_one(app, map, version, line).await {
            Ok(Some(outcome)) => outcome,
            Ok(None) => continue,
            Err(e) => {
                tracing::error!(rubato.issue = %line.issue, error = %e, "applying a Map: line failed");
                continue;
            }
        };
        if outcome == "pinned" {
            pinned += 1;
            tracing::info!(
                rubato.issue = %line.issue,
                target = line.target.id(),
                author = %line.author,
                "pinned by its Map: line"
            );
        }
        app.metrics
            .backlog_map_lines
            .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
    }
    if pinned > 0 {
        app.notify("pin");
    }
    pinned
}

/// `None` when the line was applied on an earlier tick (not counted again).
async fn apply_one(
    app: &App,
    map: &SystemMap,
    version: &str,
    line: &Line,
) -> anyhow::Result<Option<&'static str>> {
    if store::map_line_applied(&app.db, &line.issue, &line.target).await? {
        return Ok(None);
    }
    if let Err((reason, msg)) = check_target(map, version, &line.target) {
        // Not recorded: the line may name a part the map gains later.
        tracing::debug!(rubato.issue = %line.issue, reason, "{msg}");
        return Ok(Some(reason));
    }
    let outcome = if store::is_pinned(&app.db, &line.issue).await? {
        "already_pinned"
    } else {
        store::add_pin(
            &app.db,
            &IssuePin {
                issue: line.issue.clone(),
                target: line.target.clone(),
                pinned_by: line.author.clone(),
                pinned_at: Utc::now(),
            },
        )
        .await?;
        "pinned"
    };
    store::record_map_line(&app.db, &line.issue, &line.target, outcome).await?;
    Ok(Some(outcome))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::metric_reader;
    use rubato_protocol::otel::metric;
    use rubato_protocol::{MapNode, NodeKind, Seam};
    use std::sync::Arc;

    fn map() -> SystemMap {
        let node = |id: &str| MapNode {
            id: id.into(),
            kind: NodeKind::Context,
            repo: Some("o/r".into()),
            path: None,
            description: None,
        };
        SystemMap {
            version: 1,
            name: "t".into(),
            owner_rule: None,
            repos: vec![],
            nodes: vec![node("web"), node("rubato-server")],
            seams: vec![Seam {
                id: "web-server".into(),
                from: "web".into(),
                to: "rubato-server".into(),
                carries: None,
                owner: "rubato-server".into(),
                guard: None,
                description: None,
                flows: vec![],
            }],
            scenarios: vec![],
        }
    }

    async fn app() -> (App, metric_reader::Reader, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(
            pool,
            Config::for_tests(url, "m0x6A"),
            Arc::new(FakeGitHub::new()),
        );
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        (app, reader, tmp)
    }

    fn line(n: u64, target: MapRef) -> Line {
        Line {
            issue: IssueRef::new("o/r", n),
            target,
            author: "m0x6A".into(),
        }
    }

    fn outcomes(reader: &metric_reader::Reader) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = reader
            .points(metric::BACKLOG_MAP_LINES)
            .into_iter()
            .map(|p| {
                (
                    p.attrs.get(attr::OUTCOME).cloned().unwrap_or_default(),
                    p.value,
                )
            })
            .collect();
        v.sort();
        v
    }

    /// The guard of ADR 0026: a line pins once, and an unpin sticks. Without
    /// `map_line_applied` the third call pins the issue again and this fails.
    #[tokio::test]
    async fn a_line_pins_once_and_an_unpin_sticks() {
        let (app, reader, _tmp) = app().await;
        let m = map();
        let lines = [line(1, MapRef::seam("web-server"))];
        assert_eq!(apply_map_lines(&app, &m, "v1", &lines).await, 1);
        let pins = store::list_pins(&app.db).await.unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].target, MapRef::seam("web-server"));
        assert_eq!(pins[0].pinned_by, "m0x6A");

        // The next tick lists the same line: nothing changes and nothing is counted again.
        assert_eq!(apply_map_lines(&app, &m, "v1", &lines).await, 0);
        // A person unpins it; the line is still in the body.
        store::remove_pin(&app.db, &lines[0].issue, &lines[0].target)
            .await
            .unwrap();
        assert_eq!(apply_map_lines(&app, &m, "v1", &lines).await, 0);
        assert!(
            store::list_pins(&app.db).await.unwrap().is_empty(),
            "the unpin sticks"
        );
        assert_eq!(outcomes(&reader), [("pinned".to_string(), 1)]);
    }

    #[tokio::test]
    async fn unknown_ids_wrong_kinds_and_pinned_issues_are_counted_not_pinned() {
        let (app, reader, _tmp) = app().await;
        let m = map();
        store::add_pin(
            &app.db,
            &IssuePin {
                issue: IssueRef::new("o/r", 4),
                target: MapRef::node("web"),
                pinned_by: "someone".into(),
                pinned_at: Utc::now(),
            },
        )
        .await
        .unwrap();
        let lines = [
            line(2, MapRef::node("billing")),
            line(3, MapRef::node("web-server")),
            line(4, MapRef::node("rubato-server")),
        ];
        assert_eq!(apply_map_lines(&app, &m, "v1", &lines).await, 0);
        let pins = store::list_pins(&app.db).await.unwrap();
        assert_eq!(pins.len(), 1, "#4 keeps the pin a person gave it");
        assert_eq!(pins[0].target, MapRef::node("web"));
        assert_eq!(
            outcomes(&reader),
            [
                ("already_pinned".to_string(), 1),
                ("kind_mismatch".to_string(), 1),
                ("unknown_id".to_string(), 1)
            ]
        );
        // An unknown id is not recorded: once the map gains `billing`, the line pins.
        let mut bigger = m.clone();
        bigger.nodes.push(MapNode {
            id: "billing".into(),
            kind: NodeKind::Context,
            repo: None,
            path: None,
            description: None,
        });
        assert_eq!(apply_map_lines(&app, &bigger, "v2", &lines[..1]).await, 1);
        println!("checked {} lines", lines.len() + 1);
    }
}
