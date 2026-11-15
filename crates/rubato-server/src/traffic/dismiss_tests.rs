//! Dismissals over HTTP (ADR 0060): who dismissed, once per key, undo, the limit, what the
//! merged map can name, one workspace at a time, and the browsers told on each change.

use super::dismiss::{self, Added};
use crate::kernel::config::Config;
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState, LoadedMap};
use axum::routing::{delete, post};
use rubato_protocol::traffic::{MAX_DISMISSALS, routes};
use rubato_protocol::{
    DiscrepancyKey, ServiceId, SystemMap, TrafficDismissal, TrafficEnd, WebEvent,
};
use std::sync::Arc;

const MAP: &str = include_str!("../../../../docs/design/rubato-map.yml");

async fn serve() -> (String, App, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let cfg = Config::for_tests(url, "m0x6A");
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
    let map: SystemMap = serde_yaml_ng::from_str(MAP).unwrap();
    app.set_map(
        rubato_protocol::DEFAULT_WORKSPACE,
        LoadedMap {
            map,
            version: "v".into(),
            loaded_at: chrono::Utc::now(),
            yaml: String::new(),
        },
    )
    .await;
    // Only this slice's two routes, through its root: a slice never names the composition root.
    let router = axum::Router::new()
        .route(routes::TRAFFIC_DISMISSALS, post(super::dismiss))
        .route(
            &format!("{}/{{id}}", routes::TRAFFIC_DISMISSALS),
            delete(super::undismiss),
        )
        .with_state(app.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), app, tmp)
}

fn ghost() -> DiscrepancyKey {
    DiscrepancyKey::Ghost {
        service: ServiceId::new(Some("shop"), "checkout"),
    }
}

fn changed(rx: &mut tokio::sync::broadcast::Receiver<WebEvent>) -> usize {
    let mut n = 0;
    while let Ok(WebEvent::Changed { reason, .. }) = rx.try_recv() {
        assert_eq!(reason, "traffic");
        n += 1;
    }
    n
}

#[tokio::test(flavor = "current_thread")]
async fn a_person_dismisses_once_and_undoes() {
    let (base, app, _tmp) = serve().await;
    let mut rx = app.web_tx.subscribe();
    let http = reqwest::Client::new();
    let url = format!("{base}{}", routes::TRAFFIC_DISMISSALS);
    let body = serde_json::json!({ "target": ghost() });

    let first = http.post(&url).json(&body).send().await.unwrap();
    assert_eq!(first.status(), 201);
    let d: TrafficDismissal = first.json().await.unwrap();
    assert_eq!((d.author.as_str(), &d.target), ("m0x6A", &ghost()));
    assert_eq!(changed(&mut rx), 1);

    // The same key again: the first row, nothing new, nobody told.
    let again = http.post(&url).json(&body).send().await.unwrap();
    assert_eq!(again.status(), 200);
    assert_eq!(again.json::<TrafficDismissal>().await.unwrap().id, d.id);
    assert_eq!(changed(&mut rx), 0);
    assert_eq!(
        dismiss::list(&app.db, "default").await.unwrap(),
        vec![d.clone()]
    );

    let undo = http.delete(format!("{url}/{}", d.id)).send().await.unwrap();
    assert_eq!(undo.status(), 204);
    assert_eq!(changed(&mut rx), 1);
    assert!(dismiss::list(&app.db, "default").await.unwrap().is_empty());
    let gone = http.delete(format!("{url}/{}", d.id)).send().await.unwrap();
    assert_eq!(gone.status(), 404);
}

#[tokio::test(flavor = "current_thread")]
async fn only_what_the_map_can_name_is_dismissed() {
    let (base, _app, _tmp) = serve().await;
    let http = reqwest::Client::new();
    let url = format!("{base}{}", routes::TRAFFIC_DISMISSALS);
    let post = |target: DiscrepancyKey| {
        http.post(&url)
            .json(&serde_json::json!({ "target": target }))
            .send()
    };
    let part = |id: &str| TrafficEnd::Part { id: id.into() };
    let cases = [
        (
            DiscrepancyKey::QuietSeam {
                seam: "web-server".into(),
            },
            201,
        ),
        (
            DiscrepancyKey::QuietSeam {
                seam: "nope".into(),
            },
            422,
        ),
        (
            DiscrepancyKey::Edge {
                from: part("web"),
                to: part("github"),
            },
            201,
        ),
        (
            DiscrepancyKey::Edge {
                from: part("ghost-part"),
                to: part("web"),
            },
            422,
        ),
        (
            DiscrepancyKey::Edge {
                from: part("web"),
                to: part("web"),
            },
            422,
        ),
        (
            DiscrepancyKey::Ghost {
                service: ServiceId::new(None, &"x".repeat(201)),
            },
            422,
        ),
    ];
    for (target, want) in cases {
        let got = post(target.clone()).await.unwrap().status();
        assert_eq!(got, want, "{target:?}");
    }
    let garbage = http
        .post(&url)
        .header("content-type", "application/json")
        .body(r#"{"target":{"kind":"everything"}}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(garbage.status(), 422);
}

#[tokio::test(flavor = "current_thread")]
async fn a_workspace_sees_only_its_own_and_has_a_limit() {
    let (_base, app, _tmp) = serve().await;
    let now = chrono::Utc::now();
    let Added::New(d) = dismiss::add(&app.db, "default", &ghost(), "m0x6A", now)
        .await
        .unwrap()
    else {
        panic!("new");
    };
    assert!(dismiss::list(&app.db, "other").await.unwrap().is_empty());
    assert!(
        dismiss::remove(&app.db, "other", d.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(dismiss::list(&app.db, "default").await.unwrap().len(), 1);
    for i in 1..MAX_DISMISSALS {
        let key = DiscrepancyKey::Ghost {
            service: ServiceId::new(Some("n"), &format!("s{i}")),
        };
        assert!(matches!(
            dismiss::add(&app.db, "default", &key, "m0x6A", now)
                .await
                .unwrap(),
            Added::New(_)
        ));
    }
    let one_more = DiscrepancyKey::QuietSeam {
        seam: "web-server".into(),
    };
    assert!(matches!(
        dismiss::add(&app.db, "default", &one_more, "m0x6A", now)
            .await
            .unwrap(),
        Added::Full
    ));
    // Another workspace has its own room.
    assert!(matches!(
        dismiss::add(&app.db, "other", &one_more, "m0x6A", now)
            .await
            .unwrap(),
        Added::New(_)
    ));
    println!("checked {MAX_DISMISSALS} dismissals against the limit");
}
