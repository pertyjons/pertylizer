//! Visualization modules for signal analysis.
//!
//! These modules don't modify the audio signal - they pass it through
//! while capturing data for visual display in the GUI.

mod level_meter;
mod oscilloscope;
mod spectrum_analyzer;

pub use level_meter::LevelMeter;
pub use oscilloscope::Oscilloscope;
pub use spectrum_analyzer::SpectrumAnalyzer;

use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Atomic float wrapper for lock-free level sharing.
#[derive(Debug)]
struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn new(val: f32) -> Self {
        Self(AtomicU32::new(val.to_bits()))
    }

    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    fn store(&self, val: f32) {
        self.0.store(val.to_bits(), Ordering::Relaxed);
    }
}

/// Lock-free shared buffer for passing visualization data from audio thread to GUI.
///
/// Uses ring buffers for waveform data and atomics for levels.
/// The audio thread never blocks - if the GUI is slow, old samples are simply overwritten.
pub struct VisualizationBuffer {
    /// Ring buffer producer for left channel (audio thread writes).
    samples_l_prod: parking_lot::Mutex<ringbuf::HeapProd<f32>>,
    /// Ring buffer consumer for left channel (GUI reads).
    samples_l_cons: parking_lot::Mutex<ringbuf::HeapCons<f32>>,
    /// Ring buffer producer for right channel.
    samples_r_prod: parking_lot::Mutex<ringbuf::HeapProd<f32>>,
    /// Ring buffer consumer for right channel.
    samples_r_cons: parking_lot::Mutex<ringbuf::HeapCons<f32>>,
    /// Buffer size.
    pub size: usize,
    /// Current peak levels (atomic for lock-free access).
    peak_l: AtomicF32,
    peak_r: AtomicF32,
    /// RMS levels (atomic).
    rms_l: AtomicF32,
    rms_r: AtomicF32,
    /// Snapshot of samples for GUI (updated periodically).
    snapshot_l: parking_lot::Mutex<Vec<f32>>,
    snapshot_r: parking_lot::Mutex<Vec<f32>>,

    // Sweep data for triggered oscilloscope display (SignalMonitor)
    /// Pre-allocated sweep buffer. Written by audio thread (try_lock),
    /// read by GUI thread (blocking lock).
    sweep_data: parking_lot::Mutex<Vec<f32>>,
    /// Generation counter bumped on each successful sweep write.
    sweep_generation: AtomicU32,
    /// `voice_start_time` of the last voice that wrote a sweep.
    /// Used for "newest voice wins" arbitration.
    sweep_last_writer: AtomicU64,

    // Sample playback visualization data (lock-free)
    /// Normalized playback position (0.0-1.0).
    sample_position: AtomicF32,
    /// Normalized loop start position (0.0-1.0).
    sample_loop_start: AtomicF32,
    /// Normalized loop end position (0.0-1.0).
    sample_loop_end: AtomicF32,
    /// Loop enabled flag (1.0 = enabled, 0.0 = disabled).
    sample_loop_enabled: AtomicF32,
}

impl std::fmt::Debug for VisualizationBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisualizationBuffer")
            .field("size", &self.size)
            .finish()
    }
}

impl VisualizationBuffer {
    pub fn new(size: usize) -> Self {
        let rb_l = HeapRb::<f32>::new(size);
        let (prod_l, cons_l) = rb_l.split();

        let rb_r = HeapRb::<f32>::new(size);
        let (prod_r, cons_r) = rb_r.split();

        Self {
            samples_l_prod: parking_lot::Mutex::new(prod_l),
            samples_l_cons: parking_lot::Mutex::new(cons_l),
            samples_r_prod: parking_lot::Mutex::new(prod_r),
            samples_r_cons: parking_lot::Mutex::new(cons_r),
            size,
            peak_l: AtomicF32::new(0.0),
            peak_r: AtomicF32::new(0.0),
            rms_l: AtomicF32::new(0.0),
            rms_r: AtomicF32::new(0.0),
            snapshot_l: parking_lot::Mutex::new(vec![0.0; size]),
            snapshot_r: parking_lot::Mutex::new(vec![0.0; size]),
            // Sweep buffer pre-allocated with generous capacity
            sweep_data: parking_lot::Mutex::new(Vec::with_capacity(8192)),
            sweep_generation: AtomicU32::new(0),
            sweep_last_writer: AtomicU64::new(0),
            // Sample playback visualization defaults
            sample_position: AtomicF32::new(0.0),
            sample_loop_start: AtomicF32::new(0.0),
            sample_loop_end: AtomicF32::new(1.0),
            sample_loop_enabled: AtomicF32::new(0.0),
        }
    }

    /// Write samples to the buffer (called from audio thread).
    /// This is lock-free from the audio thread's perspective - uses try_lock.
    pub fn write_samples(&self, left: &[f32], right: &[f32]) {
        // Try to get the producer - if GUI has it locked, skip this update
        // This is safe because we're just visualization data, not critical audio
        if let Some(mut prod_l) = self.samples_l_prod.try_lock()
            && let Some(mut prod_r) = self.samples_r_prod.try_lock()
        {
            let len = left.len().min(right.len());

            // Push samples, dropping old ones if buffer is full
            for i in 0..len {
                // If buffer is full, we just skip (producer will overwrite oldest)
                let _ = prod_l.try_push(left[i]);
                let _ = prod_r.try_push(right[i]);
            }
        }
    }

