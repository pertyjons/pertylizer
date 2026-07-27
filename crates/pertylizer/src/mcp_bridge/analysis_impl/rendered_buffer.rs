//! Analysis of an already-rendered note buffer.

use super::*;

#[doc(hidden)]
/// Default RMS/centroid envelope block size (ms) for note analysis when the
/// caller doesn't request a specific resolution.
pub const DEFAULT_NOTE_ENVELOPE_WINDOW_MS: f32 = 50.0;

/// Resolve an optional `envelope_window_ms` request to a safe value: the
/// default when absent, else clamped to `[1, 5000]` ms so the envelope block
/// size stays at least one sample and never overflows the render.
pub(super) fn resolve_envelope_window_ms(requested: Option<f32>) -> f32 {
    match requested {
        Some(ms) => ms.clamp(1.0, 5000.0),
        None => DEFAULT_NOTE_ENVELOPE_WINDOW_MS,
    }
}

/// Analyze an already-rendered note with the default envelope window.
///
/// This pure analysis pass lets tests drive synthesized signals (anti-phase
/// tones, clipped tails, etc.) without spinning up the full audio engine. It
/// is also used by sweeps and the GUI, which don't expose the resolution knob.
pub fn analyze_rendered_buffer(
    rendered: &crate::audio::preview::RenderedNote,
    note: MidiNote,
    velocity: u8,
    duration_ms: u32,
    expected_note: Option<u8>,
) -> synth_mcp::types::AnalyzeNoteResult {
    analyze_rendered_buffer_with_window(
        rendered,
        note,
        velocity,
        duration_ms,
        expected_note,
        DEFAULT_NOTE_ENVELOPE_WINDOW_MS,
    )
}

