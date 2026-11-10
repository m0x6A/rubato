//! tracing everywhere; OTLP export when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Names only from
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

/// Initialise logging and, when an OTLP endpoint is configured, traces and metrics export.
pub fn init(service_name: &'static str, endpoint: Option<&str>) -> Result<Guard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(false);
    let endpoint = endpoint
        .map(str::to_owned)
        .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok());
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

/// The companion's instruments. No-ops when no meter provider is installed.
pub struct Metrics {
    pub heartbeat: Gauge<u64>,
    pub hook_forwarded: Counter<u64>,
    pub sessions_discovered: Gauge<u64>,
    pub sessions_by_state: Gauge<u64>,
    pub events_dropped: Counter<u64>,
}

impl Metrics {
    pub fn new() -> Self {
        let m: Meter = global::meter("rubato-companion");
        Self {
            heartbeat: m
                .u64_gauge(otel::metric::COMPANION_HEARTBEAT)
                .with_description("unix seconds of the last heartbeat")
                .build(),
            hook_forwarded: m.u64_counter(otel::metric::HOOK_FORWARDED).build(),
            sessions_discovered: m.u64_gauge(otel::metric::SESSIONS_DISCOVERED).build(),
            sessions_by_state: m.u64_gauge(otel::metric::SESSIONS_BY_STATE).build(),
            events_dropped: m.u64_counter(otel::metric::EVENTS_DROPPED).build(),
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