    /// Write interleaved stereo samples directly (no allocation needed).
    /// Input format: [L, R, L, R, ...]
    /// This is the preferred method from the audio thread.
    pub fn write_interleaved(&self, interleaved: &[f32]) {
        if let Some(mut prod_l) = self.samples_l_prod.try_lock()
            && let Some(mut prod_r) = self.samples_r_prod.try_lock()
        {
            // Process interleaved pairs [L, R, L, R, ...]
            for frame in interleaved.chunks(2) {
                if frame.len() >= 2 {
                    let _ = prod_l.try_push(frame[0]);
                    let _ = prod_r.try_push(frame[1]);
                }
            }
        }
    }

    /// Update peak and RMS levels from interleaved buffer (no allocation).
    /// Input format: [L, R, L, R, ...]
    #[allow(clippy::similar_names)] // L/R stereo pairs: peak_l/peak_r, rms_l/rms_r
    pub fn update_levels_interleaved(&self, interleaved: &[f32]) {
        if interleaved.len() < 2 {
            return;
        }

        let num_frames = interleaved.len() / 2;
        let mut peak_l_val = 0.0f32;
        let mut peak_r_val = 0.0f32;
        let mut sum_l = 0.0f32;
        let mut sum_r = 0.0f32;

        for frame in interleaved.chunks(2) {
            if frame.len() >= 2 {
                let l = frame[0];
                let r = frame[1];
                peak_l_val = peak_l_val.max(l.abs());
                peak_r_val = peak_r_val.max(r.abs());
                sum_l += l * l;
                sum_r += r * r;
            }
        }

        let rms_l_val = (sum_l / num_frames as f32).sqrt();
        let rms_r_val = (sum_r / num_frames as f32).sqrt();

        // Update with peak hold decay (lock-free)
        let old_peak_l = self.peak_l.load();
        self.peak_l.store(peak_l_val.max(old_peak_l * 0.995));

        let old_peak_r = self.peak_r.load();
        self.peak_r.store(peak_r_val.max(old_peak_r * 0.995));

        // Smooth RMS (lock-free)
        let old_rms_l = self.rms_l.load();
        self.rms_l.store(old_rms_l * 0.9 + rms_l_val * 0.1);

        let old_rms_r = self.rms_r.load();
        self.rms_r.store(old_rms_r * 0.9 + rms_r_val * 0.1);
    }

    /// Update peak and RMS levels (lock-free using atomics).
    #[allow(clippy::similar_names)] // L/R stereo pairs: peak_l/peak_r, rms_l/rms_r
    pub fn update_levels(&self, left: &[f32], right: &[f32]) {
        if left.is_empty() || right.is_empty() {
            return;
        }

        // Calculate peak
        let peak_l_val = left.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        let peak_r_val = right.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

        // Calculate RMS
        let rms_l_val = (left.iter().map(|s| s * s).sum::<f32>() / left.len() as f32).sqrt();
        let rms_r_val = (right.iter().map(|s| s * s).sum::<f32>() / right.len() as f32).sqrt();

        // Update with peak hold decay (lock-free)
        let old_peak_l = self.peak_l.load();
        self.peak_l.store(peak_l_val.max(old_peak_l * 0.995));

        let old_peak_r = self.peak_r.load();
        self.peak_r.store(peak_r_val.max(old_peak_r * 0.995));

        // Smooth RMS (lock-free)
        let old_rms_l = self.rms_l.load();
        self.rms_l.store(old_rms_l * 0.9 + rms_l_val * 0.1);

        let old_rms_r = self.rms_r.load();
        self.rms_r.store(old_rms_r * 0.9 + rms_r_val * 0.1);
    }

    /// Read samples for display (called from GUI thread).
    /// Returns a snapshot of the most recent samples.
    pub fn read_samples(&self) -> (Vec<f32>, Vec<f32>) {
        // Drain the ring buffers into snapshots
        let mut snapshot_l = self.snapshot_l.lock();
        let mut snapshot_r = self.snapshot_r.lock();

        if let Some(mut cons_l) = self.samples_l_cons.try_lock()
            && let Some(mut cons_r) = self.samples_r_cons.try_lock()
        {
            // Count available samples first
            let avail_l = cons_l.occupied_len();
            let avail_r = cons_r.occupied_len();

            if avail_l > 0 {
                // Rotate snapshot left by the number of new samples (O(n) but only once)
                let len_l = snapshot_l.len();
                let drain_count = avail_l.min(len_l);
                snapshot_l.drain(0..drain_count);
                // Append new samples
                while let Some(sample) = cons_l.try_pop() {
                    snapshot_l.push(sample);
                }
            }

            if avail_r > 0 {
                let len_r = snapshot_r.len();
                let drain_count = avail_r.min(len_r);
                snapshot_r.drain(0..drain_count);
                while let Some(sample) = cons_r.try_pop() {
                    snapshot_r.push(sample);
                }
            }
        }

        (snapshot_l.clone(), snapshot_r.clone())
    }