/// Like [`analyze_rendered_buffer`] but with a caller-chosen RMS/centroid
/// envelope block size. Pass a small window (e.g. 2–5 ms) to resolve fast
/// attacks the default 50 ms window collapses into a single frame.
pub fn analyze_rendered_buffer_with_window(
    rendered: &crate::audio::preview::RenderedNote,
    note: MidiNote,
    velocity: u8,
    duration_ms: u32,
    expected_note: Option<u8>,
    envelope_window_ms: f32,
) -> synth_mcp::types::AnalyzeNoteResult {
    use crate::audio::analysis;
    use synth_core::types::StereoSample;

    // Sanitize non-finite samples up-front. If a voice or effect module
    // misbehaves and produces NaN/±∞, every downstream metric (peak/RMS/DC,
    // FFT, etc.) silently returns NaN, which the JSON serializer then
    // turns into `null`. Replacing non-finite samples with 0 here keeps the
    // metrics meaningful — `clipped_samples` still records the saturated
    // range, so a runaway DSP doesn't disappear from the report.
    let rendered_samples_owned: Vec<f32>;
    let samples_slice: &[f32] = if rendered.samples.iter().all(|s| s.is_finite()) {
        &rendered.samples
    } else {
        rendered_samples_owned = rendered
            .samples
            .iter()
            .map(|&s| if s.is_finite() { s } else { 0.0 })
            .collect();
        &rendered_samples_owned
    };

    // Mix stereo-interleaved buffer down to mono for time-domain metrics
    // (peak, RMS, DC). Per-channel and mid/side decompositions below capture
    // stereo-specific behavior. Spectral / pitch analysis uses a separate
    // `analysis_signal` (see Bug 5) so anti-phase tonal content does not
    // cancel in the mono mix.
    let channels = usize::from(rendered.channels);
    let mono: Vec<f32> = match channels {
        0 => Vec::new(),
        1 => samples_slice.to_vec(),
        2 => {
            let frames = samples_slice.len() / 2;
            StereoSample::iter_frames(samples_slice, frames)
                .map(StereoSample::to_mono)
                .collect()
        }
        n => samples_slice
            .chunks_exact(n)
            .map(|frame| frame.iter().sum::<f32>() / n as f32)
            .collect(),
    };
    let sample_rate = rendered.sample_rate;

    // Per-channel decomposition for stereo signals. We compute these from
    // the (sanitized) interleaved buffer so anti-phase content cannot cancel.
    let (left_samples, right_samples): (Vec<f32>, Vec<f32>) = if rendered.channels >= 2 {
        let frames = samples_slice.len() / channels;
        let mut l = Vec::with_capacity(frames);
        let mut r = Vec::with_capacity(frames);
        for frame in samples_slice.chunks_exact(channels) {
            l.push(frame[0]);
            r.push(frame[1]);
        }
        (l, r)
    } else {
        (Vec::new(), Vec::new())
    };
    let (mid_samples, side_samples): (Vec<f32>, Vec<f32>) = if rendered.channels >= 2 {
        let len = left_samples.len();
        let mut mid = Vec::with_capacity(len);
        let mut side = Vec::with_capacity(len);
        for i in 0..len {
            mid.push((left_samples[i] + right_samples[i]) * 0.5);
            side.push((left_samples[i] - right_samples[i]) * 0.5);
        }
        (mid, side)
    } else {
        (Vec::new(), Vec::new())
    };

    // Bug 5 fix: spectral / pitch / energy analysis runs on a "phase-robust"
    // signal that survives anti-phase stereo content. Per-sample max(|L|, |R|)
    // preserves energy regardless of channel polarity (a 180°-out tone has
    // m≈0 in the mono mix but max-abs equals the original amplitude on every
    // sample). For mono input the signal is identical to `mono`.
    let analysis_signal: Vec<f32> = if rendered.channels >= 2 {
        let frames = samples_slice.len() / channels;
        let mut sig = Vec::with_capacity(frames);
        for frame in samples_slice.chunks_exact(channels) {
            // Preserve mono sign (so DC / odd-symmetry detection stays sane)
            // by picking the channel with the larger magnitude.
            let l = frame[0];
            let r = frame[1];
            let v = if l.abs() >= r.abs() { l } else { r };
            sig.push(v);
        }
        sig
    } else {
        mono.clone()
    };

    // Sample windows for the spectrum snapshots. Attack: 50–150 ms in,
    // capturing the onset transient. Sustain: actual middle of the held
    // portion, NOT the last 100 ms (which we used to do incorrectly).
    // Release: starts 25 ms after note-off — that's where the decay tail
    // and any release-trigger transient sits, instead of "last 100 ms of
    // total render" (which often missed the note-off entirely).
    let total_samples = mono.len();
    let note_samples = (f64::from(duration_ms) / 1000.0 * f64::from(sample_rate)) as usize;
    let attack_start = (0.05 * f64::from(sample_rate)) as usize;
    let attack_window = (0.10 * f64::from(sample_rate)) as usize;
    let nominal_window = attack_window;
    // True midpoint of the held note, biased so the window stays inside the
    // hold even for very short notes. Sustain is allowed to slide back to fit
    // a full window (it stays inside the held portion either way).
    let sustain_center = note_samples / 2;
    let sustain_start_target = sustain_center.saturating_sub(nominal_window / 2);
    let sustain_max_start = total_samples.saturating_sub(nominal_window.min(total_samples));
    let sustain_start = sustain_start_target.min(sustain_max_start);
    let sustain_end = sustain_start
        .saturating_add(nominal_window)
        .min(total_samples);
    // Bug 4 fix: anchor release relative to the actual note-off frame from
    // the render (see RenderedNote::note_off_frame), with a 25 ms
    // post-note-off offset. Never let the window slide BACKWARD past
    // note_off+offset — that would pull sustain audio into the release slice
    // on short tails. Instead, allow a shorter slice (the analysis helpers
    // tolerate any length) and let the slice end at total_samples.
    let release_offset_samples = (0.025 * f64::from(sample_rate)) as usize;
    let note_off_sample = rendered.note_off_frame as usize;
    let release_start = note_off_sample
        .saturating_add(release_offset_samples)
        .min(total_samples);
    let release_end = release_start
        .saturating_add(nominal_window)
        .min(total_samples);

    let attack_clamped = attack_start.min(total_samples);
    let attack_end = attack_clamped
        .saturating_add(nominal_window)
        .min(total_samples);

    let attack_slice = analysis_signal
        .get(attack_clamped..attack_end)
        .unwrap_or(&[]);
    let sustain_slice = analysis_signal
        .get(sustain_start..sustain_end)
        .unwrap_or(&[]);
    let release_slice = analysis_signal
        .get(release_start..release_end)
        .unwrap_or(&[]);

    let sr_f32 = sample_rate as f32;
    let attack_window_start_ms = attack_clamped as f32 * 1000.0 / sr_f32;
    let sustain_window_start_ms = sustain_start as f32 * 1000.0 / sr_f32;
    let release_window_start_ms = release_start as f32 * 1000.0 / sr_f32;

    let to_peaks =
        |peaks: Vec<analysis::SpectrumPeak>| -> Vec<synth_mcp::types::AnalyzeSpectrumPeak> {
            peaks.into_iter().map(Into::into).collect()
        };

    // Anchor pitch metrics. When the caller provides `expected_note`, narrow
    // the fundamental search to ±tritone (1.4× either way) so the detector
    // ignores sub-octave dominance and harmonic clutter. Otherwise sweep the
    // full audible-bass range and report whatever peak is loudest.
    let anchor_note = expected_note.unwrap_or(rendered.effective_note.as_u8());
    let expected_fundamental = synth_core::types::Hertz::from_midi(anchor_note);
    let expected_fundamental_hz = expected_fundamental.as_f32();
    let (search_min, search_max) = match expected_note {
        Some(_) if expected_fundamental_hz > 0.0 => {
            let lo = expected_fundamental_hz / std::f32::consts::SQRT_2;
            let hi = expected_fundamental_hz * std::f32::consts::SQRT_2;
            (lo.max(20.0), hi.min(20_000.0))
        }
        _ => (25.0, 5500.0),
    };

    // Pitch analysis uses `analysis_signal` so anti-phase tonal content
    // does not cancel — see Bug 5 note above.
    let pitch_slice = analysis_signal
        .get(attack_start..note_samples.min(total_samples))
        .unwrap_or(&analysis_signal);
    let (fundamental_hz, pitch_confidence) = analysis::fundamental_frequency_with_confidence(
        pitch_slice,
        sample_rate,
        search_min,
        search_max,
    );
    let pitch_error_cents = if fundamental_hz > 0.0 && expected_fundamental_hz > 0.0 {
        expected_fundamental
            .cents_between(synth_core::types::Hertz::new(fundamental_hz))
            .as_f32()
    } else {
        0.0
    };

    // Per-channel fundamentals. For stereo input we re-run pitch detection
    // on the left and right channels independently using the SAME release/
    // sustain slice region (`attack_start..note_samples`) and the SAME
    // anchored search band as `fundamental_hz`. This lets the caller spot
    // wide-stereo patches where L and R carry different fundamentals — the
    // pooled `fundamental_hz` (computed on max(|L|,|R|)) reports a single
    // value that mixes both. For mono input both fields are `None` and the
    // analysis_signal_mode reflects that.
    let (
        analysis_signal_mode,
        fundamental_left,
        fundamental_right,
        fundamental_left_confidence,
        fundamental_right_confidence,
    ) = if rendered.channels >= 2 {
        let left_slice = left_samples
            .get(attack_start..note_samples.min(left_samples.len()))
            .unwrap_or(&left_samples);
        let right_slice = right_samples
            .get(attack_start..note_samples.min(right_samples.len()))
            .unwrap_or(&right_samples);
        let (f_l, c_l) = analysis::fundamental_frequency_with_confidence(
            left_slice,
            sample_rate,
            search_min,
            search_max,
        );
        let (f_r, c_r) = analysis::fundamental_frequency_with_confidence(
            right_slice,
            sample_rate,
            search_min,
            search_max,
        );
        (
            synth_mcp::types::AnalysisSignalMode::MaxAbsStereo,
            Some(f_l),
            Some(f_r),
            Some(c_l),
            Some(c_r),
        )
    } else {
        (
            synth_mcp::types::AnalysisSignalMode::Mono,
            None,
            None,
            None,
            None,
        )
    };

    let rms_envelope = analysis::rms_envelope(&mono, sample_rate, envelope_window_ms);
    // Centroid envelope tracks brightness motion; use the phase-robust
    // signal so anti-phase content still produces a meaningful spectrum.
    let mut centroid_envelope =
        analysis::centroid_envelope(&analysis_signal, sample_rate, envelope_window_ms);
    let rms_overall = analysis::rms_overall(&mono);

    // Trim only the centroid envelope tail. The raw `rms_envelope` is left
    // alone so `envelope_estimate` (which infers release length from RMS
    // decay) can see the full tail. Threshold = 5 % of overall RMS or 1e-4,
    // whichever is higher; never trim below 4 windows. Report how many were
    // trimmed so the agent can interpret a short centroid_envelope.
    let noise_floor = (rms_overall * 0.05).max(1e-4);
    let mut trimmed_tail_windows: u32 = 0;
    while centroid_envelope.len() > 4 {
        let last_idx = centroid_envelope.len() - 1;
        let last_rms = rms_envelope.get(last_idx).copied().unwrap_or(0.0);
        if last_rms < noise_floor {
            centroid_envelope.pop();
            trimmed_tail_windows += 1;
        } else {
            break;
        }
    }

    // Per-window pitch envelope. Uses a longer window than the rms/centroid
    // envelopes because FFT bin resolution scales with window length: a
    // 50 ms window at 44.1 kHz puts only 2-3 bins inside a one-tritone
    // search band at C2 (~65 Hz), so spectral leakage from neighboring
    // harmonics can flip the winning bin and produce false "drift". A
    // 200 ms window quadruples the resolution (~5 Hz/bin) and tracks bass
    // fundamentals stably. Same anchored search band as `fundamental_hz`.
    let pitch_envelope_window_ms: f32 = 200.0;
    let pitch_envelope = analysis::pitch_envelope(
        &analysis_signal,
        sample_rate,
        pitch_envelope_window_ms,
        search_min,
        search_max,
        1.0e-3,
    );

    // Stereo correlation needs the original interleaved buffer; only meaningful
    // when there are at least two channels, otherwise mono is "perfectly
    // correlated with itself" → 1.0.
    let stereo_correlation = if rendered.channels >= 2 {
        analysis::stereo_correlation(&rendered.samples)
    } else {
        1.0
    };

    let energy_bands: synth_mcp::types::AnalyzeEnergyBands =
        analysis::energy_bands(&analysis_signal, sample_rate).into();
    let harmonic_content: synth_mcp::types::AnalyzeHarmonicContent =
        analysis::harmonic_content(&analysis_signal, sample_rate, fundamental_hz).into();
    let envelope_estimate: synth_mcp::types::AnalyzeEnvelopeEstimate =
        analysis::envelope_estimate(&rms_envelope, envelope_window_ms, duration_ms).into();

    // Centroid trend over the held portion only. Slice `centroid_envelope` to
    // the note-on duration so the release tail doesn't bias the regression.
    let held_windows = ((f64::from(duration_ms) / f64::from(envelope_window_ms)).floor()) as usize;
    let centroid_trend_slice = if held_windows > 0 && held_windows <= centroid_envelope.len() {
        &centroid_envelope[..held_windows]
    } else {
        &centroid_envelope[..]
    };
    let centroid_trend_hz_per_sec =
        analysis::centroid_trend(centroid_trend_slice, envelope_window_ms);

    let peak_amplitude = analysis::peak_amplitude(&mono);
    let dc_offset = analysis::dc_offset(&mono);
    let clipped_samples = analysis::count_clipped(&mono, 0.999);

    // Per-channel and mid/side metrics (only when stereo).
    let (peak_left, peak_right, rms_left, rms_right, dc_left, dc_right, clipped_l, clipped_r) =
        if rendered.channels >= 2 {
            (
                Some(analysis::peak_amplitude(&left_samples)),
                Some(analysis::peak_amplitude(&right_samples)),
                Some(analysis::rms_overall(&left_samples)),
                Some(analysis::rms_overall(&right_samples)),
                Some(analysis::dc_offset(&left_samples)),
                Some(analysis::dc_offset(&right_samples)),
                Some(analysis::count_clipped(&left_samples, 0.999)),
                Some(analysis::count_clipped(&right_samples, 0.999)),
            )
        } else {
            (None, None, None, None, None, None, None, None)
        };
    // Bug 3 fix: stereo_width is a continuous 0..1 measure
    // `side_rms / (mid_rms + side_rms)`. 0 = mono (energy in mid only),
    // ~0.5 = typical stereo, 1 = anti-phase / fully decorrelated (energy
    // in side only). The earlier `s / m` form returned 0 for anti-phase
    // signals — semantically "mono", the OPPOSITE of what they are.
    let (mid_rms, side_rms, stereo_width) = if rendered.channels >= 2 {
        let m = analysis::rms_overall(&mid_samples);
        let s = analysis::rms_overall(&side_samples);
        let denom = m + s;
        let w = if denom > 1.0e-9 { s / denom } else { 0.0 };
        (Some(m), Some(s), Some(w))
    } else {
        (None, None, None)
    };

    // Stereo-aware flags: clipping if EITHER channel clipped, DC if EITHER
    // channel exceeds threshold, silent only if BOTH channels are silent.
    // Per-channel data takes precedence over the mono mix when present.
    let stereo_clipping = clipped_l.unwrap_or(0) > 0 || clipped_r.unwrap_or(0) > 0;
    let stereo_dc = dc_left.unwrap_or(0.0).abs() > 0.01 || dc_right.unwrap_or(0.0).abs() > 0.01;
    let stereo_silent = match (peak_left, peak_right) {
        (Some(l), Some(r)) => l < 0.005 && r < 0.005,
        _ => peak_amplitude < 0.005,
    };
    let stereo_low_output = match (peak_left, peak_right) {
        (Some(l), Some(r)) => l.max(r) < 0.05,
        _ => peak_amplitude < 0.05,
    };

    // Split the two pitch-quality flags on the shared confidence floor so
    // callers can distinguish "locked on the wrong note" (`off_pitch`) from
    // "couldn't lock at all" (`pitch_unreliable`).
    use crate::audio::analysis::PITCH_CONFIDENCE_RELIABLE_FLOOR;
    let off_pitch_real =
        pitch_error_cents.abs() > 50.0 && pitch_confidence >= PITCH_CONFIDENCE_RELIABLE_FLOOR;
    let pitch_unreliable = pitch_confidence < PITCH_CONFIDENCE_RELIABLE_FLOOR;

    let flags = synth_mcp::types::AnalyzeFlags {
        silent: stereo_silent,
        clipping: clipped_samples > 0 || stereo_clipping,
        has_dc_offset: dc_offset.abs() > 0.01 || stereo_dc,
        low_output: stereo_low_output,
        off_pitch: off_pitch_real,
        pitch_unreliable,
    };

    synth_mcp::types::AnalyzeNoteResult {
        note_requested: note.as_u8(),
        note_played: rendered.effective_note.as_u8(),
        velocity,
        sample_rate,
        duration_seconds: rendered.duration_seconds,
        fundamental_hz,
        analysis_signal_mode,
        fundamental_left,
        fundamental_right,
        fundamental_left_confidence,
        fundamental_right_confidence,
        expected_fundamental_hz,
        pitch_error_cents,
        peak_amplitude,
        rms_overall,
        dc_offset,
        clipped_samples,
        envelope_window_ms,
        rms_envelope,
        centroid_envelope,
        spectrum_attack: to_peaks(analysis::spectrum_top_peaks(attack_slice, sample_rate, 8)),
        spectrum_sustain: to_peaks(analysis::spectrum_top_peaks(sustain_slice, sample_rate, 8)),
        spectrum_release: to_peaks(analysis::spectrum_top_peaks(release_slice, sample_rate, 8)),
        pitch_envelope,
        pitch_envelope_window_ms,
        stereo_correlation,
        energy_bands,
        harmonic_content,
        envelope_estimate,
        centroid_trend_hz_per_sec,
        flags,
        peak_left,
        peak_right,
        rms_left,
        rms_right,
        dc_left,
        dc_right,
        clipped_left: clipped_l,
        clipped_right: clipped_r,
        mid_rms,
        side_rms,
        stereo_width,
        pitch_confidence: Some(pitch_confidence),
        trimmed_tail_windows: if trimmed_tail_windows > 0 {
            Some(trimmed_tail_windows)
        } else {
            None
        },
        attack_window_start_ms: Some(attack_window_start_ms),
        sustain_window_start_ms: Some(sustain_window_start_ms),
        release_window_start_ms: Some(release_window_start_ms),
        warnings: rendered.warnings.clone(),
        // Populated by the `analyze_note` bridge method, which has session
        // access; the sweep tools reuse this buffer-analysis path and don't
        // pay the per-step description lookup.
        module_descriptions: Vec::new(),
    }
}

