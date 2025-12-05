//! Per-module CPU usage tracking.
//!
//! This module provides real-time CPU usage measurement for individual
//! modules in the audio processing graph.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::commands::ModuleId;

/// Per-module CPU usage statistics.
#[derive(Debug, Clone, Default)]
pub struct ModuleCpuStats {
    /// Current CPU usage as percentage (0.0 - 100.0+).
    pub cpu_percent: f32,
    /// Average processing time per block.
    pub avg_process_time: Duration,
    /// Peak processing time (worst case).
    pub peak_process_time: Duration,
    /// Number of samples processed.
    pub samples_processed: u64,
    /// Number of blocks processed.
    pub blocks_processed: u64,
}

impl ModuleCpuStats {
    /// Get CPU usage as a 0.0-1.0 fraction.
    pub fn cpu_fraction(&self) -> f32 {
        self.cpu_percent / 100.0
    }

    /// Check if this module is using significant CPU.
    pub fn is_hot(&self) -> bool {
        self.cpu_percent > 10.0
    }

    /// Check if this module is overloading.
    pub fn is_overloading(&self) -> bool {
        self.cpu_percent > 50.0
    }
}

/// Tracks CPU usage for a single module.
#[derive(Debug)]
struct ModuleTracker {
    /// Accumulated processing time for current measurement window.
    accumulated_time: Duration,
    /// Number of blocks in current window.
    block_count: u32,
    /// Peak time in current window.
    peak_time: Duration,
    /// Current stats.
    stats: ModuleCpuStats,
    /// Smoothing factor for exponential moving average.
    smoothing: f32,
}

impl ModuleTracker {
    fn new() -> Self {
        Self {
            accumulated_time: Duration::ZERO,
            block_count: 0,
            peak_time: Duration::ZERO,
            stats: ModuleCpuStats::default(),
            smoothing: 0.1, // 10% new, 90% old
        }
    }

    fn record(&mut self, process_time: Duration) {
        self.accumulated_time += process_time;
        self.block_count += 1;
        if process_time > self.peak_time {
            self.peak_time = process_time;
        }
    }

    fn update_stats(&mut self, block_duration: Duration) {
        if self.block_count == 0 {
            return;
        }

        let avg_time = self.accumulated_time / self.block_count;
        let cpu_percent = (avg_time.as_secs_f32() / block_duration.as_secs_f32()) * 100.0;

        // Exponential moving average for smooth display
        self.stats.cpu_percent =
            self.stats.cpu_percent * (1.0 - self.smoothing) + cpu_percent * self.smoothing;
        self.stats.avg_process_time = avg_time;
        self.stats.peak_process_time = self.peak_time;
        self.stats.blocks_processed += self.block_count as u64;

        // Reset for next window
        self.accumulated_time = Duration::ZERO;
        self.block_count = 0;
        // Keep peak for a bit longer (decay slowly)
        self.peak_time = Duration::from_secs_f32(self.peak_time.as_secs_f32() * 0.95);
    }
}

/// Tracks CPU usage across all modules.
pub struct ModuleCpuTracker {
    /// Per-module trackers.
    trackers: HashMap<ModuleId, ModuleTracker>,
    /// Block size in samples.
    block_size: usize,
    /// Sample rate.
    sample_rate: f32,
    /// Duration of one block.
    block_duration: Duration,
    /// Update interval for stats.
    update_interval: Duration,
    /// Last update time.
    last_update: Instant,
    /// Total accumulated time across all modules.
    total_accumulated: Duration,
    /// Total block count for this window.
    total_blocks: u32,
}

impl ModuleCpuTracker {
    /// Create a new CPU tracker.
    pub fn new(block_size: usize, sample_rate: f32) -> Self {
        let block_duration = Duration::from_secs_f64(block_size as f64 / sample_rate as f64);

        Self {
            trackers: HashMap::new(),
            block_size,
            sample_rate,
            block_duration,
            update_interval: Duration::from_millis(100), // Update every 100ms
            last_update: Instant::now(),
            total_accumulated: Duration::ZERO,
            total_blocks: 0,
        }
    }

