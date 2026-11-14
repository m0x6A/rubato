//! tracing everywhere; OTLP export when an endpoint is configured. Names only from
//! `rubato_protocol::otel`.

use anyhow::Result;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge, Meter};
use rubato_protocol::otel;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub struct Guard {
    tracer: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    meter: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(t) = self.tracer.take() {
            let _ = t.shutdown();
        }
        if let Some(m) = self.meter.take() {
            let _ = m.shutdown();
        }
    }
}

pub fn init(service_name: &'static str, endpoint: Option<&str>) -> Result<Guard> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(false);
    let Some(endpoint) = endpoint else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .try_init()
            .ok();
        return Ok(Guard {
            tracer: None,
            meter: None,
        });
    };
    use opentelemetry_otlp::WithExportConfig;
    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(service_name)
        .build();
    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        .build()?;
    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();
    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/metrics", endpoint.trim_end_matches('/')))
        .build()?;
    let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .with_resource(resource)
        .build();
    global::set_meter_provider(meter_provider.clone());
    use opentelemetry::trace::TracerProvider as _;
    let tracer = tracer_provider.tracer(service_name);
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()
        .ok();
    Ok(Guard {
        tracer: Some(tracer_provider),
        meter: Some(meter_provider),
    })
}

/// The server's instruments. No-ops without a meter provider.
pub struct Metrics {
    pub sessions_by_state: Gauge<u64>,
    pub issues_by_state: Gauge<u64>,
    /// Blocked issues by the station they left from (ADR 0043).
    pub issues_blocked_from: Gauge<u64>,
    pub events_dropped: Counter<u64>,
    pub rate_limit_remaining: Gauge<u64>,
    pub rate_limit_limit: Gauge<u64>,
    pub plan_parse_failures: Counter<u64>,
    pub plan_unknown_id_count: Gauge<u64>,
    pub plans_approval_stale: Gauge<u64>,
    pub seam_open_plans: Gauge<u64>,
    pub map_loaded: Gauge<u64>,
    /// Unix seconds of the last successful load. No unit: one would rename the Prometheus
    /// series to `rubato_map_loaded_at_seconds` and empty the "Map age" panel.
    pub map_loaded_at: Gauge<u64>,
    pub map_load_failures: Counter<u64>,
    pub ws_clients: Gauge<u64>,
    pub companion_heartbeat: Gauge<u64>,
    pub poll_ticks: Counter<u64>,
    /// Unix seconds of each repo's last good listing; no unit, like `map_loaded_at`.
    pub backlog_listed_at: Gauge<u64>,
    pub backlog_issues: Gauge<u64>,
    pub web_rejected: Counter<u64>,
    pub github_repos_uncovered: Gauge<u64>,
    pub layout_writes: Counter<u64>,
    pub queue_depth: Gauge<u64>,
    pub map_draft_ops: Gauge<u64>,
    pub map_proposals: Counter<u64>,
    /// Unix seconds; no unit, like `map_loaded_at`.
    pub map_proposal_state_since: Gauge<u64>,
    pub backlog_map_lines: Counter<u64>,
    pub issues_created: Counter<u64>,
    pub issues_opening: Gauge<u64>,
    pub plans_not_fitting: Gauge<u64>,
    pub plans_approvals_opening: Gauge<u64>,
    /// The configured companion stale threshold N in seconds (#105). No unit, so the series
    /// stays `rubato_companion_silent_secs`, the name `CompanionSilent` reads.
    pub companion_silent_secs: Gauge<u64>,
}

impl Metrics {
    pub fn new() -> Self {
        Self::from_meter(&global::meter("rubato-server"))
    }

