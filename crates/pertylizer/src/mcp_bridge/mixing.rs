use super::*;
use synth_mcp::bridge::InstrumentBridge;

impl synth_mcp::bridge::MixingBridge for AppSynthBridge {
    fn set_track_volume(
        &self,
        track_id: TrackId,
        volume: NormalizedValue,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = track_id;
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.volume = volume;
        Ok(())
    }

    fn set_track_pan(&self, track_id: TrackId, pan: BipolarValue) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = track_id;
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        // Convert bipolar (-1.0..1.0) to normalized (0.0..1.0) for internal storage
        track.pan = pan;
        Ok(())
    }

    fn set_track_mute(&self, track_id: TrackId, muted: bool) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = track_id;
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.set_mute(muted);
        Ok(())
    }

    fn set_track_solo(&self, track_id: TrackId, solo: bool) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = track_id;
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.set_solo(solo);
        Ok(())
    }

    fn set_track_instrument(
        &self,
        track_id: TrackId,
        instrument_id: Option<InstrumentId>,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = track_id;
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        // Every track has an instrument now; `None` is a no-op (can no longer
        // clear to "no instrument"). Signature tightening is Phase 6 MCP cleanup.
        if let Some(inst_id) = instrument_id {
            track.instrument = inst_id;
        }
        Ok(())
    }

    fn rename_track(&self, track_id: TrackId, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = track_id;
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.name = name.to_string();
        Ok(())
    }

    fn set_track_description(
        &self,
        track_id: TrackId,
        description: &str,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = track_id;
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.description = description.to_string();
        Ok(())
    }

    fn set_track_color(&self, track_id: TrackId, color: &str) -> Result<(), McpBridgeError> {
        let parsed = synth_sequencer::TrackColor::from_hex(color).ok_or_else(|| {
            McpBridgeError::Other(format!(
                "invalid color {color:?}; expected \"#RRGGBB\" or \"#RRGGBBAA\""
            ))
        })?;
        let mut song = self.shared.song.write();
        let tid = track_id;
        let track = song
            .track_mut(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.color = parsed;
        Ok(())
    }

    fn delete_track(&self, track_id: TrackId) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let tid = track_id;
        song.delete_track(tid)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        Ok(())
    }

    // === Return busses (effect sends) ===

    fn list_return_busses(&self) -> Result<Vec<synth_mcp::ReturnBusInfo>, McpBridgeError> {
        // Collect the live effect chains first (separate lock), then merge with
        // the song-owned fader settings. The effect chain is engine runtime state;
        // the fader is song state.
        let song = self.shared.song.read();
        Ok(song
            .return_busses()
            .iter()
            .map(|b| synth_mcp::ReturnBusInfo {
                id: b.id,
                name: b.name.clone(),
                volume: b.volume,
                pan: b.pan,
                mute: b.mute,
                solo: b.solo,
                color: b.color.to_hex(),
                description: b.description.clone(),
                effects: self.return_bus_effect_infos(b.id),
                sends: b
                    .sends
                    .iter()
                    .map(|s| synth_mcp::ReturnSendInfo {
                        target: s.target,
                        level: s.level,
                        enabled: s.enabled,
                    })
                    .collect(),
            })
            .collect())
    }

    fn create_return_bus(&self, name: &str) -> Result<ReturnBusId, McpBridgeError> {
        let id = {
            let mut song = self.shared.song.write();
            song.create_return_bus(name)
        };
        // Allocate the engine-side runtime channel (off the audio hot path).
        // If the command can't be queued, roll back the song write so the song
        // and engine don't disagree about which return busses exist.
        if !self
            .session
            .command_sender()
            .send(EngineCommand::CreateReturnBus { id })
        {
            self.shared.song.write().delete_return_bus(id);
            return Err(McpBridgeError::CommandSendFailed {
                command: "CreateReturnBus",
            });
        }
        Ok(id)
    }

    fn delete_return_bus(&self, return_id: ReturnBusId) -> Result<(), McpBridgeError> {
        let id = return_id;
        {
            let mut song = self.shared.song.write();
            song.delete_return_bus(id)
                .ok_or(McpBridgeError::ReturnBusNotFound(return_id))?;
        }
        if !self
            .session
            .command_sender()
            .send(EngineCommand::RemoveReturnBus { id })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "RemoveReturnBus",
            });
        }
        Ok(())
    }

    fn set_return_bus_volume(
        &self,
        return_id: ReturnBusId,
        volume: NormalizedValue,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let bus = song
            .return_bus_mut(return_id)
            .ok_or(McpBridgeError::ReturnBusNotFound(return_id))?;
        bus.volume = volume;
        Ok(())
    }

    fn set_return_bus_pan(
        &self,
        return_id: ReturnBusId,
        pan: BipolarValue,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let bus = song
            .return_bus_mut(return_id)
            .ok_or(McpBridgeError::ReturnBusNotFound(return_id))?;
        bus.pan = pan;
        Ok(())
    }

    fn set_return_bus_mute(
        &self,
        return_id: ReturnBusId,
        muted: bool,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let bus = song
            .return_bus_mut(return_id)
            .ok_or(McpBridgeError::ReturnBusNotFound(return_id))?;
        // Mute clears solo (mutually exclusive), mirroring tracks.
        bus.set_mute(muted);
        Ok(())
    }

    fn set_return_bus_solo(
        &self,
        return_id: ReturnBusId,
        solo: bool,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let bus = song
            .return_bus_mut(return_id)
            .ok_or(McpBridgeError::ReturnBusNotFound(return_id))?;
        bus.set_solo(solo);
        Ok(())
    }

    fn set_return_bus_color(
        &self,
        return_id: ReturnBusId,
        color: &str,
    ) -> Result<(), McpBridgeError> {
        let parsed = synth_sequencer::TrackColor::from_hex(color)
            .ok_or_else(|| McpBridgeError::Other(format!("invalid color '{color}'")))?;
        let mut song = self.shared.song.write();
        let bus = song
            .return_bus_mut(return_id)
            .ok_or(McpBridgeError::ReturnBusNotFound(return_id))?;
        bus.color = parsed;
        Ok(())
    }

    fn set_return_bus_description(
        &self,
        return_id: ReturnBusId,
        description: &str,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let bus = song
            .return_bus_mut(return_id)
            .ok_or(McpBridgeError::ReturnBusNotFound(return_id))?;
        bus.description = description.to_string();
        Ok(())
    }

    fn rename_return_bus(&self, return_id: ReturnBusId, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let bus = song
            .return_bus_mut(return_id)
            .ok_or(McpBridgeError::ReturnBusNotFound(return_id))?;
        bus.name = name.to_string();
        Ok(())
    }

    fn set_track_send(
        &self,
        track_id: TrackId,
        return_id: ReturnBusId,
        level: NormalizedValue,
        pre_fader: bool,
        enabled: bool,
    ) -> Result<(), McpBridgeError> {
        let rid = return_id;
        let mut song = self.shared.song.write();
        if !song.return_busses().iter().any(|b| b.id == rid) {
            return Err(McpBridgeError::ReturnBusNotFound(return_id));
        }
        let track = song
            .track_mut(track_id)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        if let Some(send) = track.sends.iter_mut().find(|s| s.target == rid) {
            send.level = level;
            send.pre_fader = pre_fader;
            send.enabled = enabled;
        } else {
            track.sends.push(synth_sequencer::TrackSend {
                target: rid,
                level,
                pre_fader,
                enabled,
            });
        }
        Ok(())
    }

    fn remove_track_send(
        &self,
        track_id: TrackId,
        return_id: ReturnBusId,
    ) -> Result<(), McpBridgeError> {
        let rid = return_id;
        let mut song = self.shared.song.write();
        let track = song
            .track_mut(track_id)
            .ok_or(McpBridgeError::TrackNotFound(track_id))?;
        track.sends.retain(|s| s.target != rid);
        Ok(())
    }

    fn set_return_send(
        &self,
        from_id: ReturnBusId,
        to_id: ReturnBusId,
        level: NormalizedValue,
        enabled: bool,
    ) -> Result<(), McpBridgeError> {
        let from = from_id;
        let to = to_id;
        let mut song = self.shared.song.write();
        if !song.return_busses().iter().any(|b| b.id == from) {
            return Err(McpBridgeError::ReturnBusNotFound(from_id));
        }
        if !song.return_busses().iter().any(|b| b.id == to) {
            return Err(McpBridgeError::ReturnBusNotFound(to_id));
        }
        // Refuse self-sends and any routing that would close a cycle (the engine
        // can only process an acyclic return graph in dependency order).
        if song.return_send_would_cycle(from, to) {
            return Err(McpBridgeError::Other(format!(
                "return send {from_id} -> {to_id} would create a routing cycle"
            )));
        }
        let bus = song
            .return_bus_mut(from)
            .ok_or(McpBridgeError::ReturnBusNotFound(from_id))?;
        if let Some(send) = bus.sends.iter_mut().find(|s| s.target == to) {
            send.level = level;
            send.enabled = enabled;
        } else {
            bus.sends.push(synth_sequencer::ReturnSend {
                target: to,
                level,
                enabled,
            });
        }
        Ok(())
    }

    fn remove_return_send(
        &self,
        from_id: ReturnBusId,
        to_id: ReturnBusId,
    ) -> Result<(), McpBridgeError> {
        let from = from_id;
        let to = to_id;
        let mut song = self.shared.song.write();
        let bus = song
            .return_bus_mut(from)
            .ok_or(McpBridgeError::ReturnBusNotFound(from_id))?;
        bus.sends.retain(|s| s.target != to);
        Ok(())
    }

    fn get_master_volume(&self) -> Result<f32, McpBridgeError> {
        Ok(self.session.state().master_volume.load())
    }

    fn set_master_volume(&self, volume: Gain) -> Result<(), McpBridgeError> {
        // `Gain::new` does not reject NaN/inf; guard at the boundary so a bad
        // value can't poison the master gain (the MCP tool also range-checks).
        if !volume.as_f32().is_finite() {
            return Err(McpBridgeError::Other(format!(
                "master volume must be finite, got {volume}"
            )));
        }
        if self
            .session
            .command_sender()
            .send(EngineCommand::SetMasterVolume(volume))
        {
            Ok(())
        } else {
            Err(McpBridgeError::CommandSendFailed {
                command: "SetMasterVolume",
            })
        }
    }

    fn list_master_effects(&self) -> Result<Vec<synth_mcp::ReturnEffectInfo>, McpBridgeError> {
        Ok(self
            .session
            .state()
            .master_effects
            .read()
            .iter()
            .map(return_effect_info)
            .collect())
    }

    fn add_master_effect(&self, effect_type: &str) -> Result<String, McpBridgeError> {
        let module_type = parse_module_type(effect_type)
            .ok_or_else(|| McpBridgeError::InvalidModuleType(effect_type.to_string()))?;
        let Some((effect, _descriptor)) = crate::module_factory::create_effect(module_type) else {
            return Err(McpBridgeError::InvalidModuleType(format!(
                "{effect_type} is not an effect"
            )));
        };
        let module_id = ModuleId::new(module_type, self.next_master_effect_instance(module_type));
        // `instrument_id: None` targets the master effect chain.
        if !self
            .session
            .command_sender()
            .send(EngineCommand::AddEffectInstance {
                instrument_id: None,
                id: module_id,
                effect,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "AddEffectInstance",
            });
        }
        Ok(module_id.to_string())
    }

    fn remove_master_effect(&self, module_id: &str) -> Result<(), McpBridgeError> {
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        if !self
            .session
            .command_sender()
            .send(EngineCommand::RemoveEffect {
                instrument_id: None,
                id: mid,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "RemoveEffect",
            });
        }
        Ok(())
    }

    fn set_master_effect_parameter(
        &self,
        module_id: &str,
        param_name: &str,
        value: BridgeParamValue,
    ) -> Result<ParameterInfo, McpBridgeError> {
        let (mid, param, info) = self.resolve_effect_param(module_id, param_name, value)?;
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetEffectParameter {
                instrument_id: None,
                module_id: mid,
                param,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetEffectParameter",
            });
        }
        Ok(info)
    }

    fn set_master_effect_enabled(
        &self,
        module_id: &str,
        enabled: bool,
    ) -> Result<(), McpBridgeError> {
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetEffectEnabled {
                instrument_id: None,
                module_id: mid,
                enabled,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetEffectEnabled",
            });
        }
        Ok(())
    }

    fn reorder_master_effect(
        &self,
        module_id: &str,
        direction: synth_mcp::bridge::ReturnEffectMove,
    ) -> Result<(), McpBridgeError> {
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        let direction = match direction {
            synth_mcp::bridge::ReturnEffectMove::Up => synth_engine::commands::ReorderDirection::Up,
            synth_mcp::bridge::ReturnEffectMove::Down => {
                synth_engine::commands::ReorderDirection::Down
            }
        };
        if !self
            .session
            .command_sender()
            .send(EngineCommand::ReorderEffect {
                instrument_id: None,
                module_id: mid,
                direction,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "ReorderEffect",
            });
        }
        Ok(())
    }

    fn add_return_effect(
        &self,
        return_id: ReturnBusId,
        effect_type: &str,
    ) -> Result<String, McpBridgeError> {
        let rid = return_id;
        self.require_return_bus(rid)?;

        let module_type = parse_module_type(effect_type)
            .ok_or_else(|| McpBridgeError::InvalidModuleType(effect_type.to_string()))?;
        // `create_effect` returns `None` for voice modules / visualizers — only
        // real audio effects can live on a return bus.
        let Some((effect, _descriptor)) = crate::module_factory::create_effect(module_type) else {
            return Err(McpBridgeError::InvalidModuleType(format!(
                "{effect_type} is not an effect"
            )));
        };

        let module_id = ModuleId::new(
            module_type,
            self.next_return_effect_instance(rid, module_type),
        );
        if !self
            .session
            .command_sender()
            .send(EngineCommand::AddReturnEffect {
                return_id: rid,
                id: module_id,
                effect,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "AddReturnEffect",
            });
        }
        Ok(module_id.to_string())
    }

    fn remove_return_effect(
        &self,
        return_id: ReturnBusId,
        module_id: &str,
    ) -> Result<(), McpBridgeError> {
        let rid = return_id;
        self.require_return_bus(rid)?;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        if !self
            .session
            .command_sender()
            .send(EngineCommand::RemoveReturnEffect {
                return_id: rid,
                id: mid,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "RemoveReturnEffect",
            });
        }
        Ok(())
    }

    fn set_return_effect_parameter(
        &self,
        return_id: ReturnBusId,
        module_id: &str,
        param_name: &str,
        value: BridgeParamValue,
    ) -> Result<ParameterInfo, McpBridgeError> {
        let rid = return_id;
        self.require_return_bus(rid)?;
        let (mid, param, info) = self.resolve_effect_param(module_id, param_name, value)?;
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetReturnEffectParameter {
                return_id: rid,
                module_id: mid,
                param,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetReturnEffectParameter",
            });
        }
        Ok(info)
    }

    fn set_return_effect_enabled(
        &self,
        return_id: ReturnBusId,
        module_id: &str,
        enabled: bool,
    ) -> Result<(), McpBridgeError> {
        let rid = return_id;
        self.require_return_bus(rid)?;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetReturnEffectEnabled {
                return_id: rid,
                module_id: mid,
                enabled,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "SetReturnEffectEnabled",
            });
        }
        Ok(())
    }

    fn reorder_return_effect(
        &self,
        return_id: ReturnBusId,
        module_id: &str,
        direction: synth_mcp::bridge::ReturnEffectMove,
    ) -> Result<(), McpBridgeError> {
        let rid = return_id;
        self.require_return_bus(rid)?;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        let direction = match direction {
            synth_mcp::bridge::ReturnEffectMove::Up => synth_engine::commands::ReorderDirection::Up,
            synth_mcp::bridge::ReturnEffectMove::Down => {
                synth_engine::commands::ReorderDirection::Down
            }
        };
        if !self
            .session
            .command_sender()
            .send(EngineCommand::ReorderReturnEffect {
                return_id: rid,
                module_id: mid,
                direction,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "ReorderReturnEffect",
            });
        }
        Ok(())
    }

    // === Pattern management ===

    fn rename_pattern(&self, pattern_id: PatternId, name: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        pattern.name = name.to_string();
        Ok(())
    }

    fn set_pattern_description(
        &self,
        pattern_id: PatternId,
        description: &str,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        pattern.description = description.to_string();
        Ok(())
    }

    fn set_pattern_length(
        &self,
        pattern_id: PatternId,
        length_beats: f32,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        let pattern = song
            .pattern_mut(pid)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))?;
        pattern.length = synth_sequencer::Duration(beats_to_ticks(length_beats));
        Ok(())
    }

    fn duplicate_pattern(&self, pattern_id: PatternId) -> Result<u32, McpBridgeError> {
        let mut song = self.shared.song.write();
        let pid = pattern_id;
        song.duplicate_pattern(pid)
            .map(|new_id| new_id.0)
            .ok_or(McpBridgeError::PatternNotFound(pattern_id))
    }

    // === Song metadata ===

    fn set_song_author(&self, author: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        song.author = author.to_string();
        Ok(())
    }

    fn set_song_description(&self, description: &str) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        song.description = description.to_string();
        Ok(())
    }

    fn set_song_time_signature(
        &self,
        numerator: u8,
        denominator: u8,
    ) -> Result<(), McpBridgeError> {
        let mut song = self.shared.song.write();
        song.default_time_signature = synth_sequencer::TimeSignature {
            numerator,
            denominator,
        };
        Ok(())
    }

    // === Batch parameter set ===

    fn set_parameters(
        &self,
        instrument_id: InstrumentId,
        params: &[BridgeParamSet],
    ) -> Result<BatchResult, McpBridgeError> {
        self.validate_instrument(instrument_id)?;

        let total = params.len();
        let mut succeeded = 0usize;
        let mut items = Vec::with_capacity(total);

        for (i, ps) in params.iter().enumerate() {
            // Delegate to the single-parameter path so each item goes through the
            // same choice/address resolution and range validation — this is what
            // lets a batch set an address-based value (e.g. a Mod Matrix
            // `slot_N_dest` of `"spp-1.x"`), not just a scalar `f32`.
            match self.set_parameter(
                instrument_id,
                &ps.module_id,
                &ps.param_name,
                ps.value.clone(),
            ) {
                Ok(_) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: true,
                        id: None,
                        error: None,
                    });
                    succeeded += 1;
                }
                Err(e) => {
                    items.push(BatchItemResult {
                        index: i,
                        success: false,
                        id: None,
                        error: Some(format!("{e}")),
                    });
                }
            }
        }

        Ok(BatchResult {
            total,
            succeeded,
            failed: total - succeeded,
            items,
        })
    }

    // === Project management ===
}
