//! In-memory activity log: a bounded ring buffer fed by a `tracing` capture
//! layer, rendered by the Home activity-log console.
//!
//! Everything the program does already flows through `tracing` — MCP tool
//! calls, engine warnings, egui id-clash diagnostics, and (via a small mirror,
//! see `dialogs::set_status`) the user-facing status line. Rather than
//! hand-wiring `push()` calls at every event site, we register a second
//! `tracing_subscriber::Layer` that formats each event into a [`LogEntry`] and
//! stores it here. The GUI snapshots the buffer once per frame and renders it
//! lock-free.
//!
//! Writers are the tokio/MCP and GUI threads — never the audio thread — so a
//! plain [`std::sync::Mutex`] is fine; no real-time-safety constraint applies.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// Maximum number of entries retained in the ring buffer. Roughly a session's
/// worth of activity; the oldest entry is dropped when the buffer is full.
pub(crate) const CAP: usize = 1000;

/// Severity level of a captured log entry. A domain enum mirroring
/// [`tracing::Level`] — ordered most-severe (`Error`) to least (`Trace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub enum LogLevel {
    /// A hard failure.
    Error,
    /// A recoverable problem or noteworthy condition.
    Warn,
    /// Normal, user-relevant activity (the default view floor).
    Info,
    /// Fine-grained detail hidden behind the panel's Debug toggle.
    Debug,
    /// The most verbose per-note / per-parameter chatter.
    Trace,
}

impl LogLevel {
    /// Short uppercase tag used in the rendered console line.
    #[must_use]
    pub(crate) fn tag(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

impl From<&tracing::Level> for LogLevel {
    fn from(level: &tracing::Level) -> Self {
        match *level {
            tracing::Level::ERROR => Self::Error,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::INFO => Self::Info,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::TRACE => Self::Trace,
        }
    }
}

/// One captured `tracing` event.
#[derive(Debug, Clone)]
pub(crate) struct LogEntry {
    /// Monotonic id assigned by [`ActivityLog::push`], unique per entry and
    /// stable across snapshots and buffer eviction. A renderer keys transient
    /// per-row UI state (e.g. which row is expanded) on this rather than on a
    /// snapshot index, which shifts as the ring buffer fills. Left `0` at
    /// construction; `push` overwrites it.
    pub seq: u64,
    /// Wall-clock time the event was captured.
    pub time: std::time::SystemTime,
    /// Severity.
    pub level: LogLevel,
    /// The event's `tracing` target, e.g. `"synth_mcp::server"`.
    pub target: String,
    /// The formatted `message` field.
    pub message: String,
    /// Remaining structured fields (tool, elapsed_ms, …) as `(key, value)`.
    pub fields: Vec<(String, String)>,
}

/// Interior of an [`ActivityLog`]: the buffer plus a monotonically increasing
/// generation counter, bumped on every mutation so a renderer can cheaply tell
/// whether re-snapshotting (which deep-clones every entry) is even necessary.
struct Inner {
    buf: Mutex<VecDeque<LogEntry>>,
    generation: AtomicU64,
    /// Source of monotonic per-entry [`LogEntry::seq`] ids.
    next_seq: AtomicU64,
}

/// Shared, bounded ring buffer of [`LogEntry`]. Cheap to `clone` — every clone
/// points at the same underlying buffer.
#[derive(Clone)]
pub struct ActivityLog(Arc<Inner>);

impl Default for ActivityLog {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityLog {
    /// Create an empty log with capacity [`CAP`].
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            buf: Mutex::new(VecDeque::with_capacity(CAP)),
            generation: AtomicU64::new(0),
            next_seq: AtomicU64::new(0),
        }))
    }

    /// Append an entry, dropping the oldest when at [`CAP`]. Stamps the entry
    /// with a fresh monotonic [`LogEntry::seq`].
    pub(crate) fn push(&self, mut entry: LogEntry) {
        entry.seq = self.0.next_seq.fetch_add(1, Ordering::Relaxed);
        let mut buf = self.0.buf.lock().unwrap_or_else(|e| e.into_inner());
        if buf.len() >= CAP {
            buf.pop_front();
        }
        buf.push_back(entry);
        self.0.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A counter that increases on every `push`/`clear`. A renderer caches its
    /// last-seen value and only calls [`snapshot_into`](Self::snapshot_into)
    /// when it changes, avoiding a per-frame deep-clone of an unchanged buffer.
    #[must_use]
    pub(crate) fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::Relaxed)
    }

    /// Copy the whole buffer (oldest first) into `out`, which is cleared first.
    /// The GUI reuses a scratch `Vec` across frames; gate the call on
    /// [`generation`](Self::generation) to skip the clone when nothing changed.
    pub(crate) fn snapshot_into(&self, out: &mut Vec<LogEntry>) {
        out.clear();
        let buf = self.0.buf.lock().unwrap_or_else(|e| e.into_inner());
        out.extend(buf.iter().cloned());
    }

    /// Remove all entries.
    pub(crate) fn clear(&self) {
        self.0.buf.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.0.generation.fetch_add(1, Ordering::Relaxed);
    }
}

