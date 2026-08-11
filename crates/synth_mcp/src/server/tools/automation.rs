//! automation MCP tool handlers.

use super::super::*;

#[tool_router(router = automation_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "Add automation points to a pattern. Each point specifies a parameter (e.g. Volume, Pan, FilterCutoff), position in beats, and a normalized value (0.0-1.0).",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn add_automation_points(
        &self,
        params: Parameters<AddAutomationPointsParam>,
    ) -> Result<Json<BatchResult>, String> {
        for pt in &params.0.points {
            if let Err(e) = validate_automation_point(pt) {
                return Err(validation_err(e));
            }
        }
        let p = params.0;
        let points: Vec<_> = p
            .points
            .into_iter()
            .map(|pt| crate::bridge::BridgeAutomationPointData {
                param: pt.effective_target(),
                instrument_id: pt.instrument_id.unwrap_or_default(),
                beat: pt.beat,
                value: pt.value,
                curve: pt.curve.unwrap_or_default(),
                curve_strength: pt.curve_strength,
            })
            .collect();
        match self.bridge.add_automation_points(p.pattern_id, &points) {
            Ok(result) => Ok(Json(result)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "List all automation lanes in a pattern with their target parameters and point counts.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_automation_lanes(
        &self,
        params: Parameters<PatternIdParam>,
    ) -> Result<Json<Listing<AutomationLaneInfo>>, String> {
        match self.bridge.list_automation_lanes(params.0.pattern_id) {
            Ok(lanes) => Ok(Json(lanes.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "List the valid automation targets for an instrument: every automatable per-module parameter in its graph (with ready-to-use 'module:<type>:<instance>:<param>' target strings, ranges, and units) plus the instrument-level macros. Use this to discover correct targets before adding automation points.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_instrument_automation_targets(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> Result<Json<Listing<AutomationTargetInfo>>, String> {
        match self
            .bridge
            .get_instrument_automation_targets(params.0.instrument_id)
        {
            Ok(targets) => Ok(Json(targets.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get all automation points for a specific parameter lane in a pattern.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_automation_points(
        &self,
        params: Parameters<GetAutomationPointsParam>,
    ) -> Result<Json<Listing<AutomationPointInfo>>, String> {
        let p = params.0;
        let target = p.target.to_target_string();
        match self.bridge.get_automation_points(
            p.pattern_id,
            &target,
            p.instrument_id.unwrap_or_default(),
        ) {
            Ok(points) => Ok(Json(points.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Remove automation points at specific beat positions from a lane.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn remove_automation_points(
        &self,
        params: Parameters<RemoveAutomationPointsParam>,
    ) -> Result<Json<BatchResult>, String> {
        let p = params.0;
        let target = p.target.to_target_string();
        match self.bridge.remove_automation_points(
            p.pattern_id,
            &target,
            p.instrument_id.unwrap_or_default(),
            &p.beats,
        ) {
            Ok(result) => Ok(Json(result)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Clear all automation points from one or more lanes (each a pattern + target + optional instrument ID).",
        annotations(destructive_hint = true, idempotent_hint = true)
    )]
    pub(crate) async fn clear_automation_lane(
        &self,
        params: Parameters<ClearAutomationLaneParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            let target = it.target.to_target_string();
            match self.bridge.clear_automation_lane(
                it.pattern_id,
                &target,
                it.instrument_id.unwrap_or_default(),
            ) {
                Ok(_count) => items.ok(),
                Err(e) => items.failed(format!("{} / {target}: {e}", it.pattern_id)),
            }
        }
        items.reply("automation lanes cleared")
    }

    #[tool(
        description = "Curve-aware automation simplifier: removes redundant breakpoints a lane's own \
                       interpolation already reproduces, so dense frame-derived lanes (e.g. imported SID \
                       automation with tens of thousands of points) shrink without audibly changing. \
                       DRY-RUN BY DEFAULT (`apply` = false): reports per-lane points_before/after, removed, \
                       and max_error (never exceeds `tolerance`) so you can preview before committing; set \
                       `apply` = true to rewrite. `tolerance` is a normalized 0..1 error bound. Step holds are \
                       preserved exactly (step points and their landing points are always kept, and no segment \
                       is simplified across a step boundary); Linear/Exponential/S-Curve segments are measured \
                       with their own interpolation. Scope with `pattern_id` and/or `target` (both omitted = \
                       every lane in every pattern). Unlike optimize_project (which only prunes unused objects) \
                       this rewrites lane point sets.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn simplify_automation(
        &self,
        params: Parameters<SimplifyAutomationParam>,
    ) -> Result<Json<SimplifyAutomationResult>, String> {
        let p = params.0;
        if let Err(e) = validate_range("tolerance", p.tolerance, 0.0, 1.0) {
            return Err(validation_err(e));
        }
        let target = p
            .target
            .as_ref()
            .map(AutomationTargetSelector::to_target_string);
        match self.bridge.simplify_automation(
            p.pattern_id,
            target.as_deref(),
            p.instrument_id.unwrap_or_default(),
            p.tolerance,
            p.apply,
        ) {
            Ok(r) => Ok(Json(r)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Scale one or more automation lanes' values around a pivot, in place (tick + curve preserved). \
                       Makes a filter sweep (or any lane) more or less dramatic without re-entering points. \
                       value' = clamp((value - pivot) * scale + pivot, 0..1).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn scale_automation_lane(
        &self,
        params: Parameters<ScaleAutomationLaneParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            let target = it.target.to_target_string();
            match self.bridge.transform_automation_lane(
                it.pattern_id,
                &target,
                it.instrument_id.unwrap_or_default(),
                it.scale,
                it.pivot.unwrap_or(0.5),
                0.0,
            ) {
                Ok(_count) => items.ok(),
                Err(e) => items.failed(format!("{} / {target}: {e}", it.pattern_id)),
            }
        }
        items.reply("automation lanes scaled")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Shift one or more automation lanes' values by a constant, in place (tick + curve preserved). \
                       value' = clamp(value + offset, 0..1).",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn offset_automation_lane(
        &self,
        params: Parameters<OffsetAutomationLaneParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            let target = it.target.to_target_string();
            match self.bridge.transform_automation_lane(
                it.pattern_id,
                &target,
                it.instrument_id.unwrap_or_default(),
                1.0,
                0.0,
                it.offset,
            ) {
                Ok(_count) => items.ok(),
                Err(e) => items.failed(format!("{} / {target}: {e}", it.pattern_id)),
            }
        }
        items.reply("automation lanes offset")
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Copy one or more automation lanes' points to another pattern/target (tick + curve preserved), \
                       optionally scaled/offset. Useful for reusing filter motion between similar voices. \
                       By default points are merged into the destination; set clear_destination to replace.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn copy_automation_lane(
        &self,
        params: Parameters<CopyAutomationLaneParam>,
    ) -> CallToolResult {
        let mut items = Mutations::new();
        for it in &params.0.items {
            let from_target = it.from_target.to_target_string();
            let to_target = it.to_target.to_target_string();
            match self.bridge.copy_automation_lane(
                it.from_pattern_id,
                &from_target,
                it.from_instrument_id.unwrap_or_default(),
                it.to_pattern_id,
                &to_target,
                it.to_instrument_id.unwrap_or_default(),
                it.scale.unwrap_or(1.0),
                it.offset.unwrap_or(0.0),
                it.clear_destination.unwrap_or(false),
            ) {
                Ok(_count) => items.ok(),
                Err(e) => items.failed(format!("{from_target} → {to_target}: {e}")),
            }
        }
        items.reply("automation lanes copied")
    }

    #[tool(
        description = "Project-wide automation overview: every lane in every pattern, grouped by 'instrument' \
                       (default), 'target', or 'pattern'. Read-only — use to audit where automation lives without \
                       querying each pattern.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_automation_summary(
        &self,
        params: Parameters<GetAutomationSummaryParam>,
    ) -> Result<Json<AutomationSummaryResult>, String> {
        let group_by = params
            .0
            .group_by
            .unwrap_or_else(|| "instrument".to_string());
        if !matches!(group_by.as_str(), "instrument" | "target" | "pattern") {
            return Err("Error: group_by must be 'instrument', 'target', or 'pattern'".to_string());
        }
        match self.bridge.get_automation_summary(&group_by) {
            Ok(result) => Ok(Json(result)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    // === Track control ===

    #[tool(
        output_schema = rmcp::handler::server::tool::schema_for_output::<RebuildInstrumentResult>(),
        description = "Rebuild an existing instrument's voice graph (new modules/params/connections) while keeping its pattern automation working. Instance counters are reset before the rebuild so modules are numbered deterministically (1.. per type, in add order) — wherever the new module set matches the old, the module ids line up and their automation lanes stay valid automatically. Lanes whose target module no longer exists are reported as `orphaned_lanes`; set `drop_orphaned: true` to delete them, otherwise they are left dangling. Returns the rebuilt module ids, preserved-lane count, and the orphaned lanes. Use this instead of build_instrument when the instrument already has automation you don't want to lose. Note: matching is by module type + add-order, so reordering same-type modules can still re-point a lane.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn rebuild_instrument_preserve_automation(
        &self,
        params: Parameters<RebuildInstrumentParam>,
    ) -> CallToolResult {
        let p = params.0;
        if let Err(e) = validate_build_instrument_fields(
            &p.name,
            p.midi_channel,
            p.volume,
            p.pan,
            &p.modules,
            p.connections.as_deref(),
        ) {
            return typed_failure(validation_err(e));
        }
        let drop_orphaned = p.drop_orphaned.unwrap_or(false);
        let spec = convert_instrument_def(
            Some(p.instrument_id),
            p.name,
            p.midi_channel,
            p.volume,
            p.pan,
            p.modules,
            p.connections,
        );
        match self
            .bridge
            .rebuild_instrument_preserve_automation(&spec, drop_orphaned)
        {
            Ok(result) => {
                // Errors here are non-fatal by design — the instrument is rebuilt
                // and its lanes reported — but they mean the rebuild did not do
                // all of what was asked.
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
}
