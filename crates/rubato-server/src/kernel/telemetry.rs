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
    pub events_dropped: Counter<u64>,
    pub rate_limit_remaining: Gauge<u64>,
    pub rate_limit_limit: Gauge<u64>,
    pub plan_parse_failures: Counter<u64>,
    pub plan_unknown_id_count: Gauge<u64>,
    pub seam_open_plans: Gauge<u64>,
    pub map_loaded: Gauge<u64>,
    pub map_load_failures: Counter<u64>,
    pub ws_clients: Gauge<u64>,
    pub companion_heartbeat: Gauge<u64>,
    pub poll_ticks: Counter<u64>,
}

impl Metrics {
    pub fn new() -> Self {
        let m: Meter = global::meter("rubato-server");
        Self {
            sessions_by_state: m.u64_gauge(otel::metric::SESSIONS_BY_STATE).build(),
            issues_by_state: m.u64_gauge(otel::metric::ISSUES_BY_STATE).build(),
            events_dropped: m.u64_counter(otel::metric::EVENTS_DROPPED).build(),
            rate_limit_remaining: m
                .u64_gauge(otel::metric::GITHUB_RATE_LIMIT_REMAINING)
                .build(),
            rate_limit_limit: m.u64_gauge(otel::metric::GITHUB_RATE_LIMIT_LIMIT).build(),
            plan_parse_failures: m.u64_counter(otel::metric::PLAN_PARSE_FAILURES).build(),
            plan_unknown_id_count: m.u64_gauge(otel::metric::PLAN_UNKNOWN_ID_COUNT).build(),
            seam_open_plans: m.u64_gauge(otel::metric::SEAM_OPEN_PLANS).build(),
            map_loaded: m.u64_gauge(otel::metric::MAP_LOADED).build(),
            map_load_failures: m.u64_counter(otel::metric::MAP_LOAD_FAILURES).build(),
            ws_clients: m.u64_gauge(otel::metric::WS_CLIENTS).build(),
            companion_heartbeat: m.u64_gauge(otel::metric::COMPANION_HEARTBEAT).build(),
            poll_ticks: m.u64_counter(otel::metric::POLL_TICKS).build(),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Test-only: collect the events emitted on this thread, with their fields as strings.
#[cfg(test)]
pub mod capture {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};

    #[derive(Debug, Clone)]
    pub struct Captured {
        pub level: tracing::Level,
        pub message: String,
        pub fields: BTreeMap<String, String>,
    }

    pub type Events = Arc<Mutex<Vec<Captured>>>;

    struct Layer(Events);

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
            let mut c = Captured {
                level: *event.metadata().level(),
                message: String::new(),
                fields: BTreeMap::new(),
            };
            event.record(&mut Visitor(&mut c));
            self.0.lock().unwrap().push(c);
        }
    }

    /// Capture until the guard drops. Use from a plain test or a current-thread runtime.
    pub fn start() -> (Events, tracing::subscriber::DefaultGuard) {
        let events: Events = Arc::default();
        let sub = tracing_subscriber::registry().with(Layer(events.clone()));
        (events, tracing::subscriber::set_default(sub))
    }
}
