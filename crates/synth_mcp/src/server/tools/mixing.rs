//! mixing MCP tool handlers.

use super::super::*;

#[tool_router(router = mixing_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "Set or clear a track's free-text description (its role, e.g. \"kick layer\", \
        \"sidechain source\"). Pass \"\" to clear. Surfaces in list_tracks.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_description(
        &self,
        params: Parameters<SetTrackDescriptionParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_track_description(it.track_id, &it.description)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.track_id)),
            }
        }
        batch_msg(ok_count, "track descriptions set", &[], &errors)
    }

    #[tool(
        description = "Set the display color of one or more tracks from a \"#RRGGBB\" / \"#RRGGBBAA\" hex string \
        (alpha ignored). Paints the arrangement so it is visually scannable. Surfaces in list_tracks.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_color(&self, params: Parameters<SetTrackColorParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_track_color(it.track_id, &it.color) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.track_id)),
            }
        }
        batch_msg(ok_count, "track colors set", &[], &errors)
    }

    #[tool(
        description = "Set mixer state on one or more tracks in a single call. Each item carries \
        a track_id plus any of volume (0.0=silent, 1.0=full, up to 2.0 for boost), pan \
        (-1.0=left..1.0=right), muted, and solo. Omitted fields are left unchanged. When any track \
        is soloed, only soloed tracks sound. To (un)assign a track's instrument, use set_track_instrument.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_mixer(&self, params: Parameters<SetTrackMixerParam>) -> String {
        for it in &params.0.items {
            if let Some(v) = it.volume
                && let Err(e) = validate_range("volume", v.as_f32(), 0.0, 2.0)
            {
                return validation_err(e);
            }
            if let Some(p) = it.pan
                && let Err(e) = validate_range("pan", p.as_f32(), -1.0, 1.0)
            {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
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
                None => ok_count += 1,
                Some(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "track mixer updates applied", &[], &errors)
    }

    #[tool(
        description = "Assign (or unassign) the instrument driving one or more tracks. Each item's \
        instrument_id is required: a number assigns that instrument, null unassigns (the track plays nothing).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_instrument(
        &self,
        params: Parameters<SetTrackInstrumentParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_track_instrument(it.track_id, it.instrument_id)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.track_id)),
            }
        }
        batch_msg(ok_count, "track instruments set", &[], &errors)
    }

    #[tool(
        description = "Rename one or more tracks. The name is shown in the sequencer track headers.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn rename_track(&self, params: Parameters<RenameTrackParam>) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_name("track", &it.name) {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_track(it.track_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.track_id)),
            }
        }
        batch_msg(ok_count, "tracks renamed", &[], &errors)
    }

    #[tool(
        description = "Delete one or more tracks and all their placements from the arrangement.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_track(&self, params: Parameters<DeleteTracksParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.track_ids {
            match self.bridge.delete_track(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "tracks deleted", &[], &errors)
    }

    // === Return busses (effect sends) ===

    #[tool(
        description = "List all return busses (effect-send destinations) with their fader settings.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_return_busses(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_return_busses() {
            Ok(busses) => to_json(&busses),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create one or more return busses (each a sub-mix with its own effect chain, fed by track sends). Returns the assigned IDs.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn create_return_bus(
        &self,
        params: Parameters<CreateReturnBusParam>,
    ) -> String {
        for name in &params.0.names {
            if let Err(e) = validate_name("return bus", name) {
                return validation_err(e);
            }
        }
        let mut ids = Vec::new();
        let mut errors = Vec::new();
        for name in &params.0.names {
            match self.bridge.create_return_bus(name) {
                Ok(id) => ids.push(format!("{id} '{name}'")),
                Err(e) => errors.push(format!("'{name}': {e}")),
            }
        }
        batch_msg(ids.len(), "return busses created", &ids, &errors)
    }

    #[tool(
        description = "Delete one or more return busses and remove every track send that targeted them.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn delete_return_bus(
        &self,
        params: Parameters<DeleteReturnBusesParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.return_ids {
            match self.bridge.delete_return_bus(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "return busses deleted", &[], &errors)
    }

    #[tool(
        description = "Set mixer state on one or more return busses in a single call. Each item \
        carries a return_id plus any of volume (0.0=silent..1.0=full), pan (-1.0=left..1.0=right), \
        muted, and solo. Omitted fields are left unchanged. When any return is soloed, only soloed \
        returns reach the master mix (bus-to-bus routing still flows).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_bus_mixer(
        &self,
        params: Parameters<SetReturnBusMixerParam>,
    ) -> String {
        for it in &params.0.items {
            // Return-bus volume is stored as NormalizedValue (clamps to [0, 1]).
            if let Some(v) = it.volume
                && let Err(e) = validate_range("volume", v.as_f32(), 0.0, 1.0)
            {
                return validation_err(e);
            }
            if let Some(p) = it.pan
                && let Err(e) = validate_range("pan", p.as_f32(), -1.0, 1.0)
            {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
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
                None => ok_count += 1,
                Some(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "return bus mixer updates applied", &[], &errors)
    }

    #[tool(
        description = "Set the display color of one or more return busses from a \"#RRGGBB\" hex string.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_bus_color(
        &self,
        params: Parameters<SetReturnBusColorParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_return_bus_color(it.return_id, &it.color) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.return_id)),
            }
        }
        batch_msg(ok_count, "return bus colors set", &[], &errors)
    }

    #[tool(
        description = "Set the free-text description / intent (\"\" clears it) on one or more return busses. Never affects audio.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_bus_description(
        &self,
        params: Parameters<SetReturnBusDescriptionParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_return_bus_description(it.return_id, &it.description)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.return_id)),
            }
        }
        batch_msg(ok_count, "return bus descriptions set", &[], &errors)
    }

    #[tool(
        description = "Rename one or more return busses.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn rename_return_bus(
        &self,
        params: Parameters<RenameReturnBusParam>,
    ) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_name("return bus", &it.name) {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_return_bus(it.return_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.return_id)),
            }
        }
        batch_msg(ok_count, "return busses renamed", &[], &errors)
    }

    #[tool(
        description = "Add or update one or more track effect sends to return busses (upsert by track+return target). pre_fader taps before the channel fader.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_track_send(&self, params: Parameters<SetTrackSendParam>) -> String {
        // The raw f32 is validated *before* it becomes a NormalizedValue (which
        // would clamp to [0, 1] and hide an over-unity request); sends max out at
        // unity, so anything above is a hard error rather than a silent clamp.
        for s in &params.0.sends {
            if let Err(e) = validate_range("level", s.level, 0.0, 1.0) {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for s in &params.0.sends {
            match self.bridge.set_track_send(
                s.track_id,
                s.return_id,
                NormalizedValue::new(s.level),
                s.pre_fader,
                s.enabled,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "track {} → return {}: {e}",
                    s.track_id, s.return_id
                )),
            }
        }
        batch_msg(ok_count, "track sends set", &[], &errors)
    }

    #[tool(
        description = "Remove one or more track effect sends to return busses.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_track_send(
        &self,
        params: Parameters<RemoveTrackSendsParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for s in &params.0.sends {
            match self.bridge.remove_track_send(s.track_id, s.return_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "track {} → return {}: {e}",
                    s.track_id, s.return_id
                )),
            }
        }
        batch_msg(ok_count, "track sends removed", &[], &errors)
    }

    #[tool(
        description = "Add or update one or more bus-to-bus sends: route one return bus's output into another (e.g. a delay return into a reverb return). Upsert by from+to target; each is rejected if it would create a routing cycle.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_send(&self, params: Parameters<SetReturnSendParam>) -> String {
        // Validate the raw f32 before it becomes a NormalizedValue (see
        // set_track_send): over-unity sends are rejected, not silently clamped.
        for s in &params.0.sends {
            if let Err(e) = validate_range("level", s.level, 0.0, 1.0) {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for s in &params.0.sends {
            match self.bridge.set_return_send(
                s.from_id,
                s.to_id,
                NormalizedValue::new(s.level),
                s.enabled,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("return {} → return {}: {e}", s.from_id, s.to_id)),
            }
        }
        batch_msg(ok_count, "return sends set", &[], &errors)
    }

    #[tool(
        description = "Remove one or more bus-to-bus sends from one return bus into another.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_return_send(
        &self,
        params: Parameters<RemoveReturnSendsParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for s in &params.0.sends {
            match self.bridge.remove_return_send(s.from_id, s.to_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("return {} → return {}: {e}", s.from_id, s.to_id)),
            }
        }
        batch_msg(ok_count, "return sends removed", &[], &errors)
    }

    // === Return-bus insert effects ===

    #[tool(
        description = "Add one or more insert effects to a return bus's effect chain, in order (e.g. put a reverb on a Reverb return). Each effect_type is a module-type key like 'rev', 'delay', 'chorus', 'eq', 'compressor'. Returns the new effects' module-ids (e.g. 'rev-1').",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_return_effect(
        &self,
        params: Parameters<AddReturnEffectsParam>,
    ) -> String {
        let p = params.0;
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for effect_type in &p.effect_types {
            match self.bridge.add_return_effect(p.return_id, effect_type) {
                Ok(module_id) => oks.push(module_id),
                Err(e) => errors.push(format!("{effect_type}: {e}")),
            }
        }
        batch_msg(
            oks.len(),
            &format!("effects added to return bus {}", p.return_id),
            &oks,
            &errors,
        )
    }

    #[tool(
        description = "Remove one or more insert effects from a return bus's effect chain by their module-ids.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_return_effect(
        &self,
        params: Parameters<RemoveReturnEffectsParam>,
    ) -> String {
        let p = params.0;
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for module_id in &p.module_ids {
            match self.bridge.remove_return_effect(p.return_id, module_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{module_id}: {e}")),
            }
        }
        batch_msg(
            ok_count,
            &format!("effects removed from return bus {}", p.return_id),
            &[],
            &errors,
        )
    }

    #[tool(
        description = "Set parameters on return-bus insert effects (one or many). Each item gives return_id, module_id, param_name (type_id or display name) and value (number, boolean, or choice string). Use list_return_busses to discover effects and their parameters.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_effect_parameter(
        &self,
        params: Parameters<SetReturnEffectParameterParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.params {
            let value = match param_value_to_bridge(it.value) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{}/{}: {e}", it.return_id, it.module_id));
                    continue;
                }
            };
            match self.bridge.set_return_effect_parameter(
                it.return_id,
                &it.module_id,
                &it.param_name,
                value,
            ) {
                Ok(_info) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        batch_msg(ok_count, "return effect parameters set", &[], &errors)
    }

    #[tool(
        description = "Enable or bypass one or more return-bus insert effects (enabled = false bypasses).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_return_effect_enabled(
        &self,
        params: Parameters<SetReturnEffectEnabledParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_return_effect_enabled(it.return_id, &it.module_id, it.enabled)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        batch_msg(ok_count, "return effect toggles applied", &[], &errors)
    }

    #[tool(
        description = "Move one or more return-bus insert effects up or down within their effect chain (direction: 'up' = earlier, 'down' = later). Moves are applied in array order.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn reorder_return_effect(
        &self,
        params: Parameters<ReorderReturnEffectParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            let direction = match it.direction.trim().to_ascii_lowercase().as_str() {
                "up" => crate::bridge::ReturnEffectMove::Up,
                "down" => crate::bridge::ReturnEffectMove::Down,
                other => {
                    errors.push(format!(
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
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        batch_msg(ok_count, "return effects reordered", &[], &errors)
    }

    // === Master bus ===

    #[tool(
        description = "Read the master output volume (0.0 = silent, 1.0 = unity).",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_master_volume(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_master_volume() {
            Ok(v) => format!("{v}"),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the master output volume (0.0 = silent, 1.0 = unity).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_master_volume(
        &self,
        params: Parameters<SetMasterVolumeParam>,
    ) -> String {
        if let Err(e) = validate_range("volume", params.0.volume.as_f32(), 0.0, 4.0) {
            return validation_err(e);
        }
        match self.bridge.set_master_volume(params.0.volume) {
            Ok(()) => format!("OK: master volume set to {}", params.0.volume),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List the master-bus insert effects (the final effect chain applied to the full mix) in processing order, with parameters.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_master_effects(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_master_effects() {
            Ok(effects) => to_json(&effects),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add one or more insert effects to the master-bus effect chain, in order (applied to the full mix, e.g. a limiter or EQ on the master). Each effect_type is a module-type key. Returns the new module-ids.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_master_effect(
        &self,
        params: Parameters<AddMasterEffectsParam>,
    ) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for effect_type in &params.0.effect_types {
            match self.bridge.add_master_effect(effect_type) {
                Ok(module_id) => oks.push(module_id),
                Err(e) => errors.push(format!("{effect_type}: {e}")),
            }
        }
        batch_msg(oks.len(), "effects added to master bus", &oks, &errors)
    }

    #[tool(
        description = "Remove one or more insert effects from the master-bus effect chain by their module-ids.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_master_effect(
        &self,
        params: Parameters<RemoveMasterEffectsParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for module_id in &params.0.module_ids {
            match self.bridge.remove_master_effect(module_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{module_id}: {e}")),
            }
        }
        batch_msg(ok_count, "effects removed from master bus", &[], &errors)
    }

    #[tool(
        description = "Set a parameter on a master-bus insert effect. param_name is the parameter's type_id or display name; value is a number, boolean, or choice string. Use list_master_effects to discover effects and parameters.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_master_effect_parameter(
        &self,
        params: Parameters<SetMasterEffectParameterParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.params {
            let value = match param_value_to_bridge(it.value) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{}: {e}", it.module_id));
                    continue;
                }
            };
            match self
                .bridge
                .set_master_effect_parameter(&it.module_id, &it.param_name, value)
            {
                Ok(_info) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.module_id)),
            }
        }
        batch_msg(ok_count, "master effect parameters set", &[], &errors)
    }

    #[tool(
        description = "Enable or bypass one or more master-bus insert effects (enabled = false bypasses).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_master_effect_enabled(
        &self,
        params: Parameters<SetMasterEffectEnabledParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_master_effect_enabled(&it.module_id, it.enabled)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.module_id)),
            }
        }
        batch_msg(ok_count, "master effect toggles applied", &[], &errors)
    }

    #[tool(
        description = "Move one or more master-bus insert effects up or down within the chain (direction: 'up' = earlier, 'down' = later). Moves are applied in array order.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn reorder_master_effect(
        &self,
        params: Parameters<ReorderMasterEffectParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            let direction = match it.direction.trim().to_ascii_lowercase().as_str() {
                "up" => crate::bridge::ReturnEffectMove::Up,
                "down" => crate::bridge::ReturnEffectMove::Down,
                other => {
                    errors.push(format!(
                        "{}: invalid direction '{other}', expected 'up' or 'down'",
                        it.module_id
                    ));
                    continue;
                }
            };
            match self.bridge.reorder_master_effect(&it.module_id, direction) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.module_id)),
            }
        }
        batch_msg(ok_count, "master effects reordered", &[], &errors)
    }

    // === Pattern management ===
}
