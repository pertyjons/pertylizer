//! analysis MCP tool handlers.

use super::super::*;

#[tool_router(router = analysis_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "Render an audio preview of a note played on an instrument. Returns a WAV audio clip of the instrument's current sound. Useful for hearing what a patch sounds like after making changes.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn preview_note(
        &self,
        params: Parameters<PreviewNoteParam>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_midi_note(params.0.note).map_err(mcp_err)?;
        validate_velocity(params.0.velocity).map_err(mcp_err)?;
        let duration_ms = params.0.duration_ms.unwrap_or(500);
        let tail_ms = params.0.tail_ms.unwrap_or(500);
        #[expect(clippy::cast_precision_loss, reason = "millisecond values fit in f32")]
        validate_range("duration_ms", duration_ms as f32, 1.0, 30000.0).map_err(mcp_err)?;
        #[expect(clippy::cast_precision_loss, reason = "millisecond values fit in f32")]
        validate_range("tail_ms", tail_ms as f32, 1.0, 30000.0).map_err(mcp_err)?;

        let preview = self
            .bridge
            .render_note_preview(
                params.0.instrument_id,
                params.0.note,
                params.0.velocity,
                duration_ms,
                tail_ms,
            )
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        use base64::Engine;
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        let encoded = {
            static ENGINE: std::sync::OnceLock<base64::engine::Simd> = std::sync::OnceLock::new();
            ENGINE
                .get_or_init(|| {
                    base64::engine::Simd::standard(base64::engine::general_purpose::PAD)
                })
                .encode(&preview.wav_data)
        };
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let encoded = base64::engine::general_purpose::STANDARD.encode(&preview.wav_data);

        let audio = ContentBlock::audio(encoded, "audio/wav");

        let text = ContentBlock::text(format!(
            "Audio preview: note {} vel {} on instrument {} ({:.1}s, {}Hz WAV, {} bytes)",
            params.0.note,
            params.0.velocity,
            params.0.instrument_id,
            preview.duration_seconds,
            preview.sample_rate,
            preview.wav_data.len(),
        ));

        Ok(CallToolResult::success(vec![text, audio]))
    }

    #[tool(
        description = "Render a note offline and return quantitative analysis of the audio: detected fundamental, peak/RMS, DC offset, clip count, RMS and centroid envelopes over time, and top spectral peaks at attack/sustain/release. Use this instead of `preview_note` when you want metrics rather than audio bytes — far cheaper to inspect than a WAV roundtrip and gives consistent measurements across calls.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_note(
        &self,
        params: Parameters<AnalyzeNoteParam>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_midi_note(params.0.note).map_err(mcp_err)?;
        validate_velocity(params.0.velocity).map_err(mcp_err)?;
        let duration_ms = params.0.duration_ms.unwrap_or(500);
        let tail_ms = params.0.tail_ms.unwrap_or(500);
        #[expect(clippy::cast_precision_loss, reason = "millisecond values fit in f32")]
        validate_range("duration_ms", duration_ms as f32, 1.0, 30000.0).map_err(mcp_err)?;
        #[expect(clippy::cast_precision_loss, reason = "millisecond values fit in f32")]
        validate_range("tail_ms", tail_ms as f32, 1.0, 30000.0).map_err(mcp_err)?;

        if let Some(expected) = params.0.expected_note {
            validate_midi_note(MidiNote::new(expected)).map_err(mcp_err)?;
        }
        if let Some(window) = params.0.envelope_window_ms {
            validate_range("envelope_window_ms", window, 1.0, 5000.0).map_err(mcp_err)?;
        }

        let result = tokio::task::block_in_place(|| {
            self.bridge.analyze_note(
                params.0.instrument_id,
                params.0.note,
                params.0.velocity,
                duration_ms,
                tail_ms,
                params.0.expected_note,
                params.0.envelope_window_ms,
            )
        })
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Render N seconds of the master bus offline and return mix-level metrics: integrated/short-term-max/momentary-max LUFS (ITU-R BS.1770-4), sample peak in dBFS, true peak in dBTP (4× oversampled per BS.1770-4 Annex 2 — catches inter-sample overshoots that emerge after DA conversion), RMS in dBFS, crest factor, 4-band frequency-balance RMS energies (sub/low/mid/high), stereo correlation, mid/side RMS, stereo width, mono-compatibility score (0..1 — how well L+R survive a mono sum), and a clipped-sample count. Use this to judge whether a mix is balanced, too quiet/loud, narrow, anti-phase, or clipping (sample or inter-sample). LUFS-S requires ≥ 3 s of audio; shorter renders report -200.0 for that field. Renders the song from `start_tick` (default 0) for `duration_seconds` (default 10, max 300) using the engine snapshot — deterministic and offline. Pass `include_per_track: true` to also receive a per-track breakdown (one soloed render per audible track) so you can tell which track is responsible for clipping, dominant energy, or sub-bass — costs roughly O(track_count) extra render time. This is the same breakdown as analyze_section's, but keyed off a duration window rather than an explicit tick range.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_mix_bus(&self, params: Parameters<AnalyzeMixBusParam>) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_mix_bus(
                duration,
                params.0.start_tick,
                params.0.include_per_track,
                scope,
            )
        })
    }

    #[tool(
        description = "Render the arrangement offline to a 32-bit float stereo WAV and return path plus stats. At the requested range end the transport stops, then tail_seconds (default 1) captures voice/effect releases without triggering later arrangement events. Pass instrument_id to isolate one instrument against a cloned song. Deterministic and offline.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn render_to_wav(&self, params: Parameters<RenderToWavParam>) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        let tail = params.0.tail_seconds.unwrap_or(1.0);
        if let Err(e) = validate_range("tail_seconds", tail, 0.0, 30.0) {
            return validation_err(e);
        }
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.render_to_wav(
                params.0.path.clone(),
                duration,
                params.0.start_tick,
                params.0.instrument_id,
                scope,
                synth_core::Seconds::new(tail),
            )
        })
    }

    #[tool(
        description = "Detailed spectrum of an offline render: detected partials (frequency + amplitude + harmonic number + cents deviation), a voiced/unvoiced verdict, and timbre descriptors — spectral centroid (brightness), flatness (0 pure tone … 1 noise), rolloff, aggregate inharmonicity, and odd/even harmonic ratio. These separate timbres the 4-band analyze_mix_bus energy metric cannot: a plain triangle, a ring-modulated triangle, and a metallic carrier have near-identical 4-band energy but very different partial structure. Pass instrument_id to fingerprint one instrument in isolation (clone-based; your project is untouched); f0_hint to sharpen harmonic tagging; log_bins > 0 to add log-spaced magnitude bins for compare_spectra. The f0 detector (McLeod/NSDF) reports unvoiced (f0 null) for noise so noise frames don't emit a garbage fundamental. Renders from start_tick (default 0) for duration_seconds (default 10, max 300), deterministic and offline; use render_quality 'full' (default) for spectral work.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_spectrum(
        &self,
        params: Parameters<AnalyzeSpectrumParam>,
    ) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_spectrum(
                duration,
                params.0.start_tick,
                params.0.instrument_id,
                params.0.f0_hint,
                params.0.max_partials,
                params.0.log_bins,
                scope,
            )
        })
    }

    #[tool(
        description = "Spectrogram of an offline render: the requested window is rendered ONCE and a sliding FFT returns one full spectrum (partials + voiced verdict + descriptors, same as analyze_spectrum) per hop_ms, analysing window_len_ms per frame. Use this when a sound's identity is its time evolution — e.g. a Commodore-64 SID voice whose spectrum switches every ~20 ms (pitched triangle frame vs chip-noise frame): the per-frame `voiced` flag reads that alternation directly. Far cheaper than calling analyze_spectrum many times — it is one render and O(1) MCP calls, not N. hop_ms defaults to 20 (≈ one PAL video frame), window_len_ms to 40; frames are capped at 4096. Renders from start_tick (default 0) for duration_seconds (default 10, max 300), deterministic and offline.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_spectrogram(
        &self,
        params: Parameters<AnalyzeSpectrogramParam>,
    ) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_spectrogram(
                duration,
                params.0.start_tick,
                params.0.instrument_id,
                params.0.f0_hint,
                params.0.max_partials,
                params.0.log_bins,
                params.0.hop_ms,
                params.0.window_len_ms,
                scope,
            )
        })
    }

    #[tool(
        description = "Run the same detailed spectral analysis as analyze_spectrum, but over an imported sample or a WAV file on disk instead of a render — detected partials, voiced/unvoiced verdict, and timbre descriptors (centroid, flatness, rolloff, inharmonicity, odd/even ratio). Use this to fingerprint a real reference recording (e.g. a SID render written by sidplayfp, or any WAV) in exactly the same units as analyze_spectrum, then feed both into compare_spectra to drive a timbre-matching loop. sample_id_or_path is either a numeric imported-sample id or a path to a WAV file; the audio is analyzed at its native sample rate and downmixed to mono. Pass log_bins > 0 to enable the broadband distance in compare_spectra.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_sample_spectrum(
        &self,
        params: Parameters<AnalyzeSampleSpectrumParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_sample_spectrum(
                params.0.sample_id_or_path.clone(),
                params.0.f0_hint,
                params.0.max_partials,
                params.0.log_bins,
                params.0.start_ms,
                params.0.window_len_ms,
            )
        })
    }

    #[tool(
        description = "Per-frame spectrogram of an imported sample or WAV file — the sample counterpart of analyze_spectrogram. Slides an FFT across the decoded audio at its NATIVE sample rate and returns one spectrum per hop (time_seconds + the same descriptor analyze_spectrum gives: partials, voiced verdict, centroid/flatness/rolloff/inharmonicity). Use it to see the time evolution of a real reference recording — e.g. a SID render alternating pitched/noise every ~20 ms — which a single aggregate analyze_sample_spectrum hides. Frames line up with analyze_spectrogram of the equivalent render so you can compare per-frame. sample_id_or_path is a numeric imported-sample id or a path to a WAV. hop_ms defaults to 20 (≈ one PAL frame), window_len_ms to 40; frame count is capped at 4096 (a warning is added on truncation). Deterministic and offline.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_sample_spectrogram(
        &self,
        params: Parameters<AnalyzeSampleSpectrogramParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_sample_spectrogram(
                params.0.sample_id_or_path.clone(),
                params.0.f0_hint,
                params.0.max_partials,
                params.0.log_bins,
                params.0.hop_ms,
                params.0.window_len_ms,
            )
        })
    }

    #[tool(
        description = "Compare two rendered/sample spectra and report broadband distances, descriptor deltas, and missing/extra partials. Time-resolved comparison is enabled by default: frames are envelope-aligned and only target-energy frames are scored, so sparse/staccato references rank correctly instead of averaging over silence. Set time_resolved=false for aggregate-only output. Deterministic and offline.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn compare_spectra(&self, params: Parameters<CompareSpectraParam>) -> String {
        let p = params.0;
        let scope = crate::bridge::AnalysisScope::from_flags(
            p.include_all,
            p.include_master_effects,
            p.include_return_effects,
            crate::bridge::RenderQuality::parse(p.render_quality.as_deref()),
        );
        let to_source = |s: &SpectrumSourceParam| crate::bridge::SpectrumSource {
            sample_id_or_path: s.sample_id_or_path.clone(),
            instrument_id: s.instrument_id,
            start_tick: s.start_tick,
            duration_seconds: s.duration_seconds,
            start_ms: s.start_ms,
            window_len_ms: s.window_len_ms,
        };
        let target = to_source(&p.target);
        let candidate = to_source(&p.candidate);
        // Mask/align default to on; only the explicit "none" string turns them off.
        let time_resolved = crate::bridge::TimeResolvedOptions {
            enabled: p.time_resolved.unwrap_or(true),
            hop_ms: p.hop_ms,
            frame_len_ms: p.frame_len_ms,
            mask_target_energy: p
                .mask
                .as_deref()
                .is_none_or(|m| !m.trim().eq_ignore_ascii_case("none")),
            align_envelope: p
                .align
                .as_deref()
                .is_none_or(|a| !a.trim().eq_ignore_ascii_case("none")),
            align_max_ms: p.align_max_ms,
        };
        run_blocking_json(move || {
            self.bridge.compare_spectra(
                target,
                candidate,
                p.f0_hint,
                p.max_partials,
                p.log_bins,
                p.mel_bands,
                scope,
                time_resolved,
            )
        })
    }

    #[tool(
        description = "Compare the amplitude CONTOURS (ADSR shape over time) of two sources — the time-domain counterpart of compare_spectra. FFT-based tools miss how a sound evolves; a SID voice's identity is largely its envelope (attack punch, decay, sustain, hard-restart click). Each side (target, candidate) is a render (optionally soloing one instrument) or an imported sample / WAV. Extracts an RMS envelope from each, peak-normalises them (shape is compared independent of loudness — use analyze_mix_bus for level), and aligns them with dynamic time warping. Returns: dtw_distance (the scalar to minimise — normalised warp distance between the contours, tolerant of small timing differences); a per-side breakdown (attack_ms, decay_ms, sustain_level, release_ms, plus attack-transient crest_factor_db and energy_rise_db — the 'punch' of the onset); and the candidate − target deltas for each. Use it to check your patch's envelope tracks a reference: watch dtw_distance fall as you tune ADSR, and read crest_factor_delta_db to see if you're missing the reference's attack punch. release_ms needs note_duration_ms; omit it and the shape distance still works. Deterministic and offline.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn compare_envelopes(
        &self,
        params: Parameters<CompareEnvelopesParam>,
    ) -> String {
        let p = params.0;
        let scope = crate::bridge::AnalysisScope::from_flags(
            p.include_all,
            p.include_master_effects,
            p.include_return_effects,
            crate::bridge::RenderQuality::parse(p.render_quality.as_deref()),
        );
        let to_source = |s: &SpectrumSourceParam| crate::bridge::SpectrumSource {
            sample_id_or_path: s.sample_id_or_path.clone(),
            instrument_id: s.instrument_id,
            start_tick: s.start_tick,
            duration_seconds: s.duration_seconds,
            start_ms: s.start_ms,
            window_len_ms: s.window_len_ms,
        };
        let target = to_source(&p.target);
        let candidate = to_source(&p.candidate);
        run_blocking_json(move || {
            self.bridge.compare_envelopes(
                target,
                candidate,
                p.envelope_window_ms,
                p.note_duration_ms,
                p.transient_window_ms,
                scope,
            )
        })
    }

    #[tool(
        description = "Incremental per-effect breakdown of the master bus. Renders the chain input (post-return mix, before any master effect) once, then re-renders the master output with the chain truncated after each effect — so you can see exactly what each master effect does to the mix. Each stage reports the full mix metrics at that point plus the delta the effect introduced: lufs_delta, peak/true-peak/rms delta in dB, stereo_width_delta, crest_delta_db (negative = more compressed dynamics), and gain_reduction_db (positive = the effect attenuated level, e.g. a limiter). Use this to verify a master limiter is catching peaks, an EQ is shaping balance, or to find the effect that is crushing your dynamics or narrowing the image. The master effect chain is always reconstructed; pass `include_return_effects: true` to feed the return wet signal into the chain input. Costs one offline render per master effect plus one for the input — O(effect_count). Renders from `start_tick` (default 0) for `duration_seconds` (default 10, max 300), deterministic and offline.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_master_chain(
        &self,
        params: Parameters<AnalyzeMasterChainParam>,
    ) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        // The master chain is always measured; only the surrounding stages are
        // optional. `from_flags` with master_effects=Some(true) forces it on.
        let scope = crate::bridge::AnalysisScope::from_flags(
            None,
            Some(true),
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge
                .analyze_master_chain(duration, params.0.start_tick, scope)
        })
    }

    #[tool(
        description = "Per-return-bus contribution to the master mix. Renders the full mix once, then re-renders with each return bus muted in turn (against a clone — your project is untouched), and reports how much each return adds: lufs_delta, peak/true-peak/rms delta in dB, and stereo_width_delta (all full − muted, so positive = the return makes the mix louder/wider/peakier). Use this to see which send effect (reverb, delay, …) is eating your headroom, widening the image, or contributing the most loudness. Because a return's wet signal cannot be cleanly soloed away from the dry track sum, the muted-difference is the honest contribution measure; returns sum in parallel, so each delta is that bus's marginal contribution. The return-bus effect chains are always reconstructed; pass `include_master_effects: true` to measure through the processed master output. Costs one offline render for the full mix plus one per return bus — O(return_count). Renders from `start_tick` (default 0) for `duration_seconds` (default 10, max 300), deterministic and offline.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_return_busses(
        &self,
        params: Parameters<AnalyzeReturnBussesParam>,
    ) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        // Return-bus chains are always measured; only the surrounding stages are
        // optional. `from_flags` with return_effects=Some(true) forces them on.
        let scope = crate::bridge::AnalysisScope::from_flags(
            None,
            params.0.include_master_effects,
            Some(true),
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge
                .analyze_return_busses(duration, params.0.start_tick, scope)
        })
    }

    #[tool(
        description = "Measure the master mix and set the master fader to reach a target loudness without breaching a true-peak ceiling. \
                       Renders the song (default 10 s) through the master + return effect chains at 44.1 kHz, measures integrated LUFS \
                       and true peak, then adjusts the master volume. The fader is post-effects, so loudness and peak scale linearly — \
                       no iteration. Returns measured vs. predicted LUFS/true-peak, the applied gain, old/new master volume, and \
                       `limited_by` (whether the target, the true-peak ceiling, or the fader range bound the result). Mutates master volume.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn auto_gain_stage(&self, params: Parameters<AutoGainStageParam>) -> String {
        let p = params.0;
        if let Err(e) = validate_range("target_lufs", p.target_lufs, -60.0, 0.0) {
            return validation_err(e);
        }
        let ceiling = p.true_peak_ceiling.unwrap_or(-1.0);
        if let Err(e) = validate_range("true_peak_ceiling", ceiling, -24.0, 0.0) {
            return validation_err(e);
        }
        let duration = p.duration_seconds.unwrap_or(10.0);
        run_blocking_json(|| {
            self.bridge
                .auto_gain_stage(p.target_lufs, ceiling, duration, p.start_tick)
        })
    }

    #[tool(
        description = "Render an explicit arrangement range [start_tick, end_tick) offline and return the same mix-bus metrics as analyze_mix_bus (LUFS-I/S/M, sample peak, true peak in dBTP, RMS, crest, banded energy, stereo correlation, mid/side, mono-compatibility, clipped samples). Use this when you want to A/B verses vs. choruses, compare a buildup to a drop, or inspect a specific musical passage rather than a fixed-duration window from the song start. Pass `include_per_track: true` to also receive a per-track breakdown (one soloed render per audible track) so you can tell which track is responsible for clipping, dominant energy, or sub-bass — costs roughly O(track_count) extra render time. Per-track `metrics.peak`/`metrics.rms` include pan-law attenuation (-3 dB at center pan: a center-panned source with internal peak 1.0 reports ~0.7071). Per-track `pre_master_peak` analytically reverses the instrument's pan + volume attenuation from the per-channel peaks and reports the patch's internal signal peak directly, so you can see internal clipping that would otherwise be hidden by a quiet pan-down.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_section(&self, params: Parameters<AnalyzeSectionParam>) -> String {
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_section(
                params.0.start_tick,
                params.0.end_tick,
                params.0.include_per_track,
                scope,
            )
        })
    }

    #[tool(
        description = "Pairwise spectral-masking report across every audible track in an arrangement range. Renders each audible track soloed once, then for every pair (a, b) compares their per-band RMS in the 4-band split (sub 0-100 Hz, low 100-500 Hz, mid 500-2000 Hz, high 2 kHz+) used elsewhere. Each pair carries the per-band overlap energy, the dominance margin in dB, an overall conflict_score in 0..=1, the dominant track id when one side leads by >6 dB on the worst-overlap band, and a textual hint such as 'Pad(2) masks Lead(3) in mid (500-2000 Hz)'. Pairs are returned sorted by descending conflict_score so the most contested combination appears first. Tracks whose soloed render sits below the -60 dBFS audibility floor in the window (i.e. effectively silent — a part that does not play in this section) are excluded from the matrix and listed under `tracks_below_floor` instead; this stops two equally-silent tracks from being ranked as a spurious 1.0 conflict. A single offline render is capped at 300 seconds, so on a longer requested range the analyzed window is clamped: the `start_bar`/`end_bar`/`start_tick`/`end_tick` fields report the range *actually* analyzed, while `requested_end_tick`/`requested_end_bar` report the full request — compare them (or read the warning) to detect partial coverage. Renders are O(track_count) (same as analyze_section with include_per_track=true); the pair matrix itself is in-memory and O(N²). Use when a section sounds muddy or when one element is being smothered and you need to know which other track is doing it.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_masking_matrix(
        &self,
        params: Parameters<AnalyzeMaskingMatrixParam>,
    ) -> String {
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_masking_matrix(
                params.0.arrangement_start_tick,
                params.0.arrangement_end_tick,
                params.0.top_pairs,
                scope,
            )
        })
    }

    #[tool(
        description = "Symbolic harmonic analysis of a pattern or arrangement range. Walks notes in time order, groups simultaneous notes into chord events, identifies chord symbols (e.g. Cm7, F7sus4), infers the most likely key via Krumhansl-Schmuckler correlation, and reports an in-key ratio, out-of-scale pitch classes, and a composite harmonic stability score. Pure symbolic — no audio rendering. Use to verify chord progressions, spot accidentally out-of-key notes, and reason about the harmonic shape of generated music. Pass `pattern_id` for one pattern, or omit it (with optional `arrangement_start_tick` / `arrangement_end_tick`) for an arrangement range.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_harmony(&self, params: Parameters<AnalyzeHarmonyParam>) -> String {
        match self.bridge.analyze_harmony(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.grouping_ticks,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Symbolic structural analysis of a single pattern. Reports density (notes per bar/beat, active ratio), pitch shape (range, mean, distinct count, duration-weighted pitch-class histogram), velocity dynamics (min/max/mean/std/range), rhythm (max/mean polyphony, distinct onsets/durations, inter-onset-interval mean+std, regularity score), and bar-level repetition (distinct bar signatures, repetition score). Pure symbolic — no audio rendering. Use to verify whether a pattern is interesting (varied vs. flat, dense vs. sparse, repetitive vs. through-composed) without listening, and as a prerequisite for variation generation heuristics.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_pattern(&self, params: Parameters<AnalyzePatternParam>) -> String {
        match self.bridge.analyze_pattern(params.0.pattern_id) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Sweep an instrument across a MIDI note range and run the same render-and-analyze pipeline as analyze_note at each step. Returns per-note metrics (fundamental, pitch error, pitch confidence, peak/RMS, centroid, clipped-sample count) plus cross-step issues (silent notes, likely-aliased notes — high centroid + low pitch confidence in the top octaves, lost pitch tracking — fundamental off by more than an octave, clipping notes, level spread in dB between loudest and quietest non-silent step). Use to catch patches that work at C4 in analyze_note and fall apart at C6 (aliasing) or C2 (energy loss). One render per step — `step_semitones` defaults to 12 (one note per octave); reduce for higher resolution, increase or limit `[low_note, high_note]` for cheaper sweeps.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_instrument_range(
        &self,
        params: Parameters<AnalyzeInstrumentRangeParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_instrument_range(
                params.0.instrument_id,
                params.0.low_note,
                params.0.high_note,
                params.0.step_semitones,
                params.0.velocity,
                params.0.duration_ms,
                params.0.tail_ms,
            )
        })
    }

    #[tool(
        description = "Hold one MIDI note and sweep velocity across [velocity_low, velocity_high]. Returns per-velocity amplitude/brightness curves plus monotonicity flags (non_monotonic_amplitude_steps — adjacent pairs where peak fell as velocity rose, non_monotonic_centroid_steps — same for brightness) and a velocity_unresponsive flag (amplitude_range_db < 3 dB across the sweep). Use to confirm a patch actually responds to velocity in a musical way (rising amplitude, brighter filter at higher velocity) instead of being effectively velocity-deaf — common surprise on patches with the wrong envelope→amp routing.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_velocity_response(
        &self,
        params: Parameters<AnalyzeVelocityResponseParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_velocity_response(
                params.0.instrument_id,
                params.0.note,
                params.0.velocity_low,
                params.0.velocity_high,
                params.0.velocity_step,
                params.0.duration_ms,
                params.0.tail_ms,
            )
        })
    }

    #[tool(
        description = "Section-level form analysis. Walks the arrangement (or a single pattern's bars in pattern scope) one bar at a time, builds a duration-weighted pitch-class histogram + note-density + active-track feature row per bar, computes a cosine self-similarity matrix, and merges adjacent similar bars into sections. Sections that match a previously labeled section (similarity >= threshold) reuse its letter; near-matches get a prime (e.g. A'). Returns the per-bar feature rows, the detected sections with per-section stats, and the distinct section count. Pure symbolic — no audio rendering. Pair with `analyze_form_map` for the compact letter-string view. `exclude_drums` defaults to true.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_arrangement(
        &self,
        params: Parameters<AnalyzeArrangementParam>,
    ) -> String {
        match self.bridge.analyze_arrangement(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.similarity_threshold,
            params.0.section_min_bars,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Compact view of the same section clustering as `analyze_arrangement`: one label per bar and a run-length-compressed form string like 'AABA' or 'ABACABA'. Cheaper to read for 'what's the structure of this song?' prompts. Uses the same default similarity threshold (0.85) and section_min_bars (2) merging. Empty bars (no melodic notes) appear as '·' in `bar_labels` and are skipped in the form string.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_form_map(&self, params: Parameters<AnalyzeFormMapParam>) -> String {
        match self.bridge.analyze_form_map(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.similarity_threshold,
            params.0.section_min_bars,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Single-number 'does this song have a hook?' diagnostic. Runs `find_motifs` internally with min_interval_length (default 3) and min_count (default 3), then scores the result: hook_score = 0.5 × normalized_longest_motif_length + 0.3 × log2(1 + best_count) / log2(1 + total_notes) + 0.2 × coverage_ratio, clamped to [0, 1]. coverage_ratio is the fraction of melodic notes that participate in at least one qualifying motif. `strongest_motif` is the longest motif (ties broken by count) if any qualify; absent when the score is 0. Pure symbolic — no audio rendering.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_hook_strength(
        &self,
        params: Parameters<AnalyzeHookStrengthParam>,
    ) -> String {
        match self.bridge.analyze_hook_strength(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.min_interval_length,
            params.0.min_count,
            params.0.max_occurrences_per_motif,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Bar-level tension curve over the scope. Builds per-bar rows from existing analyzers — harmonic_function for chord tension, bar_features for density/register/rhythmic activity, plus (in audio mode) a single offline render sliced per bar for loudness, brightness, band entropy, and stereo width. Returns per-bar values, the cluster-derived section labels (so the caller can map bars to A/B/A'), a peak/trough/mean/std-dev summary, and shape warnings: chorus reprises with lower energy, builds that peak too early, drops that lose low-end, and otherwise monotone curves. `include_audio` defaults to true in arrangement scope and false in pattern scope. No new measurements — pure synthesis on top of the existing analyzers.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_tension_curve(
        &self,
        params: Parameters<AnalyzeTensionCurveParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_tension_curve(
                params.0.pattern_id,
                params.0.arrangement_start_tick,
                params.0.arrangement_end_tick,
                params.0.include_audio,
                params.0.similarity_threshold,
                params.0.section_min_bars,
                params.0.exclude_drums,
                params.0.exclude_track_ids,
            )
        })
    }

    #[tool(
        description = "Meta-analysis: runs the relevant analyzers across harmony, mix, groove, arrangement, composition, and patch categories, applies a rule set per category, and returns ranked fix suggestions with supporting evidence. No new measurements — every suggestion references metrics already produced by the underlying analyzer tools. `categories` is a subset of [harmony, mix, groove, arrangement, composition, patch] (empty/null = all). `include_audio` (default true) gates the mix-bus / masking / audio-augmented tension-curve checks. The audio-backed mix rules render the arrangement offline, which is capped at 300 seconds per window; when the analyzed scope is longer the densest 300-second window is sampled automatically (mix problems concentrate where the arrangement is busiest) and a `warnings` entry reports the sampled bar range — so the mix rules run on long songs instead of being skipped. `max_suggestions` defaults to 15.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn suggest_music_fixes(
        &self,
        params: Parameters<SuggestMusicFixesParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.suggest_music_fixes(
                params.0.pattern_id,
                params.0.arrangement_start_tick,
                params.0.arrangement_end_tick,
                params.0.categories,
                params.0.include_audio,
                params.0.max_suggestions,
                params.0.exclude_drums,
                params.0.exclude_track_ids,
            )
        })
    }

    #[tool(
        description = "Parse a chord symbol (e.g. 'Cm7', 'F#maj7', 'Bbsus4', 'G7sus4', 'C5') and return MIDI notes for the requested voicing rooted at `octave` (default 4 = middle-C octave). Voicings: 'close' (default — notes stacked above the root), 'drop2' (drop the 2nd-highest note an octave), 'drop3' (drop the 3rd-highest), 'open' (drop2+drop3 combined). Pure symbolic — does not touch the song; pair with `add_note` to place. Saves the AI from re-deriving chord intervals by hand on every progression.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn generate_chord(&self, params: Parameters<GenerateChordParam>) -> String {
        match self.bridge.generate_chord(
            &params.0.symbol,
            params.0.octave.unwrap_or(4),
            params.0.voicing.as_deref(),
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create a new pattern and fill it with a chord progression in one call. Each symbol in `chords` is voiced \
                       like generate_chord and placed as a block of notes spanning `beats_per_chord` (default 4 = one 4/4 bar), \
                       laid end to end. Returns the new pattern id, total length, and a per-chord breakdown. Saves building \
                       pad/glue patterns chord-by-chord with generate_chord + add_notes.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn create_chord_progression_pattern(
        &self,
        params: Parameters<CreateChordProgressionPatternParam>,
    ) -> String {
        let p = params.0;
        let velocity = p.velocity.unwrap_or(80);
        if velocity > 127 {
            return format!("Error: {}", McpBridgeError::InvalidVelocity(velocity));
        }
        match self.bridge.create_chord_progression_pattern(
            &p.name,
            &p.chords,
            p.beats_per_chord.unwrap_or(4.0),
            p.octave.unwrap_or(4),
            p.voicing.as_deref(),
            velocity,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Transpose every note in `pattern_id` by `semitones` (signed). Notes whose new pitch would leave the 0..127 MIDI range are left untouched and counted in `notes_out_of_range`. When both `scale_tonic` (0..12) and `scale_name` are set, transposed pitches that land off-scale are snapped to the nearest in-scale pitch using `tie_break` ('up'/'down'/'nearest', default 'up') — useful for staying diatonic when the AI shifts a phrase. Scale names: major, minor, harmonic_minor, melodic_minor, dorian, phrygian, lydian, mixolydian, locrian, pentatonic_major, pentatonic_minor, blues, chromatic. Replaces a 20-call sequence of update_note transposes.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn transpose_notes(&self, params: Parameters<TransposeNotesParam>) -> String {
        match self.bridge.transpose_notes(
            params.0.pattern_id,
            params.0.semitones,
            params.0.scale_tonic,
            params.0.scale_name.as_deref(),
            params.0.tie_break.as_deref(),
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Snap every note in `pattern_id` to the nearest pitch of the given key/scale. Cleans up AI-generated material that drifted off-key. Returns notes_already_in_scale + notes_moved, mean and max absolute correction in semitones. `tie_break` ('up' default / 'down' / 'nearest') decides which way to snap when a pitch is equidistant from two scale degrees. Scale names: major, minor, harmonic_minor, melodic_minor, dorian, phrygian, lydian, mixolydian, locrian, pentatonic_major, pentatonic_minor, blues, chromatic.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn quantize_notes_to_scale(
        &self,
        params: Parameters<QuantizeNotesToScaleParam>,
    ) -> String {
        match self.bridge.quantize_notes_to_scale(
            params.0.pattern_id,
            params.0.scale_tonic,
            &params.0.scale_name,
            params.0.tie_break.as_deref(),
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Snap note start ticks in `pattern_id` to a `grid_ticks` grid (240 = sixteenth at 960 PPQN, 480 = eighth, 960 = quarter) with optional swing (0..=1, even positions stay / odd push back by up to half-grid), humanization (max ±tick jitter per note), and quantize strength (0..=1; 1.0 = full snap, 0.5 = halfway between original and grid). Humanization is deterministic given the same `humanize_seed` (default 0) — reuse the seed to A/B compare different swing/strength settings without changing the jitter pattern. Returns notes_moved, mean and max tick deltas. Pure symbolic — no rendering.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn quantize_notes_to_grid(
        &self,
        params: Parameters<QuantizeNotesToGridParam>,
    ) -> String {
        match self.bridge.quantize_notes_to_grid(
            params.0.pattern_id,
            params.0.grid_ticks,
            params.0.strength,
            params.0.swing,
            params.0.humanize_ticks,
            params.0.humanize_seed,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Symbolic drum-feel analysis. Classifies each drum note via the General MIDI drum map (kick / snare / closed-hat / open-hat / tom / cymbal / clap / other) and reports backbeat strength (snare hits landing on beats 2 and 4), hat subdivision (quarter / 8th / 16th / triplet_8th / triplet_16th / irregular / none), hat density per beat, ghost-note count (snare hits below half the loudest snare velocity), fill candidates (bars whose density exceeds 2× the mean), and bar-level repetition over drum notes. Pure symbolic — no audio rendering. Pass `pattern_id` to analyze one pattern as-is (no drum-track filtering); omit it (with optional `arrangement_start_tick` / `arrangement_end_tick`) to analyze every track auto-classified as Drums by `get_instrument_profiles` (confidence ≥ 0.6). Useful for answering 'why does this beat sound flat?' without listening.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_drum_groove(
        &self,
        params: Parameters<AnalyzeDrumGrooveParam>,
    ) -> String {
        match self.bridge.analyze_drum_groove(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Symbolic kick/bass-lock diagnostics — answers 'does the bass actually work with the beat?' without listening. Identifies drum tracks (Role::Drums, conf ≥ 0.6) and bass tracks (Role::Bass, conf ≥ 0.6) via the same `infer_all_profiles` path that `analyze_harmony`'s drum filter uses, then aligns kick onsets (GM MIDI 35/36) against bass note onsets within `onset_tolerance_ticks` (default 120 = ±1/32-note at 960 PPQN). Reports `lock_score` (matched kicks / total kicks — how often the kick gets bass support), `coverage_score` (matched / total bass onsets — how often the bass has a kick beneath it), kick-only / bass-only counts, and a bass-pitch stability summary (most common pitch class on matched onsets and its share — high share = rooted bass, low share + many PCs = walking or melodic bass). Pass `pattern_id` to analyze a single combined rhythm-section pattern (kicks = GM kick MIDI, bass = everything else); omit it for arrangement scope across track-classified content.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_bass_drum_lock(
        &self,
        params: Parameters<AnalyzeBassDrumLockParam>,
    ) -> String {
        match self.bridge.analyze_bass_drum_lock(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.onset_tolerance_ticks,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Tonal-function analysis on top of analyze_harmony. Runs the same chord-identification + key inference pipeline, then annotates each chord with a scale-degree Roman numeral (I, V7, IV, vii°, …), a function bucket (tonic / subdominant / dominant / other / chromatic), and a 0..1 tension score; detects cadences (authentic V → I, plagal IV → I, half — anything → V, deceptive V → vi) on consecutive chord pairs and reports a function distribution + tension-curve summary. Use this to reason about progression quality and direction — 'does this song actually resolve?' or 'where does the tension peak?'. Pass `pattern_id` for one pattern, or omit it (with optional `arrangement_start_tick` / `arrangement_end_tick`) for an arrangement range. `exclude_drums` defaults to true.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn analyze_harmonic_function(
        &self,
        params: Parameters<AnalyzeHarmonicFunctionParam>,
    ) -> String {
        match self.bridge.analyze_harmonic_function(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.grouping_ticks,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }
}
