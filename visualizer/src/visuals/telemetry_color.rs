//! Shared helpers for mapping telemetry data to visual parameters.
//!
//! Effects call these instead of hardcoding telemetry-to-visual mappings,
//! ensuring all color/intensity decisions respect the active theme.

use super::theme::ThemeMaterialPolicy;

/// Approximate centroid range for normalization (Hz).
const CENTROID_MIN: f32 = 200.0;
const CENTROID_MAX: f32 = 8000.0;

/// Map spectral centroid (Hz) to a hue using the active theme's range.
///
/// Low centroid (bass-heavy) maps to `centroid_hue_low`,
/// high centroid (bright) maps to `centroid_hue_high`.
/// Returns a hue offset in degrees (0-360).
#[must_use]
pub fn centroid_to_hue(centroid_hz: f32, policy: &ThemeMaterialPolicy) -> f32 {
    let t = ((centroid_hz - CENTROID_MIN) / (CENTROID_MAX - CENTROID_MIN)).clamp(0.0, 1.0);
    policy.centroid_hue_low + (policy.centroid_hue_high - policy.centroid_hue_low) * t
}

/// Compute a flux-driven emissive boost factor.
///
/// Returns a multiplier >= 0.0 that can be added to the base emissive.
/// At flux=0 this returns 0.0; at high flux it returns up to `flux_intensity_scale`.
#[must_use]
pub fn flux_emissive_boost(flux: f32, policy: &ThemeMaterialPolicy) -> f32 {
    // Flux is typically 0.0-1.0+ (can exceed 1.0 on sharp transients)
    let normalized = flux.clamp(0.0, 2.0);
    normalized * policy.flux_intensity_scale
}

/// Compute a beat-phase pulse factor (0.0-1.0).
///
/// Peaks at the downbeat (phase=0) and decays toward phase=1.
/// The strength is scaled by the theme's `beat_pulse_strength`.
#[must_use]
pub fn beat_pulse_factor(beat_phase: f32, policy: &ThemeMaterialPolicy) -> f32 {
    // Cosine pulse: 1.0 at phase=0, 0.0 at phase=0.5
    let raw = (1.0 - beat_phase).max(0.0);
    let pulse = raw * raw; // Quadratic falloff for snappier feel
    pulse * policy.beat_pulse_strength
}

/// Map RMS level to an emissive scale factor.
///
/// Returns a multiplier around 1.0, ranging from ~0.5 (silence) to `rms_emissive_scale` (loud).
#[cfg(test)]
#[must_use]
fn rms_to_emissive(rms_mono: f32, policy: &ThemeMaterialPolicy) -> f32 {
    // RMS is linear amplitude 0.0-1.0 typically
    let t = rms_mono.clamp(0.0, 1.0);
    // Lerp from base (0.5) to theme's rms_emissive_scale
    0.5 + t * (policy.rms_emissive_scale - 0.5)
}

/// Returns true if peak level exceeds threshold, suitable for triggering flashes.
#[must_use]
pub fn peak_exceeds_threshold(peak_l: f32, peak_r: f32, threshold: f32) -> bool {
    let peak_mono = (peak_l + peak_r) * 0.5;
    peak_mono > threshold
}

/// Hue offset for an instrument category, used by note-driven effects
/// to give each instrument type a distinct visual color layer.
#[must_use]
pub fn category_hue_offset(category: synth_osc_protocol::InstrumentCategory) -> f32 {
    use synth_osc_protocol::InstrumentCategory;
    match category {
        InstrumentCategory::Drums => 0.0,         // red range
        InstrumentCategory::Bass => 230.0,        // deep blue
        InstrumentCategory::Pad => 280.0,         // purple
        InstrumentCategory::Lead => 50.0,         // golden yellow
        InstrumentCategory::Arp => 180.0,         // cyan
        InstrumentCategory::Keys => 130.0,        // green
        InstrumentCategory::FX => 30.0,           // orange
        InstrumentCategory::Uncategorized => 0.0, // no offset
    }
}

