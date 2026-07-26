use super::*;

impl synth_mcp::bridge::AnalysisBridge for AppSynthBridge {
    fn render_note_preview(
        &self,
        instrument_id: InstrumentId,
        note: MidiNote,
        velocity: u8,
        duration_ms: u32,
        tail_ms: u32,
    ) -> Result<synth_mcp::types::AudioPreview, McpBridgeError> {
        crate::audio::preview::render_note_preview(
            &self.session,
            &self.sample_library,
            instrument_id,
            note,
            Velocity::from_midi(velocity),
            duration_ms,
            tail_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn analyze_note(
        &self,
        instrument_id: InstrumentId,
        note: MidiNote,
        velocity: u8,
        duration_ms: u32,
        tail_ms: u32,
        expected_note: Option<u8>,
        envelope_window_ms: Option<f32>,
    ) -> Result<synth_mcp::types::AnalyzeNoteResult, McpBridgeError> {
        let mut result = analyze_rendered_note(
            &self.session,
            &self.sample_library,
            NoteAnalysisQuery::from_wire(
                instrument_id,
                note,
                velocity,
                duration_ms,
                tail_ms,
                expected_note,
                envelope_window_ms,
            ),
        )?;
        // Attach the patch's intent so the agent can correlate the measured
        // signal with why each module is there. Done here (not in the shared
        // buffer-analysis path) so the velocity/range sweeps don't repeat the
        // lookup per step.
        let modules = self
            .session
            .state()
            .shared_graph
            .get_modules_for_instrument(instrument_id);
        result.module_descriptions = collect_module_descriptions(&modules);
        Ok(result)
    }

    fn analyze_harmony(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        grouping_ticks: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<TrackId>>,
    ) -> Result<AnalyzeHarmonyResult, McpBridgeError> {
        let (query, mut scope_warnings) = harmony_query_from_flat(
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            exclude_drums,
            exclude_track_ids,
        );
        let mut result = analyze_song_harmony(&self.session, &self.shared, query, grouping_ticks)?;
        // Surface the flat-API "ignored in pattern scope" warnings first.
        scope_warnings.append(&mut result.warnings);
        result.warnings = scope_warnings;
        Ok(result)
    }

    fn analyze_pattern(
        &self,
        pattern_id: PatternId,
    ) -> Result<AnalyzePatternResult, McpBridgeError> {
        analyze_pattern_impl(&self.shared, pattern_id)
    }

    fn analyze_drum_groove(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
    ) -> Result<synth_mcp::types::AnalyzeDrumGrooveResult, McpBridgeError> {
        analyze_drum_groove_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
        )
    }

    fn analyze_bass_drum_lock(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        onset_tolerance_ticks: Option<u32>,
    ) -> Result<synth_mcp::types::AnalyzeBassDrumLockResult, McpBridgeError> {
        analyze_bass_drum_lock_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            onset_tolerance_ticks,
        )
    }