    /// Copy the left-channel snapshot into `dst` with windowing applied, without
    /// draining the ring buffer.
    ///
    /// Copies the last `dst.len()` samples from the snapshot into `dst`,
    /// multiplied element-wise by `window`. If the snapshot is shorter than `dst`,
    /// the remaining elements are zeroed. Unlike [`read_samples`], this only locks
    /// the snapshot mutex (not the consumer), so it does not compete with the GUI.
    pub fn copy_snapshot_windowed_into(&self, dst: &mut [f32], window: &[f32]) {
        let snapshot = self.snapshot_l.lock();
        let copy_len = snapshot.len().min(dst.len());
        let src_offset = snapshot.len().saturating_sub(copy_len);
        for i in 0..copy_len {
            dst[i] = snapshot[src_offset + i] * window[i];
        }
        for sample in dst.iter_mut().skip(copy_len) {
            *sample = 0.0;
        }
    }

    /// Get current peak levels (lock-free).
    pub fn get_peaks(&self) -> (f32, f32) {
        (self.peak_l.load(), self.peak_r.load())
    }

    /// Get current RMS levels (lock-free).
    pub fn get_rms(&self) -> (f32, f32) {
        (self.rms_l.load(), self.rms_r.load())
    }

    /// Reset peak hold (lock-free).
    pub fn reset_peaks(&self) {
        self.peak_l.store(0.0);
        self.peak_r.store(0.0);
    }

    // ==================== Sample Playback Visualization ====================

    /// Update sample playback visualization data (called from audio thread).
    ///
    /// All values are normalized (0.0-1.0).
    /// This is lock-free and safe to call from the audio thread.
    pub fn set_sample_playback(
        &self,
        position: f32,
        loop_start: f32,
        loop_end: f32,
        loop_enabled: bool,
    ) {
        self.sample_position.store(position);
        self.sample_loop_start.store(loop_start);
        self.sample_loop_end.store(loop_end);
        self.sample_loop_enabled
            .store(if loop_enabled { 1.0 } else { 0.0 });
    }

    /// Get sample playback visualization data (called from GUI thread).
    ///
    /// Returns `(position, loop_start, loop_end, loop_enabled)`.
    /// All position values are normalized (0.0-1.0).
    #[must_use]
    pub fn get_sample_playback(&self) -> (f32, f32, f32, bool) {
        (
            self.sample_position.load(),
            self.sample_loop_start.load(),
            self.sample_loop_end.load(),
            self.sample_loop_enabled.load() > 0.5,
        )
    }

    /// Update only the playback position (for efficiency when loop params don't change).
    pub fn set_sample_position(&self, position: f32) {
        self.sample_position.store(position);
    }

    /// Get current sample playback position (lock-free).
    #[must_use]
    pub fn get_sample_position(&self) -> f32 {
        self.sample_position.load()
    }

    /// Read the latest sweep data (called from GUI thread).
    ///
    /// Returns `None` if no sweep has been written yet.
    /// Blocking lock is OK here — only the GUI thread calls this.
    #[must_use]
    pub fn read_sweep(&self) -> Option<Vec<f32>> {
        let data = self.sweep_data.lock();
        if data.is_empty() {
            None
        } else {
            Some(data.clone())
        }
    }
}

impl Default for VisualizationBuffer {
    fn default() -> Self {
        Self::new(2048)
    }
}

// Note: We can't derive Clone because of the ring buffer split,
// but we can create a new buffer with the same size
impl Clone for VisualizationBuffer {
    fn clone(&self) -> Self {
        let new = Self::new(self.size);
        // Copy sweep data if any
        if let Some(data) = self.sweep_data.try_lock()
            && let Some(mut new_data) = new.sweep_data.try_lock()
        {
            new_data.clear();
            new_data.extend_from_slice(&data);
        }
        new
    }
}

impl synth_core::VisualizationSink for VisualizationBuffer {
    fn write_vis_samples(&self, left: &[f32], right: &[f32]) {
        self.write_samples(left, right);
    }

    fn write_sweep(&self, samples: &[f32], voice_start_time: u64) -> bool {
        // "Newest voice wins": only accept if this voice started at or after
        // the last writer. Uses Relaxed ordering — exact ordering between
        // concurrent voices is not critical for visualization.
        let last = self.sweep_last_writer.load(Ordering::Relaxed);
        if voice_start_time < last {
            return false;
        }

        // try_lock: RT-safe, never blocks the audio thread
        if let Some(mut data) = self.sweep_data.try_lock() {
            self.sweep_last_writer
                .store(voice_start_time, Ordering::Relaxed);
            // clear + extend reuses existing capacity (no allocation)
            data.clear();
            data.extend_from_slice(samples);
            self.sweep_generation.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}
