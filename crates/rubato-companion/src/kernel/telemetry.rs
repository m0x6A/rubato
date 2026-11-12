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
        Self::from_meter(&global::meter("rubato-companion"))
    }

    /// The instruments on a given meter; tests pass one backed by an in-memory reader.
    pub fn from_meter(m: &Meter) -> Self {
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
    use tracing::span::{Attributes, Id, Record};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::LookupSpan;

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
        /// Where this thread's spans go while a span guard lives.
        static SPAN_SINK: RefCell<Option<Events>> = const { RefCell::new(None) };
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

    impl<S> tracing_subscriber::Layer<S> for Layer
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
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

        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            let Some(sink) = SPAN_SINK.try_with(|s| s.borrow().clone()).ok().flatten() else {
                return;
            };
            let mut c = Captured {
                level: *attrs.metadata().level(),
                message: attrs.metadata().name().to_owned(),
                fields: BTreeMap::new(),
            };
            attrs.record(&mut Visitor(&mut c));
            let index = {
                let mut spans = sink.lock().unwrap();
                spans.push(c);
                spans.len() - 1
            };
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(SpanSlot(sink, index));
            }
        }

        fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
            let Some(span) = ctx.span(id) else { return };
            let ext = span.extensions();
            if let Some(SpanSlot(sink, index)) = ext.get::<SpanSlot>() {
                values.record(&mut Visitor(&mut sink.lock().unwrap()[*index]));
            }
        }
    }

    /// Where a captured span's later `record` calls land: its sink and its index there.
    struct SpanSlot(Events, usize);

    fn install() {
        static GLOBAL: Once = Once::new();
        GLOBAL.call_once(|| {
            tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Layer))
                .expect("the capture subscriber is the only global subscriber in unit tests");
            // A callsite first reached while the subscriber was being installed may have
            // cached "nobody listens"; ask again now that it is in place.
            tracing::callsite::rebuild_interest_cache();
        });
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
        install();
        let events: Events = Arc::default();
        let before = SINK.with(|s| s.borrow_mut().replace(events.clone()));
        (events, Guard(before))
    }

    /// Capture the spans opened on this thread until the guard drops: one `Captured` per span,
    /// its name in `message`, its fields as opened plus every later `record` (from any thread).
    /// Kept apart from `start` so tests that count events see no spans.
    pub fn spans() -> (Events, SpanGuard) {
        install();
        let spans: Events = Arc::default();
        let before = SPAN_SINK.with(|s| s.borrow_mut().replace(spans.clone()));
        (spans, SpanGuard(before))
    }

    /// Stops the span capture on this thread when dropped.
    pub struct SpanGuard(Option<Events>);

    impl Drop for SpanGuard {
        fn drop(&mut self) {
            let before = self.0.take();
            let _ = SPAN_SINK.try_with(|s| *s.borrow_mut() = before);
        }
    }
}
