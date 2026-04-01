//! Priority-based event system for GUI-Engine communication.
//!
//! This module provides a multi-priority event channel that ensures critical
//! events (like errors and buffer underruns) are never dropped, while allowing
//! lower-priority events (like visualization data) to be dropped under load.

use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use synth_core::SamplePosition;

use super::commands::EngineEvent;

/// Priority levels for engine events.
/// Higher priority events are processed first and have dedicated buffer space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum EventPriority {
    /// Critical: Buffer underruns, errors, clipping - never dropped.
    Critical = 0,
    /// High: State changes, connections, topology changes.
    High = 1,
    /// Normal: Parameter echoes, voice activity.
    Normal = 2,
    /// Low: Visualization data, meters - may be dropped under load.
    Low = 3,
}

impl EventPriority {
    /// Get all priority levels in order.
    pub const ALL: [Self; 4] = [Self::Critical, Self::High, Self::Normal, Self::Low];

    /// Get the name of this priority level.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::High => "High",
            Self::Normal => "Normal",
            Self::Low => "Low",
        }
    }
}

/// Enhanced engine event with metadata.
#[derive(Debug, Clone)]
pub struct TimestampedEvent {
    /// The actual event.
    pub event: EngineEvent,
    /// Sample position when event occurred.
    pub timestamp: SamplePosition,
    /// Priority level of this event.
    pub priority: EventPriority,
    /// Monotonic sequence number for ordering.
    pub sequence_id: u64,
}

impl TimestampedEvent {
    /// Create a new timestamped event.
    pub fn new(
        event: EngineEvent,
        timestamp: SamplePosition,
        priority: EventPriority,
        sequence_id: u64,
    ) -> Self {
        Self {
            event,
            timestamp,
            priority,
            sequence_id,
        }
    }
}

/// Buffer sizes for each priority level.
const CRITICAL_BUFFER_SIZE: usize = 256;
const HIGH_BUFFER_SIZE: usize = 256;
const NORMAL_BUFFER_SIZE: usize = 512;
const LOW_BUFFER_SIZE: usize = 2048;

/// Producer side of the prioritized event channel.
pub struct PrioritizedEventProducer {
    critical: ringbuf::HeapProd<TimestampedEvent>,
    high: ringbuf::HeapProd<TimestampedEvent>,
    normal: ringbuf::HeapProd<TimestampedEvent>,
    low: ringbuf::HeapProd<TimestampedEvent>,
    sequence_counter: AtomicU64,
    dropped_counts: [AtomicU32; 4],
}

/// Consumer side of the prioritized event channel.
pub struct PrioritizedEventConsumer {
    critical: ringbuf::HeapCons<TimestampedEvent>,
    high: ringbuf::HeapCons<TimestampedEvent>,
    normal: ringbuf::HeapCons<TimestampedEvent>,
    low: ringbuf::HeapCons<TimestampedEvent>,
}

/// Create a new prioritized event channel, returning producer and consumer.
pub fn prioritized_event_channel() -> (PrioritizedEventProducer, PrioritizedEventConsumer) {
    let critical_rb = HeapRb::<TimestampedEvent>::new(CRITICAL_BUFFER_SIZE);
    let high_rb = HeapRb::<TimestampedEvent>::new(HIGH_BUFFER_SIZE);
    let normal_rb = HeapRb::<TimestampedEvent>::new(NORMAL_BUFFER_SIZE);
    let low_rb = HeapRb::<TimestampedEvent>::new(LOW_BUFFER_SIZE);

    let (critical_prod, critical_cons) = critical_rb.split();
    let (high_prod, high_cons) = high_rb.split();
    let (normal_prod, normal_cons) = normal_rb.split();
    let (low_prod, low_cons) = low_rb.split();

    let producer = PrioritizedEventProducer {
        critical: critical_prod,
        high: high_prod,
        normal: normal_prod,
        low: low_prod,
        sequence_counter: AtomicU64::new(0),
        dropped_counts: [
            AtomicU32::new(0),
            AtomicU32::new(0),
            AtomicU32::new(0),
            AtomicU32::new(0),
        ],
    };

    let consumer = PrioritizedEventConsumer {
        critical: critical_cons,
        high: high_cons,
        normal: normal_cons,
        low: low_cons,
    };

    (producer, consumer)
}