/// Map a normalized band position (0.0 = lowest freq, 1.0 = highest) to a
/// perceptually meaningful hue. Uses a nonlinear curve that gives bass
/// frequencies more hue space (red→orange→yellow) since human hearing is
/// logarithmic and low frequencies span more perceptual territory.
///
/// Mapping: bass (0.0) → 0° red, mids (0.5) → 120° green, highs (1.0) → 270° violet.
#[must_use]
pub fn band_frequency_hue(band_position: f32) -> f32 {
    let t = band_position.clamp(0.0, 1.0);
    // Nonlinear: sqrt gives bass more hue space, compress treble
    let curved = t.sqrt();
    curved * 270.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neon_policy() -> ThemeMaterialPolicy {
        ThemeMaterialPolicy::default() // Neon defaults
    }

    #[test]
    fn test_centroid_to_hue_low() {
        let policy = neon_policy();
        let hue = centroid_to_hue(CENTROID_MIN, &policy);
        assert!(
            (hue - 270.0).abs() < 0.01,
            "Low centroid should map to hue_low"
        );
    }

    #[test]
    fn test_centroid_to_hue_high() {
        let policy = neon_policy();
        let hue = centroid_to_hue(CENTROID_MAX, &policy);
        assert!(
            (hue - 360.0).abs() < 0.01,
            "High centroid should map to hue_high"
        );
    }

    #[test]
    fn test_centroid_to_hue_mid() {
        let policy = neon_policy();
        let hue = centroid_to_hue((CENTROID_MIN + CENTROID_MAX) / 2.0, &policy);
        let expected = (270.0 + 360.0) / 2.0;
        assert!(
            (hue - expected).abs() < 0.01,
            "Mid centroid should map to midpoint"
        );
    }

    #[test]
    fn test_flux_boost_zero() {
        let policy = neon_policy();
        assert!((flux_emissive_boost(0.0, &policy)).abs() < 0.001);
    }

    #[test]
    fn test_flux_boost_scales_with_theme() {
        let mut policy = neon_policy();
        let boost_default = flux_emissive_boost(1.0, &policy);
        policy.flux_intensity_scale = 3.0;
        let boost_high = flux_emissive_boost(1.0, &policy);
        assert!(
            boost_high > boost_default,
            "Higher scale should give bigger boost"
        );
    }

    #[test]
    fn test_beat_pulse_at_downbeat() {
        let policy = neon_policy();
        let pulse = beat_pulse_factor(0.0, &policy);
        assert!(pulse > 0.5, "Pulse should be strong at downbeat");
    }

    #[test]
    fn test_beat_pulse_at_offbeat() {
        let policy = neon_policy();
        let pulse = beat_pulse_factor(1.0, &policy);
        assert!(pulse < 0.01, "Pulse should be near zero at end of beat");
    }

    #[test]
    fn test_rms_to_emissive_silent() {
        let policy = neon_policy();
        let e = rms_to_emissive(0.0, &policy);
        assert!((e - 0.5).abs() < 0.01, "Silent should give base emissive");
    }

    #[test]
    fn test_rms_to_emissive_loud() {
        let policy = neon_policy();
        let e = rms_to_emissive(1.0, &policy);
        assert!(
            (e - policy.rms_emissive_scale).abs() < 0.01,
            "Full RMS should give max scale"
        );
    }

    #[test]
    fn test_peak_threshold() {
        assert!(peak_exceeds_threshold(0.9, 0.8, 0.7));
        assert!(!peak_exceeds_threshold(0.3, 0.2, 0.7));
    }

    #[test]
    fn test_category_hue_offset_drums_vs_bass() {
        use synth_osc_protocol::InstrumentCategory;
        let drums = category_hue_offset(InstrumentCategory::Drums);
        let bass = category_hue_offset(InstrumentCategory::Bass);
        assert!(
            (drums - bass).abs() > 100.0,
            "Different categories should have distinct hues"
        );
    }

    #[test]
    fn test_band_frequency_hue_range() {
        let low = band_frequency_hue(0.0);
        let high = band_frequency_hue(1.0);
        assert!(low < 1.0, "Bass should map near 0° (red)");
        assert!(
            (high - 270.0).abs() < 0.01,
            "Treble should map near 270° (violet)"
        );
    }

    #[test]
    fn test_band_frequency_hue_nonlinear() {
        let mid_linear = band_frequency_hue(0.5);
        // With sqrt curve, mid should be > 135 (the linear midpoint)
        // sqrt(0.5) ≈ 0.707, so hue ≈ 191°
        assert!(
            mid_linear > 135.0,
            "Nonlinear curve should give bass more hue space"
        );
    }
}