impl From<crate::audio::analysis::SpectrumPeak> for synth_mcp::types::AnalyzeSpectrumPeak {
    fn from(p: crate::audio::analysis::SpectrumPeak) -> Self {
        Self {
            freq_hz: p.freq_hz,
            magnitude_db: p.magnitude_db,
        }
    }
}

impl From<crate::audio::analysis::EnergyBands> for synth_mcp::types::AnalyzeEnergyBands {
    fn from(b: crate::audio::analysis::EnergyBands) -> Self {
        Self {
            sub: b.sub,
            low: b.low,
            mid: b.mid,
            high: b.high,
        }
    }
}

impl From<crate::audio::analysis::HarmonicContent> for synth_mcp::types::AnalyzeHarmonicContent {
    fn from(h: crate::audio::analysis::HarmonicContent) -> Self {
        Self {
            thd_db: h.thd_db,
            odd_even_ratio_db: h.odd_even_ratio_db,
            n_harmonics: h.n_harmonics,
        }
    }
}

impl From<crate::audio::analysis::EnvelopeEstimate> for synth_mcp::types::AnalyzeEnvelopeEstimate {
    fn from(e: crate::audio::analysis::EnvelopeEstimate) -> Self {
        Self {
            attack_ms: e.attack_ms,
            decay_ms: e.decay_ms,
            sustain_level: e.sustain_level,
            release_ms: e.release_ms,
        }
    }
}