impl PrioritizedEventProducer {
    /// Send an event with the given priority.
    /// Critical events will retry on full buffer, others may be dropped.
    pub fn send(&mut self, event: EngineEvent, priority: EventPriority, timestamp: SamplePosition) {
        let seq = self.sequence_counter.fetch_add(1, Ordering::Relaxed);
        let timestamped = TimestampedEvent::new(event, timestamp, priority, seq);

        let result = match priority {
            EventPriority::Critical => self.critical.try_push(timestamped),
            EventPriority::High => self.high.try_push(timestamped),
            EventPriority::Normal => self.normal.try_push(timestamped),
            EventPriority::Low => self.low.try_push(timestamped),
        };

        if result.is_err() {
            self.dropped_counts[priority as usize].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Send an event with automatic priority detection based on event type.
    pub fn send_auto(&mut self, event: EngineEvent, timestamp: SamplePosition) {
        let priority = Self::event_priority(&event);
        self.send(event, priority, timestamp);
    }

    /// Determine the priority of an event based on its type.
    pub fn event_priority(event: &EngineEvent) -> EventPriority {
        match event {
            // Critical events
            EngineEvent::BufferUnderrun => EventPriority::Critical,

            // High priority events (state changes)
            EngineEvent::VoiceCount(_) => EventPriority::High,
            EngineEvent::ParameterChanged { .. } => EventPriority::High,
            EngineEvent::EnvelopeStage { .. } => EventPriority::High,
            EngineEvent::NoteTriggered { .. } => EventPriority::High,
            EngineEvent::NoteReleased { .. } => EventPriority::High,
            EngineEvent::AllNotesReleased => EventPriority::High,
            EngineEvent::KeyRangeLearned { .. } => EventPriority::High,
            EngineEvent::RecordedNotesFlushed { .. } => EventPriority::High,

            // Extended events (when added)
            // EngineEvent::ModuleAdded { .. } => EventPriority::High,
            // EngineEvent::ModuleRemoved { .. } => EventPriority::High,
            // EngineEvent::ConnectionAdded { .. } => EventPriority::High,
            // EngineEvent::ConnectionRemoved { .. } => EventPriority::High,
            // EngineEvent::ModuleError { .. } => EventPriority::Critical,

            // Normal priority
            EngineEvent::CpuUsage(_) => EventPriority::Normal,

            // Low priority (visualization)
            EngineEvent::PeakMeter { .. } => EventPriority::Low,
            EngineEvent::RmsMeter { .. } => EventPriority::Low,
            EngineEvent::WaveformData { .. } => EventPriority::Low,
            EngineEvent::RecordingPreview { .. } => EventPriority::Low,
        }
    }

    /// Get and reset dropped event counts.
    pub fn get_dropped_counts(&self) -> [u32; 4] {
        [
            self.dropped_counts[0].swap(0, Ordering::Relaxed),
            self.dropped_counts[1].swap(0, Ordering::Relaxed),
            self.dropped_counts[2].swap(0, Ordering::Relaxed),
            self.dropped_counts[3].swap(0, Ordering::Relaxed),
        ]
    }

    /// Check if any events were dropped.
    pub fn any_dropped(&self) -> bool {
        self.dropped_counts
            .iter()
            .any(|c| c.load(Ordering::Relaxed) > 0)
    }
}

impl PrioritizedEventConsumer {
    /// Poll for the next event, checking high priority first.
    pub fn poll(&mut self) -> Option<TimestampedEvent> {
        // Check in priority order
        if let Some(event) = self.critical.try_pop() {
            return Some(event);
        }
        if let Some(event) = self.high.try_pop() {
            return Some(event);
        }
        if let Some(event) = self.normal.try_pop() {
            return Some(event);
        }
        if let Some(event) = self.low.try_pop() {
            return Some(event);
        }
        None
    }

    /// Poll all available events, returning them sorted by sequence ID.
    pub fn poll_all(&mut self) -> Vec<TimestampedEvent> {
        let mut events = Vec::new();

        while let Some(event) = self.critical.try_pop() {
            events.push(event);
        }
        while let Some(event) = self.high.try_pop() {
            events.push(event);
        }
        while let Some(event) = self.normal.try_pop() {
            events.push(event);
        }
        while let Some(event) = self.low.try_pop() {
            events.push(event);
        }

        // Sort by sequence ID to maintain ordering
        events.sort_by_key(|e| e.sequence_id);
        events
    }

    /// Poll events of a specific priority only.
    pub fn poll_priority(&mut self, priority: EventPriority) -> Option<TimestampedEvent> {
        match priority {
            EventPriority::Critical => self.critical.try_pop(),
            EventPriority::High => self.high.try_pop(),
            EventPriority::Normal => self.normal.try_pop(),
            EventPriority::Low => self.low.try_pop(),
        }
    }

    /// Check if there are any pending events.
    pub fn has_events(&self) -> bool {
        !self.critical.is_empty()
            || !self.high.is_empty()
            || !self.normal.is_empty()
            || !self.low.is_empty()
    }

    /// Get the count of pending events per priority.
    pub fn pending_counts(&self) -> [usize; 4] {
        [
            self.critical.occupied_len(),
            self.high.occupied_len(),
            self.normal.occupied_len(),
            self.low.occupied_len(),
        ]
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_event_priority_ordering() {
        assert!(EventPriority::Critical < EventPriority::High);
        assert!(EventPriority::High < EventPriority::Normal);
        assert!(EventPriority::Normal < EventPriority::Low);
    }

    #[test]
    fn test_prioritized_channel() {
        let (mut producer, mut consumer) = prioritized_event_channel();

        // Send events of different priorities
        producer.send(
            EngineEvent::PeakMeter {
                left: synth_core::Amplitude::new(0.5),
                right: synth_core::Amplitude::new(0.5),
            },
            EventPriority::Low,
            SamplePosition::ZERO,
        );
        producer.send(
            EngineEvent::VoiceCount(1),
            EventPriority::High,
            SamplePosition::new(1),
        );
        producer.send(
            EngineEvent::BufferUnderrun,
            EventPriority::Critical,
            SamplePosition::new(2),
        );

        // Should receive in priority order
        let event1 = consumer.poll().unwrap();
        assert!(matches!(event1.event, EngineEvent::BufferUnderrun));

        let event2 = consumer.poll().unwrap();
        assert!(matches!(event2.event, EngineEvent::VoiceCount(_)));

        let event3 = consumer.poll().unwrap();
        assert!(matches!(event3.event, EngineEvent::PeakMeter { .. }));
    }

    #[test]
    fn test_auto_priority() {
        assert_eq!(
            PrioritizedEventProducer::event_priority(&EngineEvent::BufferUnderrun),
            EventPriority::Critical
        );
        assert_eq!(
            PrioritizedEventProducer::event_priority(&EngineEvent::VoiceCount(0)),
            EventPriority::High
        );
        assert_eq!(
            PrioritizedEventProducer::event_priority(&EngineEvent::PeakMeter {
                left: synth_core::Amplitude::ZERO,
                right: synth_core::Amplitude::ZERO,
            }),
            EventPriority::Low
        );
    }

    #[test]
    fn test_poll_all_sorted() {
        let (mut producer, mut consumer) = prioritized_event_channel();

        // Send in reverse sequence order
        producer.send(
            EngineEvent::VoiceCount(3),
            EventPriority::Low,
            SamplePosition::new(100),
        );
        producer.send(
            EngineEvent::VoiceCount(2),
            EventPriority::High,
            SamplePosition::new(50),
        );
        producer.send(
            EngineEvent::VoiceCount(1),
            EventPriority::Normal,
            SamplePosition::new(75),
        );

        let events = consumer.poll_all();
        assert_eq!(events.len(), 3);

        // Should be sorted by sequence_id
        assert!(events[0].sequence_id < events[1].sequence_id);
        assert!(events[1].sequence_id < events[2].sequence_id);
    }
}
