//! samples MCP tool handlers.

use super::super::*;

#[tool_router(router = samples_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "List all samples in the sample library. Returns id, name, duration, channels, \
                       sample rate, root note, and source type for each sample. Use optional \
                       name_filter to search by name substring.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_samples(&self, params: Parameters<ListSamplesParam>) -> String {
        match self.bridge.list_samples(params.0.name_filter.as_deref()) {
            Ok(samples) => to_json(&samples),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Import one or more WAV files into the sample library. Returns the array of new \
                       sample infos with assigned IDs. Each entry may override the name and set the root \
                       MIDI note (0-127, default 60=C4).",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn import_sample(&self, params: Parameters<ImportSampleParam>) -> String {
        for s in &params.0.samples {
            if let Some(note) = s.root_note
                && let Err(e) = validate_midi_note(MidiNote::new(note))
            {
                return format!("Error: {e}");
            }
        }
        let mut infos = Vec::new();
        let mut errors = Vec::new();
        for s in &params.0.samples {
            match self
                .bridge
                .import_sample(&s.path, s.name.as_deref(), s.root_note)
            {
                Ok(info) => infos.push(info),
                Err(e) => errors.push(format!("'{}': {e}", s.path)),
            }
        }
        batch_json("imported", &infos, &errors)
    }

    #[tool(
        description = "Delete one or more samples from the library by ID. Use list_samples to find sample IDs.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_sample(&self, params: Parameters<DeleteSamplesParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.delete_sample(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "samples deleted", &[], &errors)
    }

    #[tool(
        description = "Rename one or more samples in the library.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn rename_sample(&self, params: Parameters<RenameSampleParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_sample(it.sample_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "samples renamed", &[], &errors)
    }

    #[tool(
        description = "Set the root MIDI note for one or more samples (determines playback pitch mapping). \
                       Note 60 = C4 (middle C). Range: 0-127.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_sample_root_note(
        &self,
        params: Parameters<SetSampleRootNoteParam>,
    ) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_midi_note(it.note) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_sample_root_note(it.sample_id, it.note) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "sample root notes set", &[], &errors)
    }

    #[tool(
        description = "Normalize peak level to 0 dB (maximum without clipping) for one or more samples.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn normalize_sample(&self, params: Parameters<SampleIdsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.normalize_sample(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "samples normalized", &[], &errors)
    }

    #[tool(
        description = "Reverse the audio data in place for one or more samples.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn reverse_sample(&self, params: Parameters<SampleIdsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.reverse_sample(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "samples reversed", &[], &errors)
    }

    #[tool(
        description = "Auto-trim silence from the start and end of one or more samples. Sets crop markers \
                       at the first and last audible frames (threshold: -40 dB).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn trim_sample_silence(&self, params: Parameters<SampleIdsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.trim_sample_silence(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "samples trimmed", &[], &errors)
    }

    #[tool(
        description = "Get detailed information about a sample including peak level, RMS, DC offset, \
                       memory usage, and loop/crop regions in seconds.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_sample_info(&self, params: Parameters<SampleIdParam>) -> String {
        match self.bridge.get_sample_info(params.0.sample_id) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create a copy of one or more samples, each with a new ID. The copy gets \" (copy)\" \
                       appended to its name. Returns the array of new sample infos.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn duplicate_sample(&self, params: Parameters<SampleIdsParam>) -> String {
        let mut infos = Vec::new();
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.duplicate_sample(*id) {
                Ok(info) => infos.push(info),
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_json("duplicated", &infos, &errors)
    }

    #[tool(
        description = "Set or disable the loop region for one or more samples. When enabled, provide start \
                       and end times in seconds. Optional crossfade in milliseconds smooths the \
                       loop boundary.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_sample_loop(&self, params: Parameters<SetSampleLoopParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_sample_loop(
                it.sample_id,
                it.enabled,
                it.start_seconds,
                it.end_seconds,
                it.crossfade_ms,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "sample loops set", &[], &errors)
    }

    #[tool(
        description = "Set or remove the crop region for one or more samples. Crop defines the audible \
                       portion. Omit start_seconds and end_seconds to remove the crop and use \
                       the full sample.",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn set_sample_crop(&self, params: Parameters<SetSampleCropParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_sample_crop(it.sample_id, it.start_seconds, it.end_seconds)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "sample crops updated", &[], &errors)
    }

    #[tool(
        description = "Export one or more samples to WAV files at the given paths. Crop region is applied \
                       if set. Bit depth: 16 (default), 24, or 32 (float).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn export_sample(&self, params: Parameters<ExportSampleParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for s in params.0.samples {
            match self.bridge.export_sample(s.sample_id, &s.path, s.bit_depth) {
                Ok(()) => oks.push(s.path),
                Err(e) => errors.push(format!("{}: {e}", s.sample_id)),
            }
        }
        batch_msg(oks.len(), "samples exported", &oks, &errors)
    }

    // ========================================================================
    // SAMPLER MODULE TOOLS
    // ========================================================================

    #[tool(
        description = "Assign a sample to a Sampler module in an instrument. The module must be \
                       of type 'sampler' (prefix 'sam'). Use list_samples for sample IDs and \
                       get_instrument_info for module IDs.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn assign_sample_to_module(
        &self,
        params: Parameters<AssignSampleParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .assign_sample_to_module(it.instrument_id, &it.module_id, it.sample_id)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.module_id)),
            }
        }
        batch_msg(ok_count, "samples assigned to modules", &[], &errors)
    }

    #[tool(
        description = "Get the current state of a Sampler module: assigned sample, pitch tracking, \
                       level, play mode, direction, velocity sensitivity, fine tune, start offset.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_sampler_state(&self, params: Parameters<SamplerModuleParam>) -> String {
        match self
            .bridge
            .get_sampler_state(params.0.instrument_id, &params.0.module_id)
        {
            Ok(state) => to_json(&state),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set a parameter on a Sampler module. Parameters: pitch_tracking (true/false), \
                       level (0.0-1.0), play_mode (one_shot/sustain/loop), direction \
                       (forward/reverse/ping_pong), velocity_sensitivity (0.0-1.0), \
                       fine_tune (-100 to 100 cents), start_offset (0.0-1.0).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_sampler_parameter(
        &self,
        params: Parameters<SetSamplerParameterParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.params {
            match self.bridge.set_sampler_parameter(
                it.instrument_id,
                &it.module_id,
                &it.param_name,
                &it.value,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.module_id, it.param_name)),
            }
        }
        batch_msg(ok_count, "sampler parameters set", &[], &errors)
    }

    // ========================================================================
    // AUDIO INPUT TOOLS
    // ========================================================================
}