    /// The instruments on a given meter; tests pass one backed by an in-memory reader.
    pub fn from_meter(m: &Meter) -> Self {
        Self {
            sessions_by_state: m.u64_gauge(otel::metric::SESSIONS_BY_STATE).build(),
            issues_by_state: m.u64_gauge(otel::metric::ISSUES_BY_STATE).build(),
            issues_blocked_from: m.u64_gauge(otel::metric::ISSUES_BLOCKED_FROM).build(),
            events_dropped: m.u64_counter(otel::metric::EVENTS_DROPPED).build(),
            rate_limit_remaining: m
                .u64_gauge(otel::metric::GITHUB_RATE_LIMIT_REMAINING)
                .build(),
            rate_limit_limit: m.u64_gauge(otel::metric::GITHUB_RATE_LIMIT_LIMIT).build(),
            plan_parse_failures: m.u64_counter(otel::metric::PLAN_PARSE_FAILURES).build(),
            plan_unknown_id_count: m.u64_gauge(otel::metric::PLAN_UNKNOWN_ID_COUNT).build(),
            plans_approval_stale: m.u64_gauge(otel::metric::PLANS_APPROVAL_STALE).build(),
            seam_open_plans: m.u64_gauge(otel::metric::SEAM_OPEN_PLANS).build(),
            map_loaded: m.u64_gauge(otel::metric::MAP_LOADED).build(),
            map_loaded_at: m.u64_gauge(otel::metric::MAP_LOADED_AT).build(),
            map_load_failures: m.u64_counter(otel::metric::MAP_LOAD_FAILURES).build(),
            ws_clients: m.u64_gauge(otel::metric::WS_CLIENTS).build(),
            companion_heartbeat: m.u64_gauge(otel::metric::COMPANION_HEARTBEAT).build(),
            poll_ticks: m.u64_counter(otel::metric::POLL_TICKS).build(),
            backlog_listed_at: m.u64_gauge(otel::metric::BACKLOG_LISTED_AT).build(),
            backlog_issues: m.u64_gauge(otel::metric::BACKLOG_ISSUES).build(),
            web_rejected: m.u64_counter(otel::metric::WEB_REJECTED).build(),
            github_repos_uncovered: m.u64_gauge(otel::metric::GITHUB_REPOS_UNCOVERED).build(),
            layout_writes: m.u64_counter(otel::metric::LAYOUT_WRITES).build(),
            queue_depth: m.u64_gauge(otel::metric::QUEUE_DEPTH).build(),
            map_draft_ops: m.u64_gauge(otel::metric::MAP_DRAFT_OPS).build(),
            map_proposals: m.u64_counter(otel::metric::MAP_PROPOSALS).build(),
            map_proposal_state_since: m.u64_gauge(otel::metric::MAP_PROPOSAL_STATE_SINCE).build(),
            backlog_map_lines: m.u64_counter(otel::metric::BACKLOG_MAP_LINES).build(),
            issues_created: m.u64_counter(otel::metric::ISSUES_CREATED).build(),
            issues_opening: m.u64_gauge(otel::metric::ISSUES_OPENING).build(),
            plans_not_fitting: m.u64_gauge(otel::metric::PLANS_NOT_FITTING).build(),
            plans_approvals_opening: m.u64_gauge(otel::metric::PLANS_APPROVALS_OPENING).build(),
            companion_silent_secs: m.u64_gauge(otel::metric::COMPANION_SILENT_SECS).build(),
        }
    }

    /// What every poll tick records: one `rubato.poll.ticks`, so `ServerPollSilent` sees it
    /// stop, and the configured N, so `CompanionSilent` has it on every export (#105).
    pub fn record_poll_tick(&self, limits: &rubato_protocol::Limits) {
        self.poll_ticks.add(1, &[]);
        self.companion_silent_secs
            .record(limits.companion_silent_secs, &[]);
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Test-only: instruments backed by an in-memory exporter, read back as data points.
#[cfg(test)]
pub mod metric_reader {
    use super::Metrics;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use std::collections::BTreeMap;

    /// One exported data point. Every instrument here is `u64`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Point {
        pub name: String,
        pub unit: String,
        pub attrs: BTreeMap<String, String>,
        pub value: u64,
    }

    pub struct Reader {
        provider: SdkMeterProvider,
        exporter: InMemoryMetricExporter,
    }

    /// A reader and the instruments it reads. Nothing is exported until `collect`.
    pub fn start() -> (Reader, Metrics) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let metrics = Metrics::from_meter(&provider.meter("rubato-test"));
        (Reader { provider, exporter }, metrics)
    }

    fn attrs<'a>(
        kvs: impl Iterator<Item = &'a opentelemetry::KeyValue>,
    ) -> BTreeMap<String, String> {
        kvs.map(|kv| (kv.key.to_string(), kv.value.to_string()))
            .collect()
    }

    impl Reader {
        /// Flush and return what one export carries: with cumulative temporality that is every
        /// series recorded so far, so a gauge shows its last value on each collection.
        pub fn collect(&self) -> Vec<Point> {
            self.exporter.reset();
            self.provider.force_flush().expect("flush");
            let mut out = Vec::new();
            for rm in self.exporter.get_finished_metrics().expect("metrics") {
                for metric in rm.scope_metrics().flat_map(|s| s.metrics()) {
                    let AggregatedMetrics::U64(data) = metric.data() else {
                        continue;
                    };
                    let values: Vec<(BTreeMap<String, String>, u64)> = match data {
                        MetricData::Gauge(g) => g
                            .data_points()
                            .map(|p| (attrs(p.attributes()), p.value()))
                            .collect(),
                        MetricData::Sum(s) => s
                            .data_points()
                            .map(|p| (attrs(p.attributes()), p.value()))
                            .collect(),
                        _ => Vec::new(),
                    };
                    out.extend(values.into_iter().map(|(attrs, value)| Point {
                        name: metric.name().to_owned(),
                        unit: metric.unit().to_owned(),
                        attrs,
                        value,
                    }));
                }
            }
            out
        }

        /// The data points of one metric.
        pub fn points(&self, name: &str) -> Vec<Point> {
            self.collect()
                .into_iter()
                .filter(|p| p.name == name)
                .collect()
        }
    }
}