    /// Record processing time for a module.
    pub fn record(&mut self, module_id: ModuleId, process_time: Duration) {
        self.trackers
            .entry(module_id)
            .or_insert_with(ModuleTracker::new)
            .record(process_time);

        self.total_accumulated += process_time;
    }

    /// Start timing a module (returns a guard that records on drop).
    pub fn start_timing(&mut self, module_id: ModuleId) -> TimingGuard<'_> {
        TimingGuard {
            module_id,
            start: Instant::now(),
            tracker: self,
        }
    }

    /// Mark end of a block (call after processing all modules).
    pub fn end_block(&mut self) {
        self.total_blocks += 1;

        // Check if it's time to update stats
        if self.last_update.elapsed() >= self.update_interval {
            self.update_all_stats();
            self.last_update = Instant::now();
        }
    }

    fn update_all_stats(&mut self) {
        self.trackers
            .values_mut()
            .for_each(|tracker| tracker.update_stats(self.block_duration));

        // Reset totals
        self.total_accumulated = Duration::ZERO;
        self.total_blocks = 0;
    }

    /// Get stats for a specific module.
    pub fn get_stats(&self, module_id: ModuleId) -> Option<&ModuleCpuStats> {
        self.trackers.get(&module_id).map(|t| &t.stats)
    }

    /// Get stats for all modules.
    pub fn all_stats(&self) -> impl Iterator<Item = (ModuleId, &ModuleCpuStats)> {
        self.trackers.iter().map(|(id, t)| (*id, &t.stats))
    }

    /// Get total CPU usage across all modules.
    pub fn total_cpu_percent(&self) -> f32 {
        self.trackers.values().map(|t| t.stats.cpu_percent).sum()
    }

    /// Get the top N CPU-consuming modules.
    pub fn top_modules(&self, n: usize) -> Vec<(ModuleId, f32)> {
        let mut modules: Vec<_> = self
            .trackers
            .iter()
            .map(|(id, t)| (*id, t.stats.cpu_percent))
            .collect();

        modules.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        modules.truncate(n);
        modules
    }

    /// Remove tracking for a module.
    pub fn remove_module(&mut self, module_id: ModuleId) {
        self.trackers.remove(&module_id);
    }

    /// Clear all tracking data.
    pub fn clear(&mut self) {
        self.trackers.clear();
        self.total_accumulated = Duration::ZERO;
        self.total_blocks = 0;
    }

    /// Get modules that are overloading.
    pub fn overloading_modules(&self) -> Vec<ModuleId> {
        self.trackers
            .iter()
            .filter(|(_, t)| t.stats.is_overloading())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Update configuration (e.g., after sample rate change).
    pub fn configure(&mut self, block_size: usize, sample_rate: f32) {
        self.block_size = block_size;
        self.sample_rate = sample_rate;
        self.block_duration = Duration::from_secs_f64(block_size as f64 / sample_rate as f64);
    }
}

/// RAII guard for timing module processing.
pub struct TimingGuard<'a> {
    module_id: ModuleId,
    start: Instant,
    tracker: &'a mut ModuleCpuTracker,
}

impl<'a> Drop for TimingGuard<'a> {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        self.tracker.record(self.module_id, elapsed);
    }
}

/// Lightweight timing for use in audio callback (avoids allocation).
#[derive(Debug, Clone, Copy)]
pub struct ModuleTiming {
    /// Module being timed.
    pub module_id: ModuleId,
    /// Processing time in nanoseconds.
    pub nanos: u64,
}

impl ModuleTiming {
    pub fn new(module_id: ModuleId, duration: Duration) -> Self {
        Self {
            module_id,
            nanos: duration.as_nanos() as u64,
        }
    }

    pub fn duration(&self) -> Duration {
        Duration::from_nanos(self.nanos)
    }
}

