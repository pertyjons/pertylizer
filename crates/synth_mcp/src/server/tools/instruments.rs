//! instruments MCP tool handlers.

use super::super::*;

#[tool_router(router = instruments_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        output_schema = action_output_schema(),
        description = "Install or clear YAMS on a Mod Matrix (`mmx-N`), Script (`scr-N`), or AudioScript (`asc-N`) module. Despite the historical tool name, `module_id` selects the dialect. A Mod Matrix program writes one normalized offset with `out`; a Script is one control-rate program with `in1..in4` and `out1..out4` (bare `out` aliases `out1`); an AudioScript is one per-sample stereo program. `param` declarations expose real knobs on Script and AudioScript modules. An empty `source` clears the selected slot/program. `slot` is 1-based: Mod Matrix accepts 1..=16; Script and AudioScript require slot 1. Read back with get_mod_matrix_routings (mmx) or get_module_info (scr/asc); see get_yams_reference for the complete language.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_mod_matrix_script(
        &self,
        params: Parameters<SetModMatrixScriptParam>,
    ) -> CallToolResult {
        let p = params.0;
        match self
            .bridge
            .set_mod_matrix_script(p.instrument_id, &p.module_id, p.slot, &p.source)
        {
            Ok(()) if p.source.trim().is_empty() => action_ok(format!(
                "OK: cleared script on {} slot {}",
                p.module_id, p.slot
            )),
            Ok(()) => action_ok(format!(
                "OK: installed script on {} slot {}",
                p.module_id, p.slot
            )),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Play one or more MIDI notes (note on) — pass several to strike a whole chord in one call. Use note=60 for middle C, velocity=100 for moderate strength.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn note_on(&self, params: Parameters<NoteOnParam>) -> CallToolResult {
        for (index, n) in params.0.notes.iter().enumerate() {
            if let Err(e) = validate_midi_note(n.note) {
                return action_rejected_at(index, format!("Error: {e}"));
            }
            if let Err(e) = validate_velocity(n.velocity) {
                return action_rejected_at(index, format!("Error: {e}"));
            }
            if let Err(e) = validate_midi_channel(n.channel.unwrap_or(MidiChannel::CH1)) {
                return action_rejected_at(index, format!("Error: {e}"));
            }
        }
        let mut items = Mutations::new();
        for n in &params.0.notes {
            let channel = n.channel.unwrap_or(MidiChannel::CH1);
            match self
                .bridge
                .note_on(n.note, n.velocity, channel, n.instrument_id)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("note {}: {e}", n.note)),
            }
        }
        items.reply("notes on")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Stop one or more MIDI notes (note off). Use the same note numbers as the corresponding note_on.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn note_off(&self, params: Parameters<NoteOffParam>) -> CallToolResult {
        for (index, n) in params.0.notes.iter().enumerate() {
            if let Err(e) = validate_midi_note(n.note) {
                return action_rejected_at(index, format!("Error: {e}"));
            }
            if let Err(e) = validate_midi_channel(n.channel.unwrap_or(MidiChannel::CH1)) {
                return action_rejected_at(index, format!("Error: {e}"));
            }
        }
        let mut items = Mutations::new();
        for n in &params.0.notes {
            let channel = n.channel.unwrap_or(MidiChannel::CH1);
            match self.bridge.note_off(n.note, channel, n.instrument_id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("note {}: {e}", n.note)),
            }
        }
        items.reply("notes off")
    }

    #[tool(
        description = "A/B a mix change. Call with action='capture' to render the current master mix and store it as a baseline, make your change (EQ, levels, effects, …), then call action='compare' to re-render and get the deltas: lufs_delta, peak/true-peak/rms delta in dB, crest_delta_db (positive = more dynamic), stereo_width_delta (positive = wider), mono_compat_delta. Compare re-renders with the exact same window and signal chain the baseline used, so the deltas reflect only your change. The baseline is per-session and is never written to the project; capturing again overwrites it. Use this to confirm a tweak did what you intended (e.g. 'did adding the limiter actually lower the true peak without crushing dynamics?').",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn compare_mix_before_after(
        &self,
        params: Parameters<CompareMixBeforeAfterParam>,
    ) -> Result<Json<CompareMixResult>, String> {
        let p = params.0;
        let duration = p.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            p.include_all,
            p.include_master_effects,
            p.include_return_effects,
            crate::bridge::RenderQuality::parse(p.render_quality.as_deref()),
        );
        run_blocking_typed(|| {
            self.bridge
                .compare_mix_before_after(&p.action, duration, p.start_tick, p.label, scope)
        })
    }

    #[tool(
        description = "Find recurring melodic motifs in the scope. Converts each track's notes into a pitch-interval sequence (signed semitone deltas between consecutive notes in time order, ignoring rests), slides an n-gram window across each track (lengths min_interval_length..=max_interval_length, defaults 3..=6), counts identical interval sequences, and returns the top_n motifs (default 10) that appear at least min_count times (default 3). Transposition-invariant — the same shape rooted at different pitches collapses to one entry. Each motif lists its interval sequence, count, and per-occurrence locations (track id, start tick, bar/beat, first pitch). Pure symbolic — no audio rendering. `exclude_drums` defaults to true.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn find_motifs(
        &self,
        params: Parameters<FindMotifsParam>,
    ) -> Result<Json<FindMotifsResult>, String> {
        match self.bridge.find_motifs(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.min_interval_length,
            params.0.max_interval_length,
            params.0.min_count,
            params.0.top_n,
            params.0.max_occurrences_per_motif,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => Ok(Json(result)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "List all available example patches with their categories, descriptions, and tags",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_example_patches(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<Listing<ExamplePatchInfo>>, String> {
        match self.bridge.list_example_patches() {
            Ok(patches) => Ok(Json(patches.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Load an example patch by name. The GUI will update on the next frame. Use list_example_patches to see available patches.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn load_example_patch(
        &self,
        params: Parameters<LoadExamplePatchParam>,
    ) -> CallToolResult {
        match self.bridge.load_example_patch(&params.0.name) {
            Ok(msg) => action_ok(msg),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Request auto-layout of modules in the patch view. The GUI applies the layout on the next Rack-view frame, arranging modules by signal flow. If the user is in another view (AcousticWorld, Sequencer, Sample), the request stays pending until they return to Rack.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn auto_layout(&self, _params: Parameters<NoParams>) -> CallToolResult {
        match self.bridge.request_auto_layout() {
            Ok(msg) => action_ok(msg),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get a snapshot of the current UI layout: module positions, sizes, connections, and overlap analysis for debugging",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_ui_snapshot(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> Result<Json<UiSnapshot>, String> {
        match self.bridge.get_ui_snapshot(params.0.instrument_id) {
            Ok(snapshot) => Ok(Json(snapshot)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Add one or more modules to the instrument's voice graph. Modules appear in the GUI on the next frame. Returns the assigned module IDs (see also list_modules). GUI-only visualizer types (Oscilloscope/Meter/Spectrum) can't be added over MCP — they're flagged gui_only:true in list_module_types.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_module(&self, params: Parameters<AddModulesParam>) -> CallToolResult {
        let p = params.0;
        let mut items = Mutations::with_capacity(p.module_types.len());
        for module_type in &p.module_types {
            match self.bridge.add_module(p.instrument_id, module_type) {
                Ok(module_id) => items.named(module_id),
                Err(e) => items.failed(format!("{module_type}: {e}")),
            }
        }
        items.reply("modules added")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Remove one or more modules from the instrument's voice graph and disconnect all their cables.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_module(
        &self,
        params: Parameters<RemoveModulesParam>,
    ) -> CallToolResult {
        let p = params.0;
        let mut items = Mutations::new();
        for module_id in &p.module_ids {
            match self.bridge.remove_module(p.instrument_id, module_id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{module_id}: {e}")),
            }
        }
        items.reply("modules removed")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Connect one or more module port pairs in one call. Returns the number of successful connections and any errors. \
                       Each connection specifies from_module:from_port → to_module:to_port. Port names must match the module's ports (typically 'out'/'in'); the aliases 'output'→'out' and 'input'→'in' are also accepted.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn connect(&self, params: Parameters<ConnectMultipleParam>) -> CallToolResult {
        let mut items = Mutations::new();
        for c in &params.0.connections {
            match self.bridge.connect(
                params.0.instrument_id,
                &c.from_module,
                &c.from_port,
                &c.to_module,
                &c.to_port,
            ) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!(
                    "{}:{} → {}:{}: {e}",
                    c.from_module, c.from_port, c.to_module, c.to_port
                )),
            }
        }
        // Hand-rolled prose used to lead with "OK:" even when *every* cable was
        // rejected, which `result_is_failure` cannot see — a wholly-failed
        // `connect` then counted as a success and a `rollback: true` batch kept
        // the broken state. `action_reply` leads with "Error:" on total failure,
        // exactly as its mirror `disconnect` does.
        items.reply("connections made")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Clear the entire voice graph for an instrument, removing all modules and connections. Use this to start from scratch.",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn clear_graph(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> CallToolResult {
        match self.bridge.clear_graph(params.0.instrument_id) {
            Ok(()) => action_ok("OK: graph cleared".to_string()),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Splice a new module into an existing audio cable in one call (add + disconnect + reconnect). \
                       Re-routes source → new module → destination through the new module's audio ports. \
                       Choose where with one anchor: `after`/`before` (a module id), `after_type`/`before_type` \
                       (a module type — robust across instruments), or the explicit from_module/from_port/to_module/to_port \
                       cable when the path branches. With no anchor it inserts at the end of the audio path, just before output. \
                       The module type must carry audio. On any wiring failure the original cable is restored.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn insert_module_between(
        &self,
        params: Parameters<InsertModuleBetweenParam>,
    ) -> Result<Json<InsertModuleResult>, String> {
        let p = params.0;
        let anchor = match p.resolve_anchor() {
            Ok(a) => a,
            Err(e) => return Err(format!("Error: {e}")),
        };
        match self
            .bridge
            .insert_module_between(p.instrument_id, &p.module_type, anchor)
        {
            Ok(result) => Ok(Json(result)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Quick go/no-go check that an instrument actually produces audio: renders one test note offline \
                       and returns a compact verdict (is_audible, peak/RMS, clipping, fundamental, DC offset) plus warnings. \
                       Use to catch silent or broken patches before wiring them into a song. For full spectral detail use \
                       analyze_instrument_range or analyze_velocity_response.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn validate_instrument_audio(
        &self,
        params: Parameters<ValidateInstrumentAudioParam>,
    ) -> Result<Json<ValidateInstrumentAudioResult>, String> {
        let p = params.0;
        let note = p.note.unwrap_or(MidiNote::C4);
        let velocity = p.velocity.unwrap_or(100);
        let duration_ms = p.duration_ms.unwrap_or(500);
        let tail_ms = p.tail_ms.unwrap_or(500);
        if velocity > 127 {
            return Err(format!(
                "Error: {}",
                McpBridgeError::InvalidVelocity(velocity)
            ));
        }
        // Offline render — must not run straight on the tokio executor, or the
        // worker is blocked for the whole render and SSE keep-alives stall.
        // `analyze_note` (the same bridge call) wraps it the same way.
        run_blocking_typed(|| {
            self.bridge
                .analyze_note(
                    p.instrument_id,
                    note,
                    velocity,
                    duration_ms,
                    tail_ms,
                    None,
                    None,
                )
                .map(|r| distill_audio_validation(&r))
        })
    }

    // === Instrument lifecycle ===

    #[tool(
        description = "Create one or more instruments. Returns `{ message, items: [{ index, value, error }] }` — \
                       one entry per requested name, in request order: `value` is the created instrument info \
                       (with its assigned ID) and `error` names the failure. A name that fails does not fail the others.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn create_instrument(
        &self,
        params: Parameters<CreateInstrumentParam>,
    ) -> Result<Json<MutationResult<InstrumentInfo>>, String> {
        for name in &params.0.names {
            if let Err(e) = validate_name("instrument", name) {
                return Err(format!("Error: {e}"));
            }
        }
        let mut items = Mutations::with_capacity(params.0.names.len());
        for name in &params.0.names {
            match self.bridge.create_instrument(name) {
                Ok(info) => items.named(info),
                Err(e) => items.failed(format!("'{name}': {e}")),
            }
        }
        Ok(Json(items.into_result("instruments created")))
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Delete one or more instruments and all their modules. Cannot delete the default instrument (ID 0).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_instrument(
        &self,
        params: Parameters<DeleteInstrumentsParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for id in &params.0.instrument_ids {
            match self.bridge.delete_instrument(*id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", id.as_u64())),
            }
        }
        items.reply("instruments deleted")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Rename one or more instruments. The name is shown in the UI instrument strip and track selector.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn rename_instrument(
        &self,
        params: Parameters<RenameInstrumentParam>,
    ) -> CallToolResult {
        for (index, it) in params.0.items.iter().enumerate() {
            if let Err(e) = validate_name("instrument", &it.name) {
                return action_rejected_at(index, format!("Error: {e}"));
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.rename_instrument(it.instrument_id, &it.name) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.instrument_id.as_u64())),
            }
        }
        items.reply("instruments renamed")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or clear the free-text description / intent on an instrument. \
        The description never affects audio and is read back via list_instruments / \
        get_instrument_info. Use it to record why an instrument exists, what role it plays \
        in the song, or any analysis notes you want a future agent (or human) to see. \
        Pass an empty string to clear.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_instrument_description(
        &self,
        params: Parameters<SetInstrumentDescriptionParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_instrument_description(it.instrument_id, &it.description)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.instrument_id.as_u64())),
            }
        }
        items.reply("instrument descriptions set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or clear the accent color of one or more instruments from a \
        \"#RRGGBB\" / \"#RRGGBBAA\" hex string (pass \"\" to clear back to the default/auto \
        tint). Never affects audio; paints instruments so the mixer / arrangement is visually \
        scannable (e.g. red kick, blue pad, green bass) and is read back via list_instruments / \
        get_instrument_info. The color travels with the project on save.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_instrument_color(
        &self,
        params: Parameters<SetInstrumentColorParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_instrument_color(it.instrument_id, &it.color)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.instrument_id.as_u64())),
            }
        }
        items.reply("instrument colors set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or clear the patch-level accent color of one or more instruments from a \
        \"#RRGGBB\" / \"#RRGGBBAA\" hex string (pass \"\" to clear). Distinct from \
        set_instrument_color: this color travels with the patch when it is saved/exported, so a \
        shared patch carries its own suggested tint. Never affects audio; read back via \
        list_instruments / get_instrument_info as patch_color.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_patch_color(
        &self,
        params: Parameters<SetPatchColorParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.set_patch_color(it.instrument_id, &it.color) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.instrument_id.as_u64())),
            }
        }
        items.reply("patch colors set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or clear the patch-level description on an instrument's currently \
        loaded patch. This describes the *patch* (sound design intent, how it works, what it's \
        good for) and is distinct from set_instrument_description, which records the \
        instrument's per-instance role in the song. The patch description travels with the \
        patch when saved. Pass \"\" to clear.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_patch_description(
        &self,
        params: Parameters<SetPatchDescriptionParam>,
    ) -> CallToolResult {
        match self
            .bridge
            .set_patch_description(params.0.instrument_id, &params.0.description)
        {
            Ok(()) if params.0.description.is_empty() => action_ok(format!(
                "OK: cleared patch description on instrument {}",
                params.0.instrument_id
            )),
            Ok(()) => action_ok(format!(
                "OK: set instrument {} patch description ({} chars)",
                params.0.instrument_id,
                params.0.description.chars().count()
            )),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or clear the free-text description on one or more module instances \
        (what a particular module is for — e.g. \"wobble LFO for the filter cutoff\"). Takes an \
        array of self-contained {instrument_id, module_id, description} items, so a single call \
        can annotate modules across different instruments (mirrors set_instrument_description). \
        Distinct from get_module_type_info, which documents the module *type*. The description \
        travels with the patch when saved and is readable via get_module_info / list_modules. \
        Pass \"\" to clear an item. Max 2000 characters; an item is rejected if the module does \
        not exist.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_module_description(
        &self,
        params: Parameters<SetModuleDescriptionParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.set_module_description(
                it.instrument_id,
                &it.module_id,
                &it.description,
            ) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!(
                    "{}:{}: {e}",
                    it.instrument_id.as_u64(),
                    it.module_id
                )),
            }
        }
        items.reply("module descriptions set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or clear a sample's free-text description (its intent / source). \
        Pass \"\" to clear. Surfaces in list_samples / get_sample_info.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_sample_description(
        &self,
        params: Parameters<SetSampleDescriptionParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_sample_description(it.sample_id, &it.description)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.sample_id)),
            }
        }
        items.reply("sample descriptions set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set or clear the sidechain source on an instrument. When set, the \
        engine routes the source instrument's audio output into this instrument's \
        sidechain-capable modules (compressors with sidechain_enabled, envelope followers). \
        Use it for classic pumping/ducking — e.g. let a kick drum sidechain the pad. \
        Pass source = null (or omit) to disable. Self-routing is rejected.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_sidechain_source(
        &self,
        params: Parameters<SetSidechainSourceParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_sidechain_source(it.instrument_id, it.source)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.instrument_id.as_u64())),
            }
        }
        items.reply("sidechain sources set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set mixer state on one or more instruments in a single call. Each item \
        carries an instrument_id plus any of volume (0.0=silent, 1.0=unity, 2.0=max), pan \
        (-1.0=left..1.0=right), muted, solo, and enabled (disabled instruments skip all audio \
        processing — lighter than mute, which still processes but silences output). Omitted \
        fields are left unchanged. When any instrument is soloed, only soloed instruments sound.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_instrument_mixer(
        &self,
        params: Parameters<SetInstrumentMixerParam>,
    ) -> CallToolResult {
        // Validate all ranges up front so a bad value rejects the whole call.
        for (index, it) in params.0.items.iter().enumerate() {
            if let Some(v) = it.volume
                && let Err(e) = validate_range("volume", v.as_f32(), 0.0, 2.0)
            {
                return action_rejected_at(index, format!("Error: {e}"));
            }
            if let Some(p) = it.pan
                && let Err(e) = validate_range("pan", p.as_f32(), -1.0, 1.0)
            {
                return action_rejected_at(index, format!("Error: {e}"));
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            let id = it.instrument_id;
            let mut item_err: Option<String> = None;
            if let Some(v) = it.volume
                && let Err(e) = self.bridge.set_instrument_volume(id, v)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(p) = it.pan
                && let Err(e) = self.bridge.set_instrument_pan(id, p)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(m) = it.muted
                && let Err(e) = self.bridge.set_instrument_mute(id, m)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = it.solo
                && let Err(e) = self.bridge.set_instrument_solo(id, s)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(en) = it.enabled
                && let Err(e) = self.bridge.set_instrument_enabled(id, en)
            {
                item_err = Some(e.to_string());
            }
            match item_err {
                None => items.ok(),
                Some(e) => items.failed(format!("{}: {e}", id.as_u64())),
            }
        }
        items.reply("instrument mixer updates applied")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set voice-allocator config on one or more instruments in a single call. Each \
        item carries an instrument_id plus any of: allocation_mode (Polyphonic | Mono | Legato | \
        Unison), stealing_strategy (None | Oldest | Quietest | LowestPriority | SameNote), \
        unison_detune (0..100 cents, audible only in Unison mode), unison_spread (0.0..1.0 stereo \
        width, audible only in Unison mode), and max_voices (1..=128; applied on the next voice-graph \
        rebuild / project load, not live). Omitted fields are left unchanged. Read the current \
        values back via get_instrument_info.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_allocator_config(
        &self,
        params: Parameters<SetAllocatorConfigParam>,
    ) -> CallToolResult {
        // Validate all numeric ranges up front so a bad value rejects the whole
        // call (mode/strategy strings are validated per-item by the bridge).
        for (index, it) in params.0.items.iter().enumerate() {
            if let Some(d) = it.unison_detune
                && let Err(e) = validate_range("unison_detune", d, 0.0, 100.0)
            {
                return action_rejected_at(index, format!("Error: {e}"));
            }
            if let Some(s) = it.unison_spread
                && let Err(e) = validate_range("unison_spread", s, 0.0, 1.0)
            {
                return action_rejected_at(index, format!("Error: {e}"));
            }
            if let Some(v) = it.max_voices
                && !(1..=128).contains(&v)
            {
                return action_rejected_at(
                    index,
                    format!("Error: max_voices must be in 1..=128, got {v}"),
                );
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            let id = it.instrument_id;
            let mut item_err: Option<String> = None;
            // An item that sets no fields still must name a real instrument,
            // otherwise it would report a phantom success. The field setters
            // below validate existence themselves, so only the all-omitted case
            // needs an explicit check.
            let sets_nothing = it.allocation_mode.is_none()
                && it.stealing_strategy.is_none()
                && it.unison_detune.is_none()
                && it.unison_spread.is_none()
                && it.max_voices.is_none();
            if sets_nothing && let Err(e) = self.bridge.get_instrument_info(id) {
                items.failed(format!("{}: {e}", id.as_u64()));
                continue;
            }
            if let Some(m) = &it.allocation_mode
                && let Err(e) = self.bridge.set_instrument_allocation_mode(id, m)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = &it.stealing_strategy
                && let Err(e) = self.bridge.set_instrument_stealing_strategy(id, s)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(d) = it.unison_detune
                && let Err(e) = self.bridge.set_instrument_unison_detune(id, d)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = it.unison_spread
                && let Err(e) = self.bridge.set_instrument_unison_spread(id, s)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(v) = it.max_voices
                && let Err(e) = self.bridge.set_instrument_max_voices(id, v)
            {
                item_err = Some(e.to_string());
            }
            match item_err {
                None => items.ok(),
                Some(e) => items.failed(format!("{}: {e}", id.as_u64())),
            }
        }
        items.reply("allocator configs updated")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set the MIDI channel (1-16) for one or more instruments.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_instrument_midi_channel(
        &self,
        params: Parameters<SetInstrumentMidiChannelParam>,
    ) -> CallToolResult {
        for (index, it) in params.0.items.iter().enumerate() {
            if let Err(e) = validate_midi_channel(it.channel) {
                return action_rejected_at(index, format!("Error: {e}"));
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_instrument_midi_channel(it.instrument_id, it.channel)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.instrument_id.as_u64())),
            }
        }
        items.reply("instrument MIDI channels set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set the category of one or more instruments (for visualization routing). Categories: Uncategorized, Drums, Bass, Pad, Lead, Arp, Keys, FX.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_instrument_category(
        &self,
        params: Parameters<SetInstrumentCategoryParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_instrument_category(it.instrument_id, &it.category)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.instrument_id.as_u64())),
            }
        }
        items.reply("instrument categories set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Disconnect one or more cables between module ports in one call. \
                       Each connection specifies from_module:from_port → to_module:to_port (same shape as connect).",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn disconnect(
        &self,
        params: Parameters<ConnectMultipleParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for c in &params.0.connections {
            match self.bridge.disconnect(
                params.0.instrument_id,
                &c.from_module,
                &c.from_port,
                &c.to_module,
                &c.to_port,
            ) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!(
                    "{}:{} → {}:{}: {e}",
                    c.from_module, c.from_port, c.to_module, c.to_port
                )),
            }
        }
        items.reply("cables disconnected")
    }

    // === Sequencer: Song ===

    #[tool(
        description = "Set one or more module parameters in one call. Each entry is {module_id, param_name, value}; value is a number in the parameter's native range, a boolean, or a string for a choice/enum or an address (e.g. a Mod Matrix slot_N_dest of 'spp-1.x').",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_parameter(
        &self,
        params: Parameters<SetParametersParam>,
    ) -> Result<Json<BatchResult>, String> {
        let p = params.0;
        for ps in &p.params {
            if let ParamValueInput::Number(n) = &ps.value
                && n.is_nan()
            {
                return Err(format!(
                    "Error: NaN is not a valid value for parameter '{}' on module '{}'",
                    ps.param_name, ps.module_id
                ));
            }
        }
        let param_sets: Vec<_> = p
            .params
            .into_iter()
            .map(|ps| crate::bridge::BridgeParamSet {
                module_id: ps.module_id,
                param_name: ps.param_name,
                value: match ps.value {
                    ParamValueInput::Number(n) => crate::bridge::BridgeParamValue::Number(n),
                    ParamValueInput::Bool(b) => crate::bridge::BridgeParamValue::Bool(b),
                    ParamValueInput::Choice(s) => crate::bridge::BridgeParamValue::Choice(s),
                },
            })
            .collect();
        match self.bridge.set_parameters(p.instrument_id, &param_sets) {
            Ok(result) => Ok(Json(result)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Replace an MSEG module's complete shape in one call. Pass 1-16 ordered \
        segments with time (0-60 seconds), target level (0-1), and curve (-1 logarithmic, 0 \
        linear, +1 exponential). The tool sets the active segment count automatically and \
        updates the descriptor-backed segN_time/level/curve parameters used by the GUI and \
        project persistence.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_mseg_segments(
        &self,
        params: Parameters<SetMsegSegmentsParam>,
    ) -> Result<Json<BatchResult>, String> {
        let p = params.0;
        if p.segments.is_empty() || p.segments.len() > 16 {
            return Err("Error: segments must contain between 1 and 16 items".to_string());
        }
        for (index, segment) in p.segments.iter().enumerate() {
            for (field, value, minimum, maximum) in [
                ("time", segment.time, 0.0, 60.0),
                ("level", segment.level, 0.0, 1.0),
                ("curve", segment.curve, -1.0, 1.0),
            ] {
                if let Err(error) = validate_range(field, value, minimum, maximum) {
                    return Err(format!("Error: segments[{index}].{error}"));
                }
            }
        }

        match self.bridge.get_module_info(p.instrument_id, &p.module_id) {
            Ok(module) if module.module_type.eq_ignore_ascii_case("MSEG") => {}
            Ok(module) => {
                return Err(format!(
                    "Error: module '{}' is {}, not MSEG",
                    p.module_id, module.module_type
                ));
            }
            Err(error) => return Err(format!("Error: {error}")),
        }

        let mut parameter_sets = Vec::with_capacity(1 + p.segments.len() * 3);
        parameter_sets.push(crate::bridge::BridgeParamSet {
            module_id: p.module_id.clone(),
            param_name: "segments".to_string(),
            value: crate::bridge::BridgeParamValue::Number(f64::from(
                u8::try_from(p.segments.len()).unwrap_or(16),
            )),
        });
        for (index, segment) in p.segments.into_iter().enumerate() {
            for (suffix, value) in [
                ("time", segment.time),
                ("level", segment.level),
                ("curve", segment.curve),
            ] {
                parameter_sets.push(crate::bridge::BridgeParamSet {
                    module_id: p.module_id.clone(),
                    param_name: format!("seg{index}_{suffix}"),
                    value: crate::bridge::BridgeParamValue::Number(f64::from(value)),
                });
            }
        }

        // One `BatchResult` over the `segments` count plus three parameters per
        // segment — the same payload `set_parameter` answers, since that is what
        // this tool expands into.
        match self.bridge.set_parameters(p.instrument_id, &parameter_sets) {
            Ok(result) => Ok(Json(result)),
            Err(error) => Err(format!("Error: {error}")),
        }
    }

    #[tool(
        output_schema = rmcp::handler::server::tool::schema_for_output::<Listing<BuildInstrumentResult>>(),
        description = "Build one or more complete instruments in one call. Each instrument has its own modules and connections; \
                       modules are referenced by 0-based array index in connections. Returns per-instrument results with instrument_id and module_ids. \
                       Port names must match the module's ports (osc/amp/out expose 'out'/'in'); the aliases 'output'→'out' and 'input'→'in' are also accepted. If every requested connection fails the whole call errors instead of returning a zero-connection instrument (a freshly-created instrument is rolled back, so no orphan is left). \
                       PARTIAL SUCCESS: an unknown parameter name or a single failed connection does NOT fail the call — the instrument is still created and each result carries `partial_success: true` plus an `errors` list. Always check `partial_success` (parameter names are module-specific; use get_module_type_info for the valid names) before treating the patch as complete. \
                       Example instrument: modules=[{module_type:'osc'},{module_type:'amp'},{module_type:'out'}], connections=[{from:0,from_port:'out',to:1,to_port:'in'},{from:1,from_port:'out',to:2,to_port:'in'}]",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn build_instrument(
        &self,
        params: Parameters<BuildInstrumentsParam>,
    ) -> CallToolResult {
        for (idx, inst) in params.0.instruments.iter().enumerate() {
            if let Err(e) = validate_build_instrument_fields(
                &inst.name,
                inst.midi_channel,
                inst.volume,
                inst.pan,
                &inst.modules,
                inst.connections.as_deref(),
            ) {
                return typed_failure(validation_err(McpBridgeError::Other(format!(
                    "instrument[{idx}]: {e}"
                ))));
            }
        }
        let specs: Vec<_> = params
            .0
            .instruments
            .into_iter()
            .map(|i| {
                convert_instrument_def(
                    i.instrument_id,
                    i.name,
                    i.midi_channel,
                    i.volume,
                    i.pan,
                    i.modules,
                    i.connections,
                )
            })
            .collect();
        match self.bridge.build_instruments(&specs) {
            Ok(results) => {
                // `partial_success` is the tool's own word for "the instrument
                // exists but the patch is incomplete". Stated as the call's
                // verdict too, so a `rollback: true` batch stops instead of
                // leaving a half-built instrument standing.
                let outcome = if results.iter().any(|r| r.partial_success) {
                    ToolOutcome::Partial
                } else {
                    ToolOutcome::Success
                };
                typed_reply(&Listing::from(results), outcome)
            }
            Err(e) => typed_failure(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = rmcp::handler::server::tool::schema_for_output::<ApplyExamplePatchResult>(),
        description = "Apply a named example patch directly to an instrument, creating all modules, parameters, and connections. \
                       If instrument_id is omitted, creates a new instrument. Much faster than load_example_patch (no GUI queue). \
                       Use list_example_patches to see available patches.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn apply_example_patch(
        &self,
        params: Parameters<ApplyExamplePatchParam>,
    ) -> CallToolResult {
        match self
            .bridge
            .apply_example_patch(params.0.instrument_id, &params.0.patch_name)
        {
            Ok(result) => {
                // Non-fatal notes mean the patch applied but not all of it.
                let outcome = if result.errors.is_empty() {
                    ToolOutcome::Success
                } else {
                    ToolOutcome::Partial
                };
                typed_reply(&result, outcome)
            }
            Err(e) => typed_failure(format!("Error: {e}")),
        }
    }

    // === Project management ===
}
