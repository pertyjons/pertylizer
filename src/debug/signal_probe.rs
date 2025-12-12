//! Signal probe for inspecting audio buffer values.
//!
//! Allows recording and analyzing sample values at specific points in the graph.

use std::collections::HashMap;

use crate::engine::ModuleId;

/// A point in the graph to probe.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ProbePoint {
    /// Module to probe.
    pub module_id: ModuleId,
    /// Port name to probe.
    pub port_name: String,
}

impl ProbePoint {
    /// Create a new probe point.
    pub fn new(module_id: ModuleId, port_name: impl Into<String>) -> Self {
        Self {
            module_id,
            port_name: port_name.into(),
        }
    }
}

/// Statistics calculated from recorded samples.
#[derive(Debug, Clone)]
pub struct SignalStats {
    /// Minimum sample value.
    pub min: f32,
    /// Maximum sample value.
    pub max: f32,
    /// Root mean square (average power).
    pub rms: f32,
    /// Peak absolute value.
    pub peak: f32,
    /// DC offset (average value).
    pub dc_offset: f32,
    /// True if all samples are exactly 0.0.
    pub is_silent: bool,
    /// Number of samples analyzed.
    pub sample_count: usize,
}

impl SignalStats {
    /// Calculate statistics from a buffer.
    pub fn from_samples(samples: &[f32]) -> Self {
        if samples.is_empty() {
            return Self {
                min: 0.0,
                max: 0.0,
                rms: 0.0,
                peak: 0.0,
                dc_offset: 0.0,
                is_silent: true,
                sample_count: 0,
            };
        }

        let mut min = f32::MAX;
        let mut max = f32::MIN;
        let mut sum = 0.0_f64;
        let mut sum_squared = 0.0_f64;
        let mut all_zero = true;

        for &sample in samples {
            if sample != 0.0 {
                all_zero = false;
            }
            if sample < min {
                min = sample;
            }
            if sample > max {
                max = sample;
            }
            sum += sample as f64;
            sum_squared += (sample as f64) * (sample as f64);
        }

        let n = samples.len() as f64;
        let dc_offset = (sum / n) as f32;
        let rms = (sum_squared / n).sqrt() as f32;
        let peak = min.abs().max(max.abs());

        Self {
            min,
            max,
            rms,
            peak,
            dc_offset,
            is_silent: all_zero,
            sample_count: samples.len(),
        }
    }
}

/// Comparison between two signals.
#[derive(Debug)]
pub struct SignalComparison {
    /// Correlation coefficient (-1.0 to 1.0).
    /// 1.0 = identical, -1.0 = inverted, 0.0 = uncorrelated.
    pub correlation: f32,
    /// RMS of the difference signal.
    pub rms_difference: f32,
    /// True if signals are sample-identical.
    pub are_identical: bool,
}

impl SignalComparison {
    /// Compare two sample buffers.
    pub fn compare(a: &[f32], b: &[f32]) -> Self {
        if a.is_empty() || b.is_empty() || a.len() != b.len() {
            return Self {
                correlation: 0.0,
                rms_difference: f32::NAN,
                are_identical: false,
            };
        }

        let n = a.len() as f64;

        // Calculate means
        let mean_a: f64 = a.iter().map(|&x| x as f64).sum::<f64>() / n;
        let mean_b: f64 = b.iter().map(|&x| x as f64).sum::<f64>() / n;

        // Calculate correlation and difference
        let mut sum_ab = 0.0_f64;
        let mut sum_aa = 0.0_f64;
        let mut sum_bb = 0.0_f64;
        let mut sum_diff_sq = 0.0_f64;
        let mut all_identical = true;

        for i in 0..a.len() {
            let da = a[i] as f64 - mean_a;
            let db = b[i] as f64 - mean_b;
            sum_ab += da * db;
            sum_aa += da * da;
            sum_bb += db * db;

            let diff = a[i] as f64 - b[i] as f64;
            sum_diff_sq += diff * diff;

            if a[i] != b[i] {
                all_identical = false;
            }
        }

        let denom = (sum_aa * sum_bb).sqrt();
        let correlation = if denom > 0.0 {
            (sum_ab / denom) as f32
        } else {
            0.0
        };

        let rms_difference = (sum_diff_sq / n).sqrt() as f32;

        Self {
            correlation,
            rms_difference,
            are_identical: all_identical,
        }
    }
}