/// Fixed-size buffer for collecting timing data in audio callback.
pub struct TimingBuffer {
    /// Timing entries.
    entries: [Option<ModuleTiming>; 64],
    /// Number of entries.
    count: usize,
}

impl TimingBuffer {
    pub const fn new() -> Self {
        Self {
            entries: [None; 64],
            count: 0,
        }
    }

    /// Add a timing entry.
    pub fn push(&mut self, timing: ModuleTiming) {
        if self.count < self.entries.len() {
            self.entries[self.count] = Some(timing);
            self.count += 1;
        }
    }

    /// Get all entries.
    pub fn entries(&self) -> impl Iterator<Item = &ModuleTiming> {
        self.entries[..self.count].iter().filter_map(|e| e.as_ref())
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.count = 0;
        // No need to clear entries, we track via count
    }

    /// Check if buffer is full.
    pub fn is_full(&self) -> bool {
        self.count >= self.entries.len()
    }

    /// Get entry count.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl Default for TimingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::typed_params::ModuleType;

    #[test]
    fn test_module_tracker() {
        let mut tracker = ModuleTracker::new();

        // Record some times
        tracker.record(Duration::from_micros(100));
        tracker.record(Duration::from_micros(150));
        tracker.record(Duration::from_micros(120));

        // Block duration for 128 samples at 44100 Hz = ~2.9ms
        let block_duration = Duration::from_secs_f64(128.0 / 44100.0);
        tracker.update_stats(block_duration);

        assert!(tracker.stats.cpu_percent > 0.0);
        assert_eq!(tracker.stats.blocks_processed, 3);
    }

    #[test]
    fn test_cpu_tracker() {
        let mut tracker = ModuleCpuTracker::new(128, 44100.0);

        let osc1 = ModuleId::new(ModuleType::Oscillator, 1);
        let filter = ModuleId::new(ModuleType::Filter, 1);

        // Record some processing times
        tracker.record(osc1, Duration::from_micros(50));
        tracker.record(filter, Duration::from_micros(100));
        tracker.end_block();

        tracker.record(osc1, Duration::from_micros(45));
        tracker.record(filter, Duration::from_micros(95));
        tracker.end_block();

        // Force stats update
        std::thread::sleep(Duration::from_millis(110));
        tracker.record(osc1, Duration::from_micros(48));
        tracker.end_block();

        // Check we have stats
        let osc_stats = tracker.get_stats(osc1);
        assert!(osc_stats.is_some());

        let filter_stats = tracker.get_stats(filter);
        assert!(filter_stats.is_some());
    }

    #[test]
    fn test_timing_buffer() {
        let mut buffer = TimingBuffer::new();
        assert!(buffer.is_empty());

        let osc1 = ModuleId::new(ModuleType::Oscillator, 1);
        buffer.push(ModuleTiming::new(osc1, Duration::from_micros(100)));

        assert_eq!(buffer.len(), 1);
        assert!(!buffer.is_empty());

        buffer.clear();
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_top_modules() {
        let mut tracker = ModuleCpuTracker::new(128, 44100.0);

        let osc1 = ModuleId::new(ModuleType::Oscillator, 1);
        let osc2 = ModuleId::new(ModuleType::Oscillator, 2);
        let filter = ModuleId::new(ModuleType::Filter, 1);

        // Give filter the most CPU time
        for _ in 0..10 {
            tracker.record(osc1, Duration::from_micros(50));
            tracker.record(osc2, Duration::from_micros(30));
            tracker.record(filter, Duration::from_micros(200));
            tracker.end_block();
        }

        // Force update
        std::thread::sleep(Duration::from_millis(110));
        tracker.record(osc1, Duration::from_micros(50));
        tracker.end_block();

        let top = tracker.top_modules(2);
        assert!(!top.is_empty());
        // Filter should be first (highest CPU)
        assert_eq!(top[0].0, filter);
    }
}
