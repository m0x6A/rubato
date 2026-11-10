//! Every tracing event the in-process runtime emits, as `message key=value …` lines, so a step
//! can assert on a log line. Scenarios run one at a time; the World clears the list.

use std::sync::Mutex;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt};

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

struct Line(String);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.insert_str(0, &format!("{value:?}"));
        } else {
            self.0.push_str(&format!(" {}={value:?}", field.name()));
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut l = Line(String::new());
        event.record(&mut l);
        LINES.lock().unwrap().push(l.0);
    }
}

pub fn install() {
    let sub = tracing_subscriber::registry().with(Capture);
    tracing::subscriber::set_global_default(sub).expect("one subscriber");
}

pub fn clear() {
    LINES.lock().unwrap().clear();
}

pub fn contains(text: &str) -> bool {
    LINES.lock().unwrap().iter().any(|l| l.contains(text))
}
