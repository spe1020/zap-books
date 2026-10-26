//! A minimal `tracing` subscriber that prints one line per event to stderr.
//! Callers only ever log timings, model names, and field statuses.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

struct Line(String);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, " {value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            let _ = write!(self.0, " {value}");
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }
}

/// Prints events at `INFO` and above.
pub struct StderrLog {
    next_span: AtomicU64,
}

impl Subscriber for StderrLog {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::INFO
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed).max(1))
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut line = Line(format!("extractor {}", event.metadata().level()));
        event.record(&mut line);
        eprintln!("{}", line.0);
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// Installs the stderr logger for the process. Safe to call more than once.
pub fn init() {
    let _ = tracing::subscriber::set_global_default(StderrLog {
        next_span: AtomicU64::new(1),
    });
}