/// Probe for recording and analyzing signal values at specific points.
#[derive(Debug)]
pub struct SignalProbe {
    /// Recorded samples per probe point.
    recordings: HashMap<ProbePoint, Vec<f32>>,
    /// Maximum samples to keep per probe (prevents unbounded memory growth).
    max_samples: usize,
}

impl SignalProbe {
    /// Create a new signal probe with default settings.
    pub fn new() -> Self {
        Self {
            recordings: HashMap::new(),
            max_samples: 1_000_000, // ~22 seconds at 44.1kHz
        }
    }

    /// Create a probe with a custom sample limit.
    pub fn with_max_samples(max_samples: usize) -> Self {
        Self {
            recordings: HashMap::new(),
            max_samples,
        }
    }

    /// Add a probe point (pre-allocate storage).
    pub fn add_probe(&mut self, module_id: ModuleId, port: impl Into<String>) {
        let point = ProbePoint::new(module_id, port);
        self.recordings.entry(point).or_default();
    }

    /// Record samples from a buffer.
    ///
    /// Called during graph processing to capture signal values.
    pub fn record(&mut self, module_id: ModuleId, port: &str, buffer: &[f32]) {
        let point = ProbePoint::new(module_id, port);
        if let Some(recording) = self.recordings.get_mut(&point) {
            // Respect sample limit
            let available = self.max_samples.saturating_sub(recording.len());
            let to_copy = buffer.len().min(available);
            recording.extend_from_slice(&buffer[..to_copy]);
        }
    }

    /// Record samples using a ProbePoint reference.
    pub fn record_at(&mut self, point: &ProbePoint, buffer: &[f32]) {
        if let Some(recording) = self.recordings.get_mut(point) {
            let available = self.max_samples.saturating_sub(recording.len());
            let to_copy = buffer.len().min(available);
            recording.extend_from_slice(&buffer[..to_copy]);
        }
    }

    /// Get recorded samples for a probe point.
    pub fn samples(&self, module_id: ModuleId, port: &str) -> Option<&[f32]> {
        let point = ProbePoint::new(module_id, port);
        self.recordings.get(&point).map(|v| v.as_slice())
    }

    /// Get samples using a ProbePoint reference.
    pub fn samples_at(&self, point: &ProbePoint) -> Option<&[f32]> {
        self.recordings.get(point).map(|v| v.as_slice())
    }

    /// Calculate statistics for a probe point.
    pub fn stats(&self, module_id: ModuleId, port: &str) -> Option<SignalStats> {
        self.samples(module_id, port).map(SignalStats::from_samples)
    }

    /// Calculate statistics using a ProbePoint reference.
    pub fn stats_at(&self, point: &ProbePoint) -> Option<SignalStats> {
        self.samples_at(point).map(SignalStats::from_samples)
    }

    /// Compare two probe points.
    pub fn compare(&self, a: &ProbePoint, b: &ProbePoint) -> Option<SignalComparison> {
        let samples_a = self.samples_at(a)?;
        let samples_b = self.samples_at(b)?;
        Some(SignalComparison::compare(samples_a, samples_b))
    }

    /// Clear all recordings.
    pub fn clear(&mut self) {
        for recording in self.recordings.values_mut() {
            recording.clear();
        }
    }

    /// Clear recording for a specific probe point.
    pub fn clear_at(&mut self, point: &ProbePoint) {
        if let Some(recording) = self.recordings.get_mut(point) {
            recording.clear();
        }
    }

    /// Remove a probe point entirely.
    pub fn remove_probe(&mut self, module_id: ModuleId, port: &str) {
        let point = ProbePoint::new(module_id, port);
        self.recordings.remove(&point);
    }

    /// Get all active probe points.
    pub fn probe_points(&self) -> impl Iterator<Item = &ProbePoint> {
        self.recordings.keys()
    }

    /// Check if a probe point has any recorded samples.
    pub fn has_samples(&self, module_id: ModuleId, port: &str) -> bool {
        let point = ProbePoint::new(module_id, port);
        self.recordings
            .get(&point)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    /// Get the total number of samples recorded across all probe points.
    pub fn total_samples(&self) -> usize {
        self.recordings.values().map(|v| v.len()).sum()
    }

    /// Get a summary of all recorded signals.
    pub fn summary(&self) -> Vec<(ProbePoint, SignalStats)> {
        self.recordings
            .iter()
            .filter(|(_, samples)| !samples.is_empty())
            .map(|(point, samples)| (point.clone(), SignalStats::from_samples(samples)))
            .collect()
    }
}

impl Default for SignalProbe {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::engine::typed_params::ModuleType;

