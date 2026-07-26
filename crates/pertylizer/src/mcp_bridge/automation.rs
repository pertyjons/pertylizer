macro_rules! synth_bridge_automation {
    () => {
    fn add_automation_points(
        &self,
        pattern_id: PatternId,
        points: &[BridgeAutomationPointData],
    ) -> Result<BatchResult, McpBridgeError> {
        use synth_sequencer::{AutomationPoint, PatternTick};

        // Pre-fetch each referenced instrument's live module ids (before taking
        // the song lock) so Module targets can be validated against the real
        // graph without re-querying per point.
        let module_cache = self.module_id_cache(points.iter().map(|pt| pt.instrument_id));
        let targets: Vec<Result<synth_sequencer::AutomationTarget, McpBridgeError>> = points
            .iter()
            .map(|pt| {
                let valid = module_cache
                    .get(&pt.instrument_id)
                    .map_or(&[][..], Vec::as_slice);
                let target =
                    self.build_live_automation_target(&pt.param, pt.instrument_id, valid)?;
                self.validate_automation_target_owner(&target)?;
                Ok(target)
            })
            .collect();

        let mut song_w = self.shared.song.write();
        let pat_id = pattern_id;
        let pattern = song_w
            .pattern_mut(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let mut succeeded = 0usize;
        let mut items = Vec::new();
        let total = points.len();

        for (i, (pt, target)) in points.iter().zip(targets).enumerate() {
            // Share the same target builder as the read/edit/clear tools so the
            // `module:<type>:<instance>:<param>` syntax (validated against the
            // automatable allowlist + instrument graph) can also *create* lanes,
            // not just plain instrument params.
            let target = match target {
                Ok(t) => t,
                Err(e) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            };

            let tick = PatternTick(beats_to_ticks(pt.beat));
            let curve = curve_from_kind(pt.curve, pt.curve_strength);
            let lane = pattern.get_or_create_automation(target);
            lane.add_point(
                AutomationPoint::new(tick, NormalizedValue::new(pt.value)).with_curve(curve),
            );
            items.push(BatchItemResult {
                index: i,
                success: true,
                id: None,
                error: None,
            });
            succeeded += 1;
        }

        Ok(BatchResult {
            total,
            succeeded,
            failed: total - succeeded,
            items,
        })
    }

    fn list_automation_lanes(
        &self,
        pattern_id: PatternId,
    ) -> Result<Vec<AutomationLaneInfo>, McpBridgeError> {
        let song = self.shared.song.read();
        let pat_id = pattern_id;
        let pattern = song
            .pattern(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        Ok(pattern
            .automation
            .iter()
            .map(|lane| {
                let (target_name, instrument_id, scope) = automation_target_info(&lane.target);
                AutomationLaneInfo {
                    target: target_name,
                    instrument_id,
                    scope: scope.to_string(),
                    point_count: lane.len(),
                }
            })
            .collect())
    }

    fn get_instrument_automation_targets(
        &self,
        instrument_id: InstrumentId,
    ) -> Result<Vec<AutomationTargetInfo>, McpBridgeError> {
        use synth_core::ModuleType;
        use synth_sequencer::AutoInstrumentParam;
        self.validate_instrument(instrument_id)?;

        // Read from the session's synchronous registry (carries the live
        // descriptors), so freshly-added modules appear and we don't rebuild a
        // descriptor per module.
        let inst_id = instrument_id;
        let modules = self.session.all_modules_for_instrument(inst_id);

        let has_filter = modules
            .keys()
            .any(|id| id.module_type == ModuleType::Filter);
        let has_envelope = modules
            .keys()
            .any(|id| id.module_type == ModuleType::Envelope);

        let mut targets = Vec::new();

        // Per-module automatable parameters — enumerated by the shared helper
        // (single source of truth for the automatable filter + positional
        // identity, so the GUI Mod Grid picker and this tool never diverge),
        // enriched with unit/range/curve from the descriptor.
        for group in crate::module_targets::module_target_groups(&modules) {
            let prefix = group.module_id.module_type.prefix();
            let module_id = group.module_id.to_string();
            let Some(descriptor) = modules.get(&group.module_id) else {
                continue;
            };
            for (type_id, _) in &group.params {
                let Some(pd) = descriptor.parameters.iter().find(|p| &p.type_id == type_id) else {
                    continue;
                };
                let unit = pd.unit.suffix().trim();
                targets.push(AutomationTargetInfo {
                    target: format!(
                        "module:{prefix}:{}:{}",
                        group.module_id.instance, pd.type_id
                    ),
                    kind: "module".to_string(),
                    module_id: Some(module_id.clone()),
                    param_id: Some(pd.type_id.clone()),
                    display_name: pd.name.clone(),
                    unit: (!unit.is_empty()).then(|| unit.to_string()),
                    min: Some(pd.range.min),
                    max: Some(pd.range.max),
                    response_curve: Some(format!("{:?}", pd.response_curve)),
                });
            }
        }

        // Instrument-level macros — only those whose backing module exists, so
        // the tool doesn't advertise a target that resolves to nothing.
        for param in AutoInstrumentParam::ALL {
            let available = match param {
                AutoInstrumentParam::Volume | AutoInstrumentParam::Pan => true,
                AutoInstrumentParam::FilterCutoff | AutoInstrumentParam::FilterResonance => {
                    has_filter
                }
                AutoInstrumentParam::Attack
                | AutoInstrumentParam::Decay
                | AutoInstrumentParam::Sustain
                | AutoInstrumentParam::Release => has_envelope,
            };
            if !available {
                continue;
            }
            targets.push(AutomationTargetInfo {
                target: format!("{param:?}"),
                kind: "instrument".to_string(),
                module_id: None,
                param_id: None,
                display_name: param.display_name().to_string(),
                unit: None,
                min: None,
                max: None,
                response_curve: None,
            });
        }

        Ok(targets)
    }

    fn get_automation_points(
        &self,
        pattern_id: PatternId,
        target: &str,
        instrument_id: InstrumentId,
    ) -> Result<Vec<AutomationPointInfo>, McpBridgeError> {
        let valid_modules = self.instrument_module_ids(instrument_id);
        let auto_target =
            self.build_live_automation_target(target, instrument_id, &valid_modules)?;
        self.validate_automation_target_owner(&auto_target)?;
        let song = self.shared.song.read();
        let pat_id = pattern_id;
        let pattern = song
            .pattern(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let lane = pattern
            .automation_lane(&auto_target)
            .ok_or_else(|| McpBridgeError::Other(format!("automation lane not found: {target}")))?;

        Ok(lane
            .points()
            .iter()
            .map(|p| AutomationPointInfo {
                beat: ticks_to_beats(p.tick.0),
                value: p.value.as_f32(),
                curve: format_curve_type(p.curve),
            })
            .collect())
    }

    fn remove_automation_points(
        &self,
        pattern_id: PatternId,
        target: &str,
        instrument_id: InstrumentId,
        beats: &[f32],
    ) -> Result<BatchResult, McpBridgeError> {
        let valid_modules = self.instrument_module_ids(instrument_id);
        let auto_target =
            self.build_live_automation_target(target, instrument_id, &valid_modules)?;
        self.validate_automation_target_owner(&auto_target)?;
        let mut song = self.shared.song.write();
        let pat_id = pattern_id;
        let pattern = song
            .pattern_mut(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let lane = pattern
            .automation_lane_mut(&auto_target)
            .ok_or_else(|| McpBridgeError::Other(format!("automation lane not found: {target}")))?;

        let total = beats.len();
        let mut succeeded = 0usize;
        let mut items = Vec::with_capacity(total);

        for (i, &beat) in beats.iter().enumerate() {
            let tick = synth_sequencer::PatternTick(beats_to_ticks(beat));
            if lane.remove_point(tick).is_some() {
                items.push(BatchItemResult {
                    index: i,
                    success: true,
                    id: None,
                    error: None,
                });
                succeeded += 1;
            } else {
                items.push(BatchItemResult {
                    index: i,
                    success: false,
                    id: None,
                    error: Some(format!("no point at beat {beat}")),
                });
            }
        }

        Ok(BatchResult {
            total,
            succeeded,
            failed: total - succeeded,
            items,
        })
    }

    fn clear_automation_lane(
        &self,
        pattern_id: PatternId,
        target: &str,
        instrument_id: InstrumentId,
    ) -> Result<usize, McpBridgeError> {
        let valid_modules = self.instrument_module_ids(instrument_id);
        let auto_target =
            self.build_live_automation_target(target, instrument_id, &valid_modules)?;
        self.validate_automation_target_owner(&auto_target)?;
        let mut song = self.shared.song.write();
        let pat_id = pattern_id;
        let pattern = song
            .pattern_mut(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        let lane = pattern
            .automation_lane_mut(&auto_target)
            .ok_or_else(|| McpBridgeError::Other(format!("automation lane not found: {target}")))?;
        let count = lane.len();
        lane.clear();
        Ok(count)
    }

    #[allow(clippy::too_many_arguments)]
    fn simplify_automation(
        &self,
        pattern_id: Option<PatternId>,
        target: Option<&str>,
        instrument_id: InstrumentId,
        tolerance: f32,
        apply: bool,
    ) -> Result<synth_mcp::types::SimplifyAutomationResult, McpBridgeError> {
        use synth_mcp::types::{LaneSimplification, SimplifyAutomationResult};

        let tol = tolerance.clamp(0.0, 1.0);
        let mut warnings = Vec::new();
        if tolerance.is_nan() || tolerance < 0.0 {
            return Err(McpBridgeError::Other(
                "tolerance must be a non-negative normalized value (0.0..1.0)".to_string(),
            ));
        }

        // Optional single-lane filter: resolve the DSL target once.
        let target_filter = match target {
            Some(t) => {
                let valid = self.instrument_module_ids(instrument_id);
                Some(self.build_live_automation_target(t, instrument_id, &valid)?)
            }
            None => None,
        };

        // Take the write lock unconditionally: a dry-run simply skips the
        // mutation, keeping one code path for read and apply.
        let mut song = self.shared.song.write();
        let pattern_ids: Vec<PatternId> = match pattern_id {
            Some(pid) => {
                if song.pattern(pid).is_none() {
                    return Err(McpBridgeError::PatternNotFound(pid));
                }
                vec![pid]
            }
            None => song.patterns().map(|p| p.id).collect(),
        };

        let mut lanes = Vec::new();
        let mut total_before = 0usize;
        let mut total_after = 0usize;
        for pid in pattern_ids {
            let Some(pattern) = song.pattern_mut(pid) else {
                continue;
            };
            for lane in pattern.automation.iter_mut() {
                if let Some(tf) = &target_filter
                    && &lane.target != tf
                {
                    continue;
                }
                let before = lane.points().len();
                if before <= 2 {
                    continue;
                }
                let (kept, max_error) = simplify_automation_points(lane.points(), tol);
                let after = kept.len();
                if after >= before {
                    continue;
                }
                if apply {
                    lane.clear();
                    for p in &kept {
                        lane.add_point(*p);
                    }
                }
                total_before += before;
                total_after += after;
                lanes.push(LaneSimplification {
                    pattern_id: pid,
                    target: automation_target_info(&lane.target).0,
                    points_before: before,
                    points_after: after,
                    removed: before - after,
                    max_error,
                });
            }
        }
        drop(song);

        if let Some(t) = target
            && target_filter.is_some()
            && lanes.is_empty()
        {
            warnings.push(format!(
                "no simplifiable lane matched target '{t}' in the requested scope"
            ));
        }

        Ok(SimplifyAutomationResult {
            applied: apply,
            tolerance: tol,
            total_points_before: total_before,
            total_points_after: total_after,
            total_removed: total_before - total_after,
            lanes,
            warnings,
        })
    }

    fn transform_automation_lane(
        &self,
        pattern_id: PatternId,
        target: &str,
        instrument_id: InstrumentId,
        scale: f32,
        pivot: f32,
        offset: f32,
    ) -> Result<usize, McpBridgeError> {
        use synth_sequencer::AutomationPoint;

        let valid_modules = self.instrument_module_ids(instrument_id);
        let auto_target =
            self.build_live_automation_target(target, instrument_id, &valid_modules)?;
        self.validate_automation_target_owner(&auto_target)?;
        let mut song = self.shared.song.write();
        let pat_id = pattern_id;
        let pattern = song
            .pattern_mut(pat_id)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;

        // Require the lane to exist — don't silently create an empty one.
        let lane = pattern
            .automation_lane(&auto_target)
            .ok_or_else(|| McpBridgeError::Other(format!("automation lane not found: {target}")))?;

        let transformed: Vec<AutomationPoint> = lane
            .points()
            .iter()
            .map(|p| {
                let v = ((p.value.as_f32() - pivot) * scale + pivot + offset).clamp(0.0, 1.0);
                AutomationPoint::new(p.tick, NormalizedValue::new(v)).with_curve(p.curve)
            })
            .collect();
        let count = transformed.len();

        let lane = pattern.get_or_create_automation(auto_target);
        lane.clear();
        for pt in transformed {
            lane.add_point(pt);
        }
        Ok(count)
    }

    fn copy_automation_lane(
        &self,
        from_pattern_id: PatternId,
        from_target: &str,
        from_instrument_id: InstrumentId,
        to_pattern_id: PatternId,
        to_target: &str,
        to_instrument_id: InstrumentId,
        scale: f32,
        offset: f32,
        clear_destination: bool,
    ) -> Result<usize, McpBridgeError> {
        use synth_sequencer::AutomationPoint;

        let from_valid = self.instrument_module_ids(from_instrument_id);
        let to_valid = self.instrument_module_ids(to_instrument_id);
        let from_at =
            self.build_live_automation_target(from_target, from_instrument_id, &from_valid)?;
        let to_at = self.build_live_automation_target(to_target, to_instrument_id, &to_valid)?;
        self.validate_automation_target_owner(&from_at)?;
        self.validate_automation_target_owner(&to_at)?;
        let mut song = self.shared.song.write();
        let from_pat = from_pattern_id;
        let to_pat = to_pattern_id;

        // Read + transform the source points first (owned copy), releasing the
        // immutable borrow before mutating the destination lane.
        let src_points: Vec<AutomationPoint> = {
            let pattern = song
                .pattern(from_pat)
                .ok_or(McpBridgeError::PatternNotFound(from_pattern_id))?;
            let lane = pattern.automation_lane(&from_at).ok_or_else(|| {
                McpBridgeError::Other(format!("source automation lane not found: {from_target}"))
            })?;
            lane.points()
                .iter()
                .map(|p| {
                    let v = (p.value.as_f32() * scale + offset).clamp(0.0, 1.0);
                    AutomationPoint::new(p.tick, NormalizedValue::new(v)).with_curve(p.curve)
                })
                .collect()
        };

        let dest = song
            .pattern_mut(to_pat)
            .ok_or(McpBridgeError::PatternNotFound(to_pattern_id))?;
        let lane = dest.get_or_create_automation(to_at);
        if clear_destination {
            lane.clear();
        }
        let count = src_points.len();
        for pt in src_points {
            lane.add_point(pt);
        }
        Ok(count)
    }

    // === Track control ===

    };
}
