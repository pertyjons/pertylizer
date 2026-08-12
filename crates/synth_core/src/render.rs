//! Offline-render configuration shared by the application renderers and the
//! MCP analysis surface.
//!
//! These types describe *what* an offline render reconstructs — which optional
//! signal-chain stages, at what sample rate — with no protocol semantics
//! attached. They live here rather than in `synth_mcp` because the headless
//! render command and the offline arrangement renderer take them too, and
//! application rendering must not depend on MCP request types.

use crate::audio::DeviceSampleRate;

/// Sample rate at which an offline analysis render runs.
///
/// `Full` (44.1 kHz) is the default and the only rate at which every metric is
/// trustworthy — it covers the full audible band (Nyquist 22 kHz). `Draft`
/// (22.05 kHz) roughly halves render time per buffer, which compounds across the
/// per-track renders of `analyze_section`/`analyze_masking_matrix`, but its
/// Nyquist is only 11 kHz, so the `high` energy band is truncated, `true_peak`
/// is less reliable, LUFS is biased (its K-weighting filters are tuned for
/// 44.1 kHz), and distortion-heavy patches alias more. Use `Draft` for quick
/// level/balance/RMS passes; use `Full` when LUFS accuracy, high-band,
/// true-peak, or saturation behavior matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderQuality {
    /// 22.05 kHz — ~2× faster per render, full-band metrics unreliable.
    Draft,
    /// 44.1 kHz — full audible band, all metrics trustworthy (default).
    #[default]
    Full,
}

impl RenderQuality {
    /// 22.05 kHz draft rate.
    pub const DRAFT_SAMPLE_RATE: DeviceSampleRate = DeviceSampleRate::new(22_050);
    /// 44.1 kHz full rate (the analysis default).
    pub const FULL_SAMPLE_RATE: DeviceSampleRate = DeviceSampleRate::CD_QUALITY;

    /// Render sample rate for this quality.
    #[must_use]
    pub fn sample_rate(self) -> DeviceSampleRate {
        match self {
            Self::Draft => Self::DRAFT_SAMPLE_RATE,
            Self::Full => Self::FULL_SAMPLE_RATE,
        }
    }

    /// Parse an optional quality flag string, as exposed by callers'
    /// `render_quality` fields. `Some("draft")` → `Draft`; everything else
    /// (including `None` and unrecognized values) → `Full`, the safe default.
    #[must_use]
    pub fn parse(flag: Option<&str>) -> Self {
        match flag {
            Some(s) if s.eq_ignore_ascii_case("draft") => Self::Draft,
            _ => Self::Full,
        }
    }
}

/// Which optional stages of the signal chain an offline analysis render should
/// reconstruct on top of the dry instrument sum, plus the render sample rate.
///
/// The default (`AnalysisScope::default()`) preserves the historical behavior:
/// instruments and their own effect chains only, return busses summed dry, no
/// master processing, rendered at the full 44.1 kHz rate. Each effect flag opts
/// a stage back in; `render_sample_rate` selects the render resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisScope {
    /// Load the master effect chain (master-bus limiter/EQ/compressor, …).
    pub master_effects: bool,
    /// Load each return bus's effect chain (else returns stay dry).
    pub return_effects: bool,
    /// Sample rate the offline render runs at (see [`RenderQuality`]).
    pub render_sample_rate: DeviceSampleRate,
}

impl Default for AnalysisScope {
    fn default() -> Self {
        Self {
            master_effects: false,
            return_effects: false,
            render_sample_rate: RenderQuality::FULL_SAMPLE_RATE,
        }
    }
}

impl AnalysisScope {
    /// Build a scope from the optional per-stage flags. `all` turns on every
    /// effect stage; the per-stage flags OR in on top of it. Every `None`
    /// effect flag resolves to `false`, so omitting them yields the dry
    /// default. `quality` selects the render resolution
    /// (`RenderQuality::default()` = full).
    #[must_use]
    pub fn from_flags(
        all: Option<bool>,
        master_effects: Option<bool>,
        return_effects: Option<bool>,
        quality: RenderQuality,
    ) -> Self {
        let all = all.unwrap_or(false);
        Self {
            master_effects: all || master_effects.unwrap_or(false),
            return_effects: all || return_effects.unwrap_or(false),
            render_sample_rate: quality.sample_rate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_maps_to_expected_rates() {
        assert_eq!(RenderQuality::Draft.sample_rate().as_u32(), 22_050);
        assert_eq!(RenderQuality::Full.sample_rate().as_u32(), 44_100);
        assert_eq!(RenderQuality::default(), RenderQuality::Full);
    }

    #[test]
    fn parse_accepts_draft_case_insensitively() {
        assert_eq!(RenderQuality::parse(Some("draft")), RenderQuality::Draft);
        assert_eq!(RenderQuality::parse(Some("DRAFT")), RenderQuality::Draft);
        assert_eq!(RenderQuality::parse(Some("Draft")), RenderQuality::Draft);
    }

    #[test]
    fn parse_falls_back_to_full() {
        assert_eq!(RenderQuality::parse(None), RenderQuality::Full);
        assert_eq!(RenderQuality::parse(Some("full")), RenderQuality::Full);
        assert_eq!(RenderQuality::parse(Some("garbage")), RenderQuality::Full);
        assert_eq!(RenderQuality::parse(Some("")), RenderQuality::Full);
    }

    #[test]
    fn from_flags_all_turns_every_stage_on() {
        let scope = AnalysisScope::from_flags(Some(true), None, None, RenderQuality::Full);
        assert!(scope.master_effects);
        assert!(scope.return_effects);
    }

    #[test]
    fn from_flags_per_stage_flags_or_in() {
        let scope = AnalysisScope::from_flags(None, Some(true), None, RenderQuality::Full);
        assert!(scope.master_effects);
        assert!(!scope.return_effects);

        // `all: false` must not veto an explicit per-stage `true`.
        let scope = AnalysisScope::from_flags(Some(false), None, Some(true), RenderQuality::Full);
        assert!(!scope.master_effects);
        assert!(scope.return_effects);
    }

    #[test]
    fn from_flags_omitted_flags_yield_the_dry_default() {
        let scope = AnalysisScope::from_flags(None, None, None, RenderQuality::default());
        assert_eq!(scope, AnalysisScope::default());
    }

    #[test]
    fn from_flags_quality_selects_the_rate() {
        let scope = AnalysisScope::from_flags(None, None, None, RenderQuality::Draft);
        assert_eq!(scope.render_sample_rate, RenderQuality::DRAFT_SAMPLE_RATE);
    }
}
