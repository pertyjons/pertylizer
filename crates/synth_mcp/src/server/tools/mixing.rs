//! mixing MCP tool handlers.

use super::super::*;

#[tool_router(router = mixing_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        output_schema = action_output_schema(),
        description = "Set or clear a track's free-text description (its role, e.g. \"kick layer\", \
        \"sidechain source\"). Pass \"\" to clear. Surfaces in list_tracks.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_description(
        &self,
        params: Parameters<SetTrackDescriptionParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_track_description(it.track_id, &it.description)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.track_id)),
            }
        }
        items.reply("track descriptions set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set the display color of one or more tracks from a \"#RRGGBB\" / \"#RRGGBBAA\" hex string \
        (alpha ignored). Paints the arrangement so it is visually scannable. Surfaces in list_tracks.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_color(
        &self,
        params: Parameters<SetTrackColorParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.set_track_color(it.track_id, &it.color) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.track_id)),
            }
        }
        items.reply("track colors set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set mixer state on one or more tracks in a single call. Each item carries \
        a track_id plus any of volume (0.0=silent, 1.0=full, up to 2.0 for boost), pan \
        (-1.0=left..1.0=right), muted, and solo. Omitted fields are left unchanged. When any track \
        is soloed, only soloed tracks sound. To (un)assign a track's instrument, use set_track_instrument.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_mixer(
        &self,
        params: Parameters<SetTrackMixerParam>,
    ) -> CallToolResult {
        for (index, it) in params.0.items.iter().enumerate() {
            if let Some(v) = it.volume
                && let Err(e) = validate_range("volume", v.as_f32(), 0.0, 2.0)
            {
                return action_rejected_at(index, validation_err(e));
            }
            if let Some(p) = it.pan
                && let Err(e) = validate_range("pan", p.as_f32(), -1.0, 1.0)
            {
                return action_rejected_at(index, validation_err(e));
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            let id = it.track_id;
            let mut item_err: Option<String> = None;
            if let Some(v) = it.volume
                && let Err(e) = self.bridge.set_track_volume(id, v)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(p) = it.pan
                && let Err(e) = self.bridge.set_track_pan(id, p)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(m) = it.muted
                && let Err(e) = self.bridge.set_track_mute(id, m)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = it.solo
                && let Err(e) = self.bridge.set_track_solo(id, s)
            {
                item_err = Some(e.to_string());
            }
            match item_err {
                None => items.ok(),
                Some(e) => items.failed(format!("{id}: {e}")),
            }
        }
        items.reply("track mixer updates applied")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Assign (or unassign) the instrument driving one or more tracks. Each item's \
        instrument_id is required: a number assigns that instrument, null unassigns (the track plays nothing).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_instrument(
        &self,
        params: Parameters<SetTrackInstrumentParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_track_instrument(it.track_id, it.instrument_id)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.track_id)),
            }
        }
        items.reply("track instruments set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Rename one or more tracks. The name is shown in the sequencer track headers.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn rename_track(
        &self,
        params: Parameters<RenameTrackParam>,
    ) -> CallToolResult {
        for (index, it) in params.0.items.iter().enumerate() {
            if let Err(e) = validate_name("track", &it.name) {
                return action_rejected_at(index, validation_err(e));
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.rename_track(it.track_id, &it.name) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.track_id)),
            }
        }
        items.reply("tracks renamed")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Delete one or more tracks and all their placements from the arrangement.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_track(
        &self,
        params: Parameters<DeleteTracksParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for id in &params.0.track_ids {
            match self.bridge.delete_track(*id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{id}: {e}")),
            }
        }
        items.reply("tracks deleted")
    }

    // === Return busses (effect sends) ===

    #[tool(
        description = "List all return busses (effect-send destinations) with their fader settings.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_return_busses(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<Listing<ReturnBusInfo>>, String> {
        match self.bridge.list_return_busses() {
            Ok(busses) => Ok(Json(busses.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Create one or more return busses (each a sub-mix with its own effect chain, fed by track sends). Returns the assigned IDs.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn create_return_bus(
        &self,
        params: Parameters<CreateReturnBusParam>,
    ) -> CallToolResult {
        for (index, name) in params.0.names.iter().enumerate() {
            if let Err(e) = validate_name("return bus", name) {
                return action_rejected_at(index, validation_err(e));
            }
        }
        let mut items = Mutations::new();
        for name in &params.0.names {
            match self.bridge.create_return_bus(name) {
                // The id, not `id 'name'` — the name is `names[index]`.
                Ok(id) => items.named(id.to_string()),
                Err(e) => items.failed(format!("'{name}': {e}")),
            }
        }
        items.reply("return busses created")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Delete one or more return busses and remove every track send that targeted them.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_return_bus(
        &self,
        params: Parameters<DeleteReturnBusesParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for id in &params.0.return_ids {
            match self.bridge.delete_return_bus(*id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{id}: {e}")),
            }
        }
        items.reply("return busses deleted")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set mixer state on one or more return busses in a single call. Each item \
        carries a return_id plus any of volume (0.0=silent..1.0=full), pan (-1.0=left..1.0=right), \
        muted, and solo. Omitted fields are left unchanged. When any return is soloed, only soloed \
        returns reach the master mix (bus-to-bus routing still flows).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_bus_mixer(
        &self,
        params: Parameters<SetReturnBusMixerParam>,
    ) -> CallToolResult {
        for (index, it) in params.0.items.iter().enumerate() {
            // Return-bus volume is stored as NormalizedValue (clamps to [0, 1]).
            if let Some(v) = it.volume
                && let Err(e) = validate_range("volume", v.as_f32(), 0.0, 1.0)
            {
                return action_rejected_at(index, validation_err(e));
            }
            if let Some(p) = it.pan
                && let Err(e) = validate_range("pan", p.as_f32(), -1.0, 1.0)
            {
                return action_rejected_at(index, validation_err(e));
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            let id = it.return_id;
            let mut item_err: Option<String> = None;
            if let Some(v) = it.volume
                && let Err(e) = self.bridge.set_return_bus_volume(id, v)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(p) = it.pan
                && let Err(e) = self.bridge.set_return_bus_pan(id, p)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(m) = it.muted
                && let Err(e) = self.bridge.set_return_bus_mute(id, m)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = it.solo
                && let Err(e) = self.bridge.set_return_bus_solo(id, s)
            {
                item_err = Some(e.to_string());
            }
            match item_err {
                None => items.ok(),
                Some(e) => items.failed(format!("{id}: {e}")),
            }
        }
        items.reply("return bus mixer updates applied")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set the display color of one or more return busses from a \"#RRGGBB\" hex string.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_bus_color(
        &self,
        params: Parameters<SetReturnBusColorParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.set_return_bus_color(it.return_id, &it.color) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.return_id)),
            }
        }
        items.reply("return bus colors set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set the free-text description / intent (\"\" clears it) on one or more return busses. Never affects audio.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_bus_description(
        &self,
        params: Parameters<SetReturnBusDescriptionParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_return_bus_description(it.return_id, &it.description)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.return_id)),
            }
        }
        items.reply("return bus descriptions set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Rename one or more return busses.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn rename_return_bus(
        &self,
        params: Parameters<RenameReturnBusParam>,
    ) -> CallToolResult {
        for (index, it) in params.0.items.iter().enumerate() {
            if let Err(e) = validate_name("return bus", &it.name) {
                return action_rejected_at(index, validation_err(e));
            }
        }
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self.bridge.rename_return_bus(it.return_id, &it.name) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.return_id)),
            }
        }
        items.reply("return busses renamed")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Add or update one or more track effect sends to return busses (upsert by track+return target). pre_fader taps before the channel fader.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_send(
        &self,
        params: Parameters<SetTrackSendParam>,
    ) -> CallToolResult {
        // The raw f32 is validated *before* it becomes a NormalizedValue (which
        // would clamp to [0, 1] and hide an over-unity request); sends max out at
        // unity, so anything above is a hard error rather than a silent clamp.
        for (index, s) in params.0.sends.iter().enumerate() {
            if let Err(e) = validate_range("level", s.level, 0.0, 1.0) {
                return action_rejected_at(index, validation_err(e));
            }
        }
        let mut items = Mutations::new();
        for s in &params.0.sends {
            match self.bridge.set_track_send(
                s.track_id,
                s.return_id,
                NormalizedValue::new(s.level),
                s.pre_fader,
                s.enabled,
            ) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!(
                    "track {} → return {}: {e}",
                    s.track_id, s.return_id
                )),
            }
        }
        items.reply("track sends set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Remove one or more track effect sends to return busses.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_track_send(
        &self,
        params: Parameters<RemoveTrackSendsParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for s in &params.0.sends {
            match self.bridge.remove_track_send(s.track_id, s.return_id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!(
                    "track {} → return {}: {e}",
                    s.track_id, s.return_id
                )),
            }
        }
        items.reply("track sends removed")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Add or update one or more bus-to-bus sends: route one return bus's output into another (e.g. a delay return into a reverb return). Upsert by from+to target; each is rejected if it would create a routing cycle.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_send(
        &self,
        params: Parameters<SetReturnSendParam>,
    ) -> CallToolResult {
        // Validate the raw f32 before it becomes a NormalizedValue (see
        // set_track_send): over-unity sends are rejected, not silently clamped.
        for (index, s) in params.0.sends.iter().enumerate() {
            if let Err(e) = validate_range("level", s.level, 0.0, 1.0) {
                return action_rejected_at(index, validation_err(e));
            }
        }
        let mut items = Mutations::new();
        for s in &params.0.sends {
            match self.bridge.set_return_send(
                s.from_id,
                s.to_id,
                NormalizedValue::new(s.level),
                s.enabled,
            ) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("return {} → return {}: {e}", s.from_id, s.to_id)),
            }
        }
        items.reply("return sends set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Remove one or more bus-to-bus sends from one return bus into another.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_return_send(
        &self,
        params: Parameters<RemoveReturnSendsParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for s in &params.0.sends {
            match self.bridge.remove_return_send(s.from_id, s.to_id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("return {} → return {}: {e}", s.from_id, s.to_id)),
            }
        }
        items.reply("return sends removed")
    }

    // === Return-bus insert effects ===

    #[tool(
        output_schema = action_output_schema(),
        description = "Add one or more insert effects to a return bus's effect chain, in order (e.g. put a reverb on a Reverb return). Each effect_type is a module-type key like 'rev', 'delay', 'chorus', 'eq', 'compressor'. Returns the new effects' module-ids (e.g. 'rev-1').",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_return_effect(
        &self,
        params: Parameters<AddReturnEffectsParam>,
    ) -> CallToolResult {
        let p = params.0;
        let mut items = Mutations::new();
        for effect_type in &p.effect_types {
            match self.bridge.add_return_effect(p.return_id, effect_type) {
                Ok(module_id) => items.named(module_id),
                Err(e) => items.failed(format!("{effect_type}: {e}")),
            }
        }
        items.reply(&format!("effects added to return bus {}", p.return_id))
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Remove one or more insert effects from a return bus's effect chain by their module-ids.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_return_effect(
        &self,
        params: Parameters<RemoveReturnEffectsParam>,
    ) -> CallToolResult {
        let p = params.0;
        let mut items = Mutations::new();
        for module_id in &p.module_ids {
            match self.bridge.remove_return_effect(p.return_id, module_id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{module_id}: {e}")),
            }
        }
        items.reply(&format!("effects removed from return bus {}", p.return_id))
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set parameters on return-bus insert effects (one or many). Each item gives return_id, module_id, param_name (type_id or display name) and value (number, boolean, or choice string). Use list_return_busses to discover effects and their parameters.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_effect_parameter(
        &self,
        params: Parameters<SetReturnEffectParameterParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in params.0.params {
            let value = match param_value_to_bridge(it.value) {
                Ok(v) => v,
                Err(e) => {
                    items.failed(format!("{}/{}: {e}", it.return_id, it.module_id));
                    continue;
                }
            };
            match self.bridge.set_return_effect_parameter(
                it.return_id,
                &it.module_id,
                &it.param_name,
                value,
            ) {
                Ok(_) => items.ok(),
                Err(e) => items.failed(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        items.reply("return effect parameters set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Enable or bypass one or more return-bus insert effects (enabled = false bypasses).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_effect_enabled(
        &self,
        params: Parameters<SetReturnEffectEnabledParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_return_effect_enabled(it.return_id, &it.module_id, it.enabled)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        items.reply("return effect toggles applied")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Move one or more return-bus insert effects up or down within their effect chain (direction: 'up' = earlier, 'down' = later). Moves are applied in array order.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn reorder_return_effect(
        &self,
        params: Parameters<ReorderReturnEffectParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            let direction = match it.direction.trim().to_ascii_lowercase().as_str() {
                "up" => crate::bridge::ReturnEffectMove::Up,
                "down" => crate::bridge::ReturnEffectMove::Down,
                other => {
                    items.failed(format!(
                        "{}/{}: invalid direction '{other}', expected 'up' or 'down'",
                        it.return_id, it.module_id
                    ));
                    continue;
                }
            };
            match self
                .bridge
                .reorder_return_effect(it.return_id, &it.module_id, direction)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        items.reply("return effects reordered")
    }

    // === Master bus ===

    // A read, not an action, so it does not take the `ActionResult` envelope —
    // that would advertise `ok_count` / `details` / `errors` for a value fetch
    // and leave the volume itself reachable only by parsing `message`. It gets
    // the payload type of its own that this comment used to defer.
    #[tool(
        description = "Read the master output volume (0.0 = silent, 1.0 = unity).",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_master_volume(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<MasterVolumeInfo>, String> {
        match self.bridge.get_master_volume() {
            Ok(volume) => Ok(Json(MasterVolumeInfo { volume })),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set the master output volume (0.0 = silent, 1.0 = unity).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_master_volume(
        &self,
        params: Parameters<SetMasterVolumeParam>,
    ) -> CallToolResult {
        if let Err(e) = validate_range("volume", params.0.volume.as_f32(), 0.0, 4.0) {
            return action_rejected(validation_err(e));
        }
        match self.bridge.set_master_volume(params.0.volume) {
            Ok(()) => action_ok(format!("OK: master volume set to {}", params.0.volume)),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "List the master-bus insert effects (the final effect chain applied to the full mix) in processing order, with parameters.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_master_effects(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<Listing<ReturnEffectInfo>>, String> {
        match self.bridge.list_master_effects() {
            Ok(effects) => Ok(Json(effects.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Add one or more insert effects to the master-bus effect chain, in order (applied to the full mix, e.g. a limiter or EQ on the master). Each effect_type is a module-type key. Returns the new module-ids.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_master_effect(
        &self,
        params: Parameters<AddMasterEffectsParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for effect_type in &params.0.effect_types {
            match self.bridge.add_master_effect(effect_type) {
                Ok(module_id) => items.named(module_id),
                Err(e) => items.failed(format!("{effect_type}: {e}")),
            }
        }
        items.reply("effects added to master bus")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Remove one or more insert effects from the master-bus effect chain by their module-ids.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_master_effect(
        &self,
        params: Parameters<RemoveMasterEffectsParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for module_id in &params.0.module_ids {
            match self.bridge.remove_master_effect(module_id) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{module_id}: {e}")),
            }
        }
        items.reply("effects removed from master bus")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Set a parameter on a master-bus insert effect. param_name is the parameter's type_id or display name; value is a number, boolean, or choice string. Use list_master_effects to discover effects and parameters.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_master_effect_parameter(
        &self,
        params: Parameters<SetMasterEffectParameterParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in params.0.params {
            let value = match param_value_to_bridge(it.value) {
                Ok(v) => v,
                Err(e) => {
                    items.failed(format!("{}: {e}", it.module_id));
                    continue;
                }
            };
            match self
                .bridge
                .set_master_effect_parameter(&it.module_id, &it.param_name, value)
            {
                Ok(_) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.module_id)),
            }
        }
        items.reply("master effect parameters set")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Enable or bypass one or more master-bus insert effects (enabled = false bypasses).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_master_effect_enabled(
        &self,
        params: Parameters<SetMasterEffectEnabledParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_master_effect_enabled(&it.module_id, it.enabled)
            {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.module_id)),
            }
        }
        items.reply("master effect toggles applied")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Move one or more master-bus insert effects up or down within the chain (direction: 'up' = earlier, 'down' = later). Moves are applied in array order.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn reorder_master_effect(
        &self,
        params: Parameters<ReorderMasterEffectParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            let direction = match it.direction.trim().to_ascii_lowercase().as_str() {
                "up" => crate::bridge::ReturnEffectMove::Up,
                "down" => crate::bridge::ReturnEffectMove::Down,
                other => {
                    items.failed(format!(
                        "{}: invalid direction '{other}', expected 'up' or 'down'",
                        it.module_id
                    ));
                    continue;
                }
            };
            match self.bridge.reorder_master_effect(&it.module_id, direction) {
                Ok(()) => items.ok(),
                Err(e) => items.failed(format!("{}: {e}", it.module_id)),
            }
        }
        items.reply("master effects reordered")
    }

    // === Pattern management ===
}