    fn test_module_id() -> ModuleId {
        ModuleId::new(ModuleType::Envelope, 1)
    }

    #[test]
    fn test_signal_stats_empty() {
        let stats = SignalStats::from_samples(&[]);
        assert!(stats.is_silent);
        assert_eq!(stats.sample_count, 0);
    }

    #[test]
    fn test_signal_stats_silence() {
        let samples = vec![0.0; 100];
        let stats = SignalStats::from_samples(&samples);
        assert!(stats.is_silent);
        assert_eq!(stats.min, 0.0);
        assert_eq!(stats.max, 0.0);
        assert_eq!(stats.rms, 0.0);
    }

    #[test]
    fn test_signal_stats_sine() {
        // Full period sine wave (2*PI radians = 1 full cycle, centered at 0)
        let samples: Vec<f32> = (0..628) // 100 samples per radian * 2*PI
            .map(|i| (i as f32 * 0.01).sin())
            .collect();
        let stats = SignalStats::from_samples(&samples);

        assert!(!stats.is_silent);
        assert!(stats.min < 0.0);
        assert!(stats.max > 0.0);
        assert!(stats.rms > 0.0);
        assert!(
            stats.dc_offset.abs() < 0.1,
            "DC offset was {}",
            stats.dc_offset
        );
    }

    #[test]
    fn test_signal_comparison_identical() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![1.0, 2.0, 3.0, 4.0];
        let cmp = SignalComparison::compare(&a, &b);

        assert!(cmp.are_identical);
        assert!((cmp.correlation - 1.0).abs() < 0.001);
        assert_eq!(cmp.rms_difference, 0.0);
    }

    #[test]
    fn test_signal_comparison_inverted() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![-1.0, -2.0, -3.0, -4.0];
        let cmp = SignalComparison::compare(&a, &b);

        assert!(!cmp.are_identical);
        assert!((cmp.correlation - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_probe_record_and_retrieve() {
        let mut probe = SignalProbe::new();
        let id = test_module_id();

        probe.add_probe(id, "out");
        probe.record(id, "out", &[0.5, 0.6, 0.7]);
        probe.record(id, "out", &[0.8, 0.9]);

        let samples = probe.samples(id, "out").unwrap();
        assert_eq!(samples, &[0.5, 0.6, 0.7, 0.8, 0.9]);
    }

    #[test]
    fn test_probe_stats() {
        let mut probe = SignalProbe::new();
        let id = test_module_id();

        probe.add_probe(id, "out");
        probe.record(id, "out", &[0.0, 0.5, 1.0, 0.5, 0.0]);

        let stats = probe.stats(id, "out").unwrap();
        assert!(!stats.is_silent);
        assert_eq!(stats.min, 0.0);
        assert_eq!(stats.max, 1.0);
        assert_eq!(stats.peak, 1.0);
    }

    #[test]
    fn test_probe_max_samples_limit() {
        let mut probe = SignalProbe::with_max_samples(10);
        let id = test_module_id();

        probe.add_probe(id, "out");
        probe.record(id, "out", &[1.0; 20]); // Try to record 20 samples

        let samples = probe.samples(id, "out").unwrap();
        assert_eq!(samples.len(), 10); // Should be capped at 10
    }

    #[test]
    fn test_probe_clear() {
        let mut probe = SignalProbe::new();
        let id = test_module_id();

        probe.add_probe(id, "out");
        probe.record(id, "out", &[1.0, 2.0, 3.0]);
        assert!(probe.has_samples(id, "out"));

        probe.clear();
        assert!(!probe.has_samples(id, "out"));
    }

    #[test]
    fn test_probe_summary() {
        let mut probe = SignalProbe::new();
        let id1 = ModuleId::new(ModuleType::Envelope, 1);
        let id2 = ModuleId::new(ModuleType::Amplifier, 1);

        probe.add_probe(id1, "out");
        probe.add_probe(id2, "out");
        probe.record(id1, "out", &[0.5; 10]);
        // id2 has no samples

        let summary = probe.summary();
        assert_eq!(summary.len(), 1); // Only id1 has samples
    }
}