/// Test-only: collect the events emitted on this thread, with their fields as strings.
#[cfg(test)]
pub mod capture {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, Once};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};

    #[derive(Debug, Clone)]
    pub struct Captured {
        pub level: tracing::Level,
        pub message: String,
        pub fields: BTreeMap<String, String>,
    }

    pub type Events = Arc<Mutex<Vec<Captured>>>;

    thread_local! {
        /// Where this thread's events go while a guard lives.
        static SINK: RefCell<Option<Events>> = const { RefCell::new(None) };
    }

    struct Layer;

    /// Stops the capture on this thread when dropped, and puts back the one it replaced.
    pub struct Guard(Option<Events>);

    impl Drop for Guard {
        fn drop(&mut self) {
            let before = self.0.take();
            let _ = SINK.try_with(|s| *s.borrow_mut() = before);
        }
    }

    struct Visitor<'a>(&'a mut Captured);

    impl Visit for Visitor<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.put(field, format!("{value:?}"));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.put(field, value.to_owned());
        }
    }

    impl Visitor<'_> {
        fn put(&mut self, field: &Field, v: String) {
            if field.name() == "message" {
                self.0.message = v;
            } else {
                self.0.fields.insert(field.name().to_owned(), v);
            }
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Layer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let Some(sink) = SINK.try_with(|s| s.borrow().clone()).ok().flatten() else {
                return;
            };
            let mut c = Captured {
                level: *event.metadata().level(),
                message: String::new(),
                fields: BTreeMap::new(),
            };
            event.record(&mut Visitor(&mut c));
            sink.lock().unwrap().push(c);
        }
    }

    /// Capture this thread's events until the guard drops. Use from a plain test or a
    /// current-thread runtime.
    ///
    /// One subscriber serves the whole test process and hands each event to its own thread's
    /// sink. A subscriber per test (`set_default`) lost events now and then: tracing keeps one
    /// answer per callsite for the whole process, and with subscribers coming and going on
    /// other test threads a line could be cached as "nobody listens" (seen in CI on
    /// `refusals_are_counted_by_route_and_reason`).
    pub fn start() -> (Events, Guard) {
        static GLOBAL: Once = Once::new();
        GLOBAL.call_once(|| {
            tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Layer))
                .expect("the capture subscriber is the only global subscriber in unit tests");
            // A callsite first reached while the subscriber was being installed may have
            // cached "nobody listens"; ask again now that it is in place.
            tracing::callsite::rebuild_interest_cache();
        });
        let events: Events = Arc::default();
        let before = SINK.with(|s| s.borrow_mut().replace(events.clone()));
        (events, Guard(before))
    }

    #[cfg(test)]
    mod tests {
        use super::start;

        fn messages(events: &super::Events) -> Vec<String> {
            let events = events.lock().unwrap();
            events.iter().map(|e| e.message.clone()).collect()
        }

        /// A capture holds its own thread's events only, and stops when its guard drops.
        #[test]
        fn a_capture_holds_its_own_threads_events_until_the_guard_drops() {
            let (events, guard) = start();
            tracing::info!(n = 1, "mine");
            // Another thread with its own capture, and one with none: neither reaches this one.
            std::thread::spawn(|| {
                let (theirs, _g) = start();
                tracing::info!("theirs");
                assert_eq!(messages(&theirs), ["theirs"]);
            })
            .join()
            .unwrap();
            std::thread::spawn(|| tracing::info!("nobody's"))
                .join()
                .unwrap();
            assert_eq!(messages(&events), ["mine"]);
            assert_eq!(
                events.lock().unwrap()[0]
                    .fields
                    .get("n")
                    .map(String::as_str),
                Some("1")
            );
            drop(guard);
            tracing::info!("after the guard");
            assert_eq!(messages(&events), ["mine"]);
        }

        /// A line first reached on a thread with no capture is still captured later on a
        /// thread with one.
        #[test]
        fn a_line_first_reached_without_a_capture_is_captured_later() {
            fn line() {
                tracing::info!("reached twice");
            }
            std::thread::spawn(line).join().unwrap();
            let (events, _guard) = start();
            line();
            assert_eq!(messages(&events), ["reached twice"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::metric_reader;
    use rubato_protocol::{Limits, otel};

    /// #105: every poll tick records the configured N, so `CompanionSilent` compares against
    /// the server's threshold on every collection, whatever the exporter's temporality.
    #[test]
    fn a_poll_tick_records_the_companion_threshold() {
        let (reader, metrics) = metric_reader::start();
        let limits = Limits {
            companion_silent_secs: 300,
            ..Limits::default()
        };
        metrics.record_poll_tick(&limits);
        let mut seen = Vec::new();
        for _ in 0..2 {
            let points = reader.points(otel::metric::COMPANION_SILENT_SECS);
            assert_eq!(points.len(), 1, "{points:?}");
            assert_eq!(points[0].value, 300);
            assert!(points[0].attrs.is_empty(), "{points:?}");
            seen.push(points[0].value);
        }
        assert_eq!(reader.points(otel::metric::POLL_TICKS)[0].value, 1);
        println!("checked the threshold on {} collections", seen.len());
    }
}