    fn analyze_harmonic_function(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        grouping_ticks: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<TrackId>>,
    ) -> Result<synth_mcp::types::AnalyzeHarmonicFunctionResult, McpBridgeError> {
        analyze_harmonic_function_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            grouping_ticks,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_arrangement(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        similarity_threshold: Option<f32>,
        section_min_bars: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<TrackId>>,
    ) -> Result<synth_mcp::types::AnalyzeArrangementResult, McpBridgeError> {
        analyze_arrangement_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            similarity_threshold,
            section_min_bars,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_form_map(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        similarity_threshold: Option<f32>,
        section_min_bars: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<TrackId>>,
    ) -> Result<synth_mcp::types::AnalyzeFormMapResult, McpBridgeError> {
        analyze_form_map_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            similarity_threshold,
            section_min_bars,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn find_motifs(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        min_interval_length: Option<u8>,
        max_interval_length: Option<u8>,
        min_count: Option<u32>,
        top_n: Option<u32>,
        max_occurrences_per_motif: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<TrackId>>,
    ) -> Result<synth_mcp::types::FindMotifsResult, McpBridgeError> {
        find_motifs_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            min_interval_length,
            max_interval_length,
            min_count,
            top_n,
            max_occurrences_per_motif,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_hook_strength(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        min_interval_length: Option<u8>,
        min_count: Option<u32>,
        max_occurrences_per_motif: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<TrackId>>,
    ) -> Result<synth_mcp::types::AnalyzeHookStrengthResult, McpBridgeError> {
        analyze_hook_strength_impl(
            &self.session,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            min_interval_length,
            min_count,
            max_occurrences_per_motif,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_tension_curve(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        include_audio: Option<bool>,
        similarity_threshold: Option<f32>,
        section_min_bars: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<TrackId>>,
    ) -> Result<synth_mcp::types::AnalyzeTensionCurveResult, McpBridgeError> {
        analyze_tension_curve_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            include_audio,
            similarity_threshold,
            section_min_bars,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn suggest_music_fixes(
        &self,
        pattern_id: Option<PatternId>,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        categories: Option<Vec<String>>,
        include_audio: Option<bool>,
        max_suggestions: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<TrackId>>,
    ) -> Result<synth_mcp::types::SuggestMusicFixesResult, McpBridgeError> {
        suggest_music_fixes_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            categories,
            include_audio,
            max_suggestions,
            exclude_drums,
            exclude_track_ids,
        )
    }

    fn analyze_mix_bus(
        &self,
        duration_seconds: f32,
        start_tick: Option<Tick>,
        include_per_track: Option<bool>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<AnalyzeMixBusResult, McpBridgeError> {
        analyze_mix_bus_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            duration_seconds,
            start_tick,
            include_per_track,
            scope,
        )
    }

    fn render_to_wav(
        &self,
        path: String,
        duration_seconds: f32,
        start_tick: Option<Tick>,
        instrument_id: Option<InstrumentId>,
        scope: synth_mcp::AnalysisScope,
        tail: synth_core::Seconds,
    ) -> Result<RenderToWavResult, McpBridgeError> {
        render_to_wav_with_tail_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            path,
            duration_seconds,
            start_tick,
            instrument_id,
            scope,
            tail,
        )
    }

    fn analyze_spectrum(
        &self,
        duration_seconds: f32,
        start_tick: Option<Tick>,
        instrument_id: Option<InstrumentId>,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<synth_mcp::types::AnalyzeSpectrumResult, McpBridgeError> {
        analyze_spectrum_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            duration_seconds,
            start_tick,
            instrument_id,
            f0_hint,
            max_partials,
            log_bins,
            scope,
        )
    }

    fn analyze_spectrogram(
        &self,
        duration_seconds: f32,
        start_tick: Option<Tick>,
        instrument_id: Option<InstrumentId>,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        hop_ms: Option<f32>,
        window_len_ms: Option<f32>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<synth_mcp::types::AnalyzeSpectrogramResult, McpBridgeError> {
        analyze_spectrogram_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            duration_seconds,
            start_tick,
            instrument_id,
            f0_hint,
            max_partials,
            log_bins,
            hop_ms,
            window_len_ms,
            scope,
        )
    }

    fn analyze_sample_spectrum(
        &self,
        sample_id_or_path: String,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        start_ms: Option<f32>,
        window_len_ms: Option<f32>,
    ) -> Result<synth_mcp::types::AnalyzeSampleSpectrumResult, McpBridgeError> {
        analyze_sample_spectrum_impl(
            &self.sample_library,
            sample_id_or_path,
            f0_hint,
            max_partials,
            log_bins,
            start_ms,
            window_len_ms,
        )
    }

    fn analyze_sample_spectrogram(
        &self,
        sample_id_or_path: String,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        hop_ms: Option<f32>,
        window_len_ms: Option<f32>,
    ) -> Result<synth_mcp::types::AnalyzeSampleSpectrogramResult, McpBridgeError> {
        analyze_sample_spectrogram_impl(
            &self.sample_library,
            sample_id_or_path,
            f0_hint,
            max_partials,
            log_bins,
            hop_ms,
            window_len_ms,
        )
    }

    fn compare_spectra(
        &self,
        target: synth_mcp::SpectrumSource,
        candidate: synth_mcp::SpectrumSource,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        mel_bands: Option<u32>,
        scope: synth_mcp::AnalysisScope,
        time_resolved: synth_mcp::TimeResolvedOptions,
    ) -> Result<synth_mcp::types::CompareSpectraResult, McpBridgeError> {
        compare_spectra_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            target,
            candidate,
            f0_hint,
            max_partials,
            log_bins,
            mel_bands,
            scope,
            time_resolved,
        )
    }

    fn compare_envelopes(
        &self,
        target: synth_mcp::SpectrumSource,
        candidate: synth_mcp::SpectrumSource,
        envelope_window_ms: Option<f32>,
        note_duration_ms: Option<u32>,
        transient_window_ms: Option<f32>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<synth_mcp::types::CompareEnvelopesResult, McpBridgeError> {
        compare_envelopes_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            target,
            candidate,
            envelope_window_ms,
            note_duration_ms,
            transient_window_ms,
            scope,
        )
    }

    fn analyze_master_chain(
        &self,
        duration_seconds: f32,
        start_tick: Option<Tick>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<AnalyzeMasterChainResult, McpBridgeError> {
        analyze_master_chain_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            duration_seconds,
            start_tick,
            scope,
        )
    }

    fn analyze_return_busses(
        &self,
        duration_seconds: f32,
        start_tick: Option<Tick>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<AnalyzeReturnBussesResult, McpBridgeError> {
        analyze_return_busses_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            duration_seconds,
            start_tick,
            scope,
        )
    }

    fn compare_mix_before_after(
        &self,
        action: &str,
        duration_seconds: f32,
        start_tick: Option<Tick>,
        label: Option<String>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<CompareMixResult, McpBridgeError> {
        compare_mix_before_after_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            action,
            duration_seconds,
            start_tick,
            label,
            scope,
        )
    }

    fn capture_snapshot(&self) -> Result<(), McpBridgeError> {
        let mut slot = self.rollback_snapshot.lock();
        // Refuse rather than overwrite: an occupied slot means another rollback
        // batch is mid-flight, and overwriting it would let one batch restore
        // the other's snapshot. Concurrent rollback batches are unsupported.
        if slot.is_some() {
            return Err(McpBridgeError::Other(
                "a rollback batch is already in progress — concurrent rollback batches are \
                 not supported"
                    .to_string(),
            ));
        }
        let opts = self.build_save_options();
        let project = crate::project_apply::build_project_from_engine(
            &self.session,
            &self.shared.song,
            &self.sample_library,
            opts,
        );
        *slot = Some(Box::new(project));
        Ok(())
    }

    fn restore_snapshot(&self) -> Result<(), McpBridgeError> {
        let project =
            self.rollback_snapshot.lock().take().ok_or_else(|| {
                McpBridgeError::Other("no project snapshot to restore".to_string())
            })?;
        crate::project_apply::apply_project(
            &project,
            &self.session,
            &self.shared.song,
            &self.sample_library,
        )
        .map_err(McpBridgeError::Other)?;
        // Rebuild the GUI mirrors against the restored project, mirroring a
        // project load — otherwise the GUI keeps showing the failed-batch state.
        self.stash_refresh(crate::mcp_shared::ProjectRefresh::Loaded(project));
        Ok(())
    }

    fn clear_snapshot(&self) {
        *self.rollback_snapshot.lock() = None;
    }

    fn analyze_section(
        &self,
        start_tick: Tick,
        end_tick: Tick,
        include_per_track: Option<bool>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<AnalyzeSectionResult, McpBridgeError> {
        analyze_section_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            start_tick,
            end_tick,
            include_per_track,
            scope,
        )
    }

    fn auto_gain_stage(
        &self,
        target_lufs: f32,
        true_peak_ceiling_dbtp: f32,
        duration_seconds: f32,
        start_tick: Option<Tick>,
    ) -> Result<AutoGainStageResult, McpBridgeError> {
        auto_gain_stage_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            target_lufs,
            true_peak_ceiling_dbtp,
            duration_seconds,
            start_tick,
        )
    }

    fn analyze_masking_matrix(
        &self,
        arrangement_start_tick: Option<Tick>,
        arrangement_end_tick: Option<Tick>,
        top_pairs: Option<u32>,
        scope: synth_mcp::AnalysisScope,
    ) -> Result<AnalyzeMaskingMatrixResult, McpBridgeError> {
        analyze_masking_matrix_impl(
            &self.session,
            &self.sample_library,
            &self.shared,
            arrangement_start_tick,
            arrangement_end_tick,
            top_pairs,
            scope,
        )
    }

    fn analyze_instrument_range(
        &self,
        instrument_id: InstrumentId,
        low_note: u8,
        high_note: u8,
        step_semitones: Option<u8>,
        velocity: Option<u8>,
        duration_ms: Option<u32>,
        tail_ms: Option<u32>,
    ) -> Result<synth_mcp::types::AnalyzeInstrumentRangeResult, McpBridgeError> {
        analyze_instrument_range_impl(
            &self.session,
            &self.sample_library,
            InstrumentRangeQuery::from_wire(
                instrument_id,
                low_note,
                high_note,
                step_semitones,
                velocity,
                duration_ms,
                tail_ms,
            )?,
        )
    }

    fn analyze_velocity_response(
        &self,
        instrument_id: InstrumentId,
        note: MidiNote,
        velocity_low: Option<u8>,
        velocity_high: Option<u8>,
        velocity_step: Option<u8>,
        duration_ms: Option<u32>,
        tail_ms: Option<u32>,
    ) -> Result<synth_mcp::types::AnalyzeVelocityResponseResult, McpBridgeError> {
        analyze_velocity_response_impl(
            &self.session,
            &self.sample_library,
            VelocityResponseQuery::from_wire(
                instrument_id,
                note,
                velocity_low,
                velocity_high,
                velocity_step,
                duration_ms,
                tail_ms,
            )?,
        )
    }

    fn generate_chord(
        &self,
        symbol: &str,
        octave: i32,
        voicing: Option<&str>,
    ) -> Result<synth_mcp::types::GenerateChordResult, McpBridgeError> {
        generate_chord_impl(symbol, octave, voicing)
    }

    fn transpose_notes(
        &self,
        pattern_id: PatternId,
        semitones: Semitones,
        scale_tonic: Option<u8>,
        scale_name: Option<&str>,
        tie_break: Option<&str>,
    ) -> Result<synth_mcp::types::TransposeNotesResult, McpBridgeError> {
        transpose_notes_impl(
            &self.shared,
            pattern_id,
            semitones,
            scale_tonic,
            scale_name,
            tie_break,
        )
    }

    fn quantize_notes_to_scale(
        &self,
        pattern_id: PatternId,
        scale_tonic: u8,
        scale_name: &str,
        tie_break: Option<&str>,
    ) -> Result<synth_mcp::types::QuantizeNotesToScaleResult, McpBridgeError> {
        quantize_notes_to_scale_impl(&self.shared, pattern_id, scale_tonic, scale_name, tie_break)
    }

    fn quantize_notes_to_grid(
        &self,
        pattern_id: PatternId,
        grid_ticks: u32,
        strength: Option<f32>,
        swing: Option<f32>,
        humanize_ticks: Option<u32>,
        humanize_seed: Option<u64>,
    ) -> Result<synth_mcp::types::QuantizeNotesToGridResult, McpBridgeError> {
        quantize_notes_to_grid_impl(
            &self.shared,
            pattern_id,
            grid_ticks,
            strength,
            swing,
            humanize_ticks,
            humanize_seed,
        )
    }

    // === Sample library ===
}