/// Collects an event's fields, splitting out the special `message` field from
/// the rest.
#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl FieldVisitor {
    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else {
            self.fields.push((field.name().to_string(), value));
        }
    }
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record(field, value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record(field, value.to_string());
    }
}

/// A `tracing_subscriber::Layer` that captures each event into an
/// [`ActivityLog`]. Attach it to a `Registry` under its own permissive filter
/// (see `main::init_tracing`); the console decides what to *show*.
pub struct CaptureLayer {
    log: ActivityLog,
}

impl CaptureLayer {
    /// Create a capture layer feeding `log`.
    #[must_use]
    pub fn new(log: ActivityLog) -> Self {
        Self { log }
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.log.push(LogEntry {
            seq: 0, // assigned by `push`
            time: std::time::SystemTime::now(),
            level: LogLevel::from(meta.level()),
            target: meta.target().to_string(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: LogLevel, msg: &str) -> LogEntry {
        LogEntry {
            seq: 0,
            time: std::time::SystemTime::now(),
            level,
            target: "test".to_string(),
            message: msg.to_string(),
            fields: Vec::new(),
        }
    }

    #[test]
    fn push_evicts_oldest_at_cap() {
        let log = ActivityLog::new();
        for i in 0..CAP + 10 {
            log.push(entry(LogLevel::Info, &format!("m{i}")));
        }
        let mut snap = Vec::new();
        log.snapshot_into(&mut snap);
        assert_eq!(snap.len(), CAP);
        // Oldest 10 were evicted; the front is now m10.
        assert_eq!(snap.first().map(|e| e.message.as_str()), Some("m10"));
        assert_eq!(
            snap.last().map(|e| e.message.as_str()),
            Some(format!("m{}", CAP + 9)).as_deref()
        );
    }

    #[test]
    fn snapshot_into_reuses_and_clears_buffer() {
        let log = ActivityLog::new();
        log.push(entry(LogLevel::Warn, "a"));
        let mut snap = vec![entry(LogLevel::Info, "stale")];
        log.snapshot_into(&mut snap);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].message, "a");
    }

    #[test]
    fn clear_empties() {
        let log = ActivityLog::new();
        log.push(entry(LogLevel::Error, "x"));
        let mut snap = Vec::new();
        log.snapshot_into(&mut snap);
        assert_eq!(snap.len(), 1);
        log.clear();
        log.snapshot_into(&mut snap);
        assert!(snap.is_empty());
    }

    #[test]
    fn generation_bumps_on_mutation() {
        let log = ActivityLog::new();
        let g0 = log.generation();
        log.push(entry(LogLevel::Info, "a"));
        let g1 = log.generation();
        assert!(g1 > g0);
        log.clear();
        assert!(log.generation() > g1);
    }

    #[test]
    fn level_ordering_error_is_most_severe() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }
}
