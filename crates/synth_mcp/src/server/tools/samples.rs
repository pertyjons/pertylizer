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
    pub(crate) async fn list_samples(
        &self,
        params: Parameters<ListSamplesParam>,
    ) -> Result<Json<Listing<SampleInfo>>, String> {
        match self.bridge.list_samples(params.0.name_filter.as_deref()) {
            Ok(samples) => Ok(Json(samples.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Import one or more WAV files into the sample library. Returns \
                       `{ message, items: [{ index, value, error }] }` — one entry per requested path, in \
                       request order: `value` is the imported sample info (with its assigned ID) and `error` \
                       names the failure. A path that fails does not fail the others. Each entry may override \
                       the name and set the root MIDI note (0-127, default 60=C4).",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn import_sample(
        &self,
        params: Parameters<ImportSampleParam>,
    ) -> Result<Json<MutationResult<SampleInfo>>, String> {
        for s in &params.0.samples {
            if let Some(note) = s.root_note
                && let Err(e) = validate_midi_note(MidiNote::new(note))
            {
                return Err(format!("Error: {e}"));
            }
        }
        let mut items = Mutations::with_capacity(params.0.samples.len());
        for s in &params.0.samples {
            match self
                .bridge
                .import_sample(&s.path, s.name.as_deref(), s.root_note)
            {
                Ok(info) => items.named(info),
                Err(e) => items.failed(format!("'{}': {e}", s.path)),
            }
        }
        Ok(Json(items.into_result("samples imported")))
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Delete one or more samples from the library by ID. Use list_samples to find sample IDs.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_sample(
        &self,
        params: Parameters<DeleteSamplesParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for id in &params.0.sample_ids {
            match self.bridge.delete_sample(*id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{id}: {e}")),
            }
        }
        items.reply("samples deleted")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Rename one or more samples in the library.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn rename_sample(
        &self,
        params: Parameters<RenameSampleParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.rename_sample(it.sample_id, &it.name) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.sample_id)),
            }
        }
        items.reply("samples renamed")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set the root MIDI note for one or more samples (determines playback pitch mapping). \
                       Note 60 = C4 (middle C). Range: 0-127.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_sample_root_note(
        &self,
        params: Parameters<SetSampleRootNoteParam>,
    ) -> CallToolResult {
        for (index, it) in params.0.items.iter().enumerate() {
            if let Err(e) = validate_midi_note(it.note) {
                return action_rejected_at(index, format!("Error: {e}"));
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.set_sample_root_note(it.sample_id, it.note) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.sample_id)),
            }
        }
        items.reply("sample root notes set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Normalize peak level to 0 dB (maximum without clipping) for one or more samples.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn normalize_sample(
        &self,
        params: Parameters<SampleIdsParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for id in &params.0.sample_ids {
            match self.bridge.normalize_sample(*id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{id}: {e}")),
            }
        }
        items.reply("samples normalized")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Reverse the audio data in place for one or more samples.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn reverse_sample(
        &self,
        params: Parameters<SampleIdsParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for id in &params.0.sample_ids {
            match self.bridge.reverse_sample(*id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{id}: {e}")),
            }
        }
        items.reply("samples reversed")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Auto-trim silence from the start and end of one or more samples. Sets crop markers \
                       at the first and last audible frames (threshold: -40 dB).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn trim_sample_silence(
        &self,
        params: Parameters<SampleIdsParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for id in &params.0.sample_ids {
            match self.bridge.trim_sample_silence(*id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{id}: {e}")),
            }
        }
        items.reply("samples trimmed")
    }

    #[tool(
        description = "Get detailed information about a sample including peak level, RMS, DC offset, \
                       memory usage, and loop/crop regions in seconds.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_sample_info(
        &self,
        params: Parameters<SampleIdParam>,
    ) -> Result<Json<DetailedSampleInfo>, String> {
        match self.bridge.get_sample_info(params.0.sample_id) {
            Ok(info) => Ok(Json(info)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Create a copy of one or more samples, each with a new ID. The copy gets \" (copy)\" \
                       appended to its name. Returns \
                       `{ message, items: [{ index, value, error }] }` — one entry per requested sample id, \
                       in request order, where `value` is the new sample info.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn duplicate_sample(
        &self,
        params: Parameters<SampleIdsParam>,
    ) -> Result<Json<MutationResult<SampleInfo>>, String> {
        let mut items = Mutations::with_capacity(params.0.sample_ids.len());
        for id in &params.0.sample_ids {
            match self.bridge.duplicate_sample(*id) {
                Ok(info) => items.named(info),
                Err(e) => items.failed(format!("{id}: {e}")),
            }
        }
        Ok(Json(items.into_result("samples duplicated")))
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or disable the loop region for one or more samples. When enabled, provide start \
                       and end times in seconds. Optional crossfade in milliseconds smooths the \
                       loop boundary.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_sample_loop(
        &self,
        params: Parameters<SetSampleLoopParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.set_sample_loop(
                it.sample_id,
                it.enabled,
                it.start_seconds,
                it.end_seconds,
                it.crossfade_ms,
            ) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.sample_id)),
            }
        }
        items.reply("sample loops set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or remove the crop region for one or more samples. Crop defines the audible \
                       portion. Omit start_seconds and end_seconds to remove the crop and use \
                       the full sample.",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn set_sample_crop(
        &self,
        params: Parameters<SetSampleCropParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_sample_crop(it.sample_id, it.start_seconds, it.end_seconds)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.sample_id)),
            }
        }
        items.reply("sample crops updated")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Export one or more samples to WAV files at the given paths. Crop region is applied \
                       if set. Bit depth: 16 (default), 24, or 32 (float).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn export_sample(
        &self,
        params: Parameters<ExportSampleParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for s in params.0.samples {
            match self.bridge.export_sample(s.sample_id, &s.path, s.bit_depth) {
                Ok(()) => items.named(s.path),
                Err(e) => items.failed(format!("{}: {e}", s.sample_id)),
            }
        }
        items.reply("samples exported")
    }

    // ========================================================================
    // SAMPLER MODULE TOOLS
    // ========================================================================

    #[tool(
        output_schema = action_output_schema(),
        description = "Assign a sample to a Sampler module in an instrument. The module must be \
                       of type 'sampler' (prefix 'sam'). Use list_samples for sample IDs and \
                       get_instrument_info for module IDs.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn assign_sample_to_module(
        &self,
        params: Parameters<AssignSampleParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .assign_sample_to_module(it.instrument_id, &it.module_id, it.sample_id)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.module_id)),
            }
        }
        items.reply("samples assigned to modules")
    }

    #[tool(
        description = "Get the current state of a Sampler module: assigned sample, pitch tracking, \
                       level, play mode, direction, velocity sensitivity, fine tune, start offset.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_sampler_state(
        &self,
        params: Parameters<SamplerModuleParam>,
    ) -> Result<Json<SamplerStateInfo>, String> {
        match self
            .bridge
            .get_sampler_state(params.0.instrument_id, &params.0.module_id)
        {
            Ok(state) => Ok(Json(state)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set a parameter on a Sampler module. Parameters: pitch_tracking (true/false), \
                       level (0.0-1.0), play_mode (one_shot/sustain/loop), direction \
                       (forward/reverse/ping_pong), velocity_sensitivity (0.0-1.0), \
                       fine_tune (-100 to 100 cents), start_offset (0.0-1.0).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_sampler_parameter(
        &self,
        params: Parameters<SetSamplerParameterParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.params {
            match self.bridge.set_sampler_parameter(
                it.instrument_id,
                &it.module_id,
                &it.param_name,
                &it.value,
            ) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}/{}: {e}", it.module_id, it.param_name)),
            }
        }
        items.reply("sampler parameters set")
    }

    // ========================================================================
    // AUDIO INPUT TOOLS
    // ========================================================================
}
