use super::*;

impl synth_mcp::bridge::SampleBridge for AppSynthBridge {
    fn list_samples(
        &self,
        filter: Option<&str>,
    ) -> Result<Vec<synth_mcp::types::SampleInfo>, McpBridgeError> {
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let metas = lib.list();
        let filter_lower = filter.map(|f| f.to_lowercase());
        Ok(metas
            .into_iter()
            .filter(|m| {
                filter_lower
                    .as_ref()
                    .is_none_or(|f| m.name.to_lowercase().contains(f))
            })
            .map(meta_to_sample_info)
            .collect())
    }

    fn import_sample(
        &self,
        path: &str,
        name: Option<&str>,
        root_note: Option<u8>,
    ) -> Result<synth_mcp::types::SampleInfo, McpBridgeError> {
        let file_path = std::path::Path::new(path);
        if !file_path.exists() {
            return Err(McpBridgeError::Other(format!("File not found: {path}")));
        }
        let target_rate = synth_core::audio::SampleRate::DVD_QUALITY;
        let mut sample = synth_sampler::load_wav(file_path, target_rate)
            .map_err(|e| McpBridgeError::Other(format!("WAV load error: {e}")))?;
        if let Some(n) = name {
            sample.meta.name = n.to_string();
        }
        if let Some(note) = root_note {
            sample.meta.root_note = Some(synth_core::MidiNote::new(note));
        }
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let id = lib.add(sample);
        let meta = lib.get_meta(id).ok_or_else(|| {
            McpBridgeError::Other("Failed to retrieve imported sample".to_string())
        })?;
        Ok(meta_to_sample_info(meta))
    }

    fn delete_sample(&self, id: u64) -> Result<(), McpBridgeError> {
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sample_id = synth_sampler::SampleId::new(id);
        lib.remove(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        Ok(())
    }

    fn rename_sample(&self, id: u64, name: &str) -> Result<(), McpBridgeError> {
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sample_id = synth_sampler::SampleId::new(id);
        let meta = lib
            .get_meta(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?
            .clone();
        let mut updated = meta;
        updated.name = name.to_string();
        lib.update_meta(sample_id, updated);
        Ok(())
    }

    fn set_sample_description(&self, id: u64, description: &str) -> Result<(), McpBridgeError> {
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sample_id = synth_sampler::SampleId::new(id);
        let mut updated = lib
            .get_meta(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?
            .clone();
        updated.description = description.to_string();
        lib.update_meta(sample_id, updated);
        Ok(())
    }

    fn set_sample_root_note(&self, id: u64, note: MidiNote) -> Result<(), McpBridgeError> {
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sample_id = synth_sampler::SampleId::new(id);
        let meta = lib
            .get_meta(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?
            .clone();
        let mut updated = meta;
        updated.root_note = Some(note);
        lib.update_meta(sample_id, updated);
        Ok(())
    }

    fn normalize_sample(&self, id: u64) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        let peak = sample.data.iter().fold(0.0_f32, |a, &s| a.max(s.abs()));
        if peak <= 0.0 || (peak - 1.0).abs() < 1e-6 {
            return Ok(());
        }
        let gain = 1.0 / peak;
        let normalized: std::sync::Arc<[f32]> = sample
            .data
            .iter()
            .map(|&s| s * gain)
            .collect::<Vec<_>>()
            .into();
        drop(lib);
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        lib.replace_data(sample_id, normalized);
        Ok(())
    }

    fn reverse_sample(&self, id: u64) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        let channels = sample.meta.channels.count() as usize;
        let frame_count = sample.meta.frame_count.as_usize();
        let mut reversed = vec![0.0_f32; sample.data.len()];
        for frame in 0..frame_count {
            let src = frame_count - 1 - frame;
            for ch in 0..channels {
                reversed[frame * channels + ch] = sample.data[src * channels + ch];
            }
        }
        let data: std::sync::Arc<[f32]> = reversed.into();
        drop(lib);
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        lib.replace_data(sample_id, data);
        Ok(())
    }

    fn trim_sample_silence(&self, id: u64) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let threshold = 0.01_f32;
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        let channels = sample.meta.channels.count() as usize;
        let frame_count = sample.meta.frame_count.as_usize();
        let mut start = 0;
        for frame in 0..frame_count {
            let mut peak = 0.0_f32;
            for ch in 0..channels {
                peak = peak.max(sample.data[frame * channels + ch].abs());
            }
            if peak > threshold {
                start = frame;
                break;
            }
        }
        let mut end = frame_count;
        for frame in (0..frame_count).rev() {
            let mut peak = 0.0_f32;
            for ch in 0..channels {
                peak = peak.max(sample.data[frame * channels + ch].abs());
            }
            if peak > threshold {
                end = frame + 1;
                break;
            }
        }
        drop(lib);
        if start < end {
            let mut lib = self
                .sample_library
                .write()
                .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
            lib.update_crop(
                sample_id,
                Some(synth_sampler::CropRegion {
                    start: synth_sampler::FrameIndex::new(start),
                    end: synth_sampler::FrameIndex::new(end),
                }),
            );
        }
        Ok(())
    }

    fn get_sample_info(
        &self,
        id: u64,
    ) -> Result<synth_mcp::types::DetailedSampleInfo, McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;

        let data = &sample.data;
        let len = data.len();
        let (peak_level, rms_level, dc_offset) = if len == 0 {
            (0.0, 0.0, 0.0)
        } else {
            let (peak, sq_sum, dc_sum) = data
                .iter()
                .fold((0.0_f32, 0.0_f32, 0.0_f32), |(peak, sq, dc), &s| {
                    (peak.max(s.abs()), sq + s * s, dc + s)
                });
            let rms = (sq_sum / len as f32).sqrt();
            let dc = dc_sum / len as f32;
            (peak, rms, dc)
        };
        let memory_bytes = len * std::mem::size_of::<f32>();
        let sr = f64::from(sample.meta.sample_rate.as_u32());

        let (loop_start_seconds, loop_end_seconds) = match &sample.meta.loop_region {
            Some(region) => (
                Some(region.start.0 as f64 / sr),
                Some(region.end.0 as f64 / sr),
            ),
            None => (None, None),
        };

        let (crop_start_seconds, crop_end_seconds) = match &sample.meta.crop {
            Some(region) => (
                Some(region.start.0 as f64 / sr),
                Some(region.end.0 as f64 / sr),
            ),
            None => (None, None),
        };

        let info = meta_to_sample_info(&sample.meta);
        Ok(synth_mcp::types::DetailedSampleInfo {
            info,
            peak_level,
            rms_level,
            dc_offset,
            memory_bytes,
            loop_start_seconds,
            loop_end_seconds,
            crop_start_seconds,
            crop_end_seconds,
        })
    }

    fn duplicate_sample(&self, id: u64) -> Result<synth_mcp::types::SampleInfo, McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let (cloned_data, cloned_meta) = {
            let lib = self
                .sample_library
                .read()
                .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
            let sample = lib
                .get(sample_id)
                .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
            (Arc::clone(&sample.data), sample.meta.clone())
        };

        let new_meta = synth_sampler::SampleMeta {
            id: synth_sampler::SampleId::new(0),
            name: format!("{} (copy)", cloned_meta.name),
            ..cloned_meta
        };
        let new_sample = synth_sampler::Sample::new(new_meta, cloned_data);

        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let new_id = lib.add(new_sample);
        let new_meta = lib.get_meta(new_id).ok_or_else(|| {
            McpBridgeError::Other("Failed to retrieve duplicated sample".to_string())
        })?;
        Ok(meta_to_sample_info(new_meta))
    }

    fn set_sample_loop(
        &self,
        id: u64,
        enabled: bool,
        start_seconds: Option<f64>,
        end_seconds: Option<f64>,
        crossfade_ms: Option<f64>,
    ) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        if enabled {
            let start_s = start_seconds.ok_or_else(|| {
                McpBridgeError::Other("start_seconds required when enabled".to_string())
            })?;
            let end_s = end_seconds.ok_or_else(|| {
                McpBridgeError::Other("end_seconds required when enabled".to_string())
            })?;
            let sample = lib
                .get(sample_id)
                .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
            let sr = f64::from(sample.meta.sample_rate.as_u32());
            let start_frame = (start_s * sr) as usize;
            let end_frame = (end_s * sr) as usize;
            let crossfade_frames = crossfade_ms
                .map(|ms| (ms / 1000.0 * sr) as usize)
                .unwrap_or(0);
            lib.update_loop(
                sample_id,
                Some(synth_sampler::LoopRegion {
                    start: synth_sampler::FrameIndex::new(start_frame),
                    end: synth_sampler::FrameIndex::new(end_frame),
                    crossfade: SampleCount::new(crossfade_frames),
                }),
            );
        } else {
            lib.update_loop(sample_id, None);
        }
        Ok(())
    }

    fn set_sample_crop(
        &self,
        id: u64,
        start_seconds: Option<f64>,
        end_seconds: Option<f64>,
    ) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let mut lib = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        if start_seconds.is_none() && end_seconds.is_none() {
            lib.update_crop(sample_id, None);
        } else {
            let sample = lib
                .get(sample_id)
                .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
            let sr = f64::from(sample.meta.sample_rate.as_u32());
            let start_frame = start_seconds.map(|s| (s * sr) as usize).unwrap_or(0);
            let end_frame = end_seconds
                .map(|s| (s * sr) as usize)
                .unwrap_or(sample.meta.frame_count.as_usize());
            lib.update_crop(
                sample_id,
                Some(synth_sampler::CropRegion {
                    start: synth_sampler::FrameIndex::new(start_frame),
                    end: synth_sampler::FrameIndex::new(end_frame),
                }),
            );
        }
        Ok(())
    }

    fn export_sample(
        &self,
        id: u64,
        path: &str,
        bit_depth: Option<u8>,
    ) -> Result<(), McpBridgeError> {
        let sample_id = synth_sampler::SampleId::new(id);
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
        let sample = lib
            .get(sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {id}")))?;
        let depth = match bit_depth {
            Some(24) => synth_sampler::BitDepth::Int24,
            Some(32) => synth_sampler::BitDepth::Float32,
            _ => synth_sampler::BitDepth::Int16,
        };
        synth_sampler::save_wav(sample, std::path::Path::new(path), depth)
            .map_err(|e| McpBridgeError::Other(format!("Export failed: {e}")))?;
        Ok(())
    }

    // === Sampler module control ===

    fn assign_sample_to_module(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
        sample_id: u64,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = instrument_id;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        // Get sample data from library
        let lib = self
            .sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        let sampler_sample_id = synth_sampler::SampleId::new(sample_id);
        let sample = lib
            .get(sampler_sample_id)
            .ok_or_else(|| McpBridgeError::Other(format!("Sample not found: {sample_id}")))?;
        let data = Arc::clone(&sample.data);
        let channels = sample.meta.channels;
        let frame_count = sample.meta.frame_count.as_usize();
        let root_note = sample.meta.root_note.unwrap_or(MidiNote::new(60));
        drop(lib);

        // Send SampleSelect param
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetModuleParameter {
                instrument_id: Some(inst_id),
                module_id: mid,
                param: Param::sample_select(sample_id),
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "assign_sample_select",
            });
        }

        // Send the actual audio data
        if !self
            .session
            .command_sender()
            .send(EngineCommand::LoadSampleData {
                instrument_id: inst_id,
                module_id: mid,
                data,
                channels,
                frame_count,
                root_note,
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "load_sample_data",
            });
        }

        Ok(())
    }

    fn get_sampler_state(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
    ) -> Result<synth_mcp::types::SamplerStateInfo, McpBridgeError> {
        use synth_core::{Param, PlayDirection, SamplerParam, SamplerPlayMode};

        self.validate_instrument(instrument_id)?;

        // `Param::as_f32()` returns 0.0 for `SamplerParam::SampleSelect` (the
        // sample id doesn't fit a slider), so the f32 path silently loses it.
        // Read the typed enum from the snapshot instead.
        let inst_id = instrument_id;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;
        let snapshot = self
            .session
            .state()
            .shared_graph
            .get_module(inst_id, mid)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        let mut sample_id: Option<synth_sampler::SampleId> = None;
        let mut pitch_tracking = true;
        let mut level: f32 = 0.0;
        let mut play_mode = SamplerPlayMode::OneShot;
        let mut direction = PlayDirection::Forward;
        let mut velocity_sensitivity: f32 = 1.0;
        let mut fine_tune: f32 = 0.0;
        let mut start_offset: f32 = 0.0;
        for param in &snapshot.parameters {
            if let Param::Sampler(sp) = param {
                match sp {
                    SamplerParam::SampleSelect(sid) if sid.as_u64() != 0 => {
                        sample_id = Some(synth_sampler::SampleId::new(sid.as_u64()));
                    }
                    SamplerParam::SampleSelect(_) => {}
                    SamplerParam::PitchTracking(b) => pitch_tracking = *b,
                    SamplerParam::Level(g) => level = g.as_f32(),
                    SamplerParam::PlayMode(m) => play_mode = *m,
                    SamplerParam::Direction(d) => direction = *d,
                    SamplerParam::VelocitySensitivity(v) => velocity_sensitivity = v.as_f32(),
                    SamplerParam::FineTune(c) => fine_tune = c.as_f32(),
                    SamplerParam::StartOffset(v) => start_offset = v.as_f32(),
                }
            }
        }

        let play_mode_str = match play_mode {
            SamplerPlayMode::OneShot => "one_shot",
            SamplerPlayMode::Sustain => "sustain",
            SamplerPlayMode::Loop => "loop",
        }
        .to_string();

        let direction_str = match direction {
            PlayDirection::Forward => "forward",
            PlayDirection::Reverse => "reverse",
            PlayDirection::PingPong => "ping_pong",
        }
        .to_string();

        let sample_name = if let Some(id) = sample_id {
            let lib = self
                .sample_library
                .read()
                .map_err(|_| McpBridgeError::Other("Lock poisoned".to_string()))?;
            lib.get_meta(id).map(|m| m.name.clone()).unwrap_or_default()
        } else {
            String::new()
        };

        Ok(synth_mcp::types::SamplerStateInfo {
            sample_id: sample_id.map_or(0, |id| id.0),
            sample_name,
            pitch_tracking,
            level: NormalizedValue::new(level),
            play_mode: play_mode_str,
            direction: direction_str,
            velocity_sensitivity,
            fine_tune,
            start_offset,
        })
    }

    fn set_sampler_parameter(
        &self,
        instrument_id: InstrumentId,
        module_id: &str,
        param_name: &str,
        value: &str,
    ) -> Result<(), McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = instrument_id;
        let mid: ModuleId = module_id
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(module_id.to_string()))?;

        let param = match param_name {
            "pitch_tracking" | "pitch_track" => {
                let b = value.parse::<bool>().map_err(|_| {
                    McpBridgeError::Other(format!(
                        "Invalid boolean value '{value}', expected 'true' or 'false'"
                    ))
                })?;
                synth_core::SamplerParam::PitchTracking(b)
            }
            "level" => {
                let v = value
                    .parse::<f32>()
                    .map_err(|_| McpBridgeError::Other(format!("Invalid float value '{value}'")))?;
                synth_core::SamplerParam::Level(synth_core::Gain::new(v))
            }
            "play_mode" => {
                let mode = match value {
                    "one_shot" => synth_core::SamplerPlayMode::OneShot,
                    "sustain" => synth_core::SamplerPlayMode::Sustain,
                    "loop" => synth_core::SamplerPlayMode::Loop,
                    _ => {
                        return Err(McpBridgeError::Other(format!(
                            "Invalid play_mode '{value}', expected one_shot/sustain/loop"
                        )));
                    }
                };
                synth_core::SamplerParam::PlayMode(mode)
            }
            "direction" => {
                let dir = match value {
                    "forward" => synth_core::PlayDirection::Forward,
                    "reverse" => synth_core::PlayDirection::Reverse,
                    "ping_pong" => synth_core::PlayDirection::PingPong,
                    _ => {
                        return Err(McpBridgeError::Other(format!(
                            "Invalid direction '{value}', expected forward/reverse/ping_pong"
                        )));
                    }
                };
                synth_core::SamplerParam::Direction(dir)
            }
            "velocity_sensitivity" | "vel_sens" => {
                let v = value
                    .parse::<f32>()
                    .map_err(|_| McpBridgeError::Other(format!("Invalid float value '{value}'")))?;
                synth_core::SamplerParam::VelocitySensitivity(NormalizedValue::new(v))
            }
            "fine_tune" => {
                let v = value
                    .parse::<f32>()
                    .map_err(|_| McpBridgeError::Other(format!("Invalid float value '{value}'")))?;
                synth_core::SamplerParam::FineTune(synth_core::Cents::new(v))
            }
            "start_offset" => {
                let v = value
                    .parse::<f32>()
                    .map_err(|_| McpBridgeError::Other(format!("Invalid float value '{value}'")))?;
                synth_core::SamplerParam::StartOffset(NormalizedValue::new(v))
            }
            _ => {
                return Err(McpBridgeError::ParameterNotFound(format!(
                    "Unknown sampler parameter '{param_name}'. Available: pitch_tracking, level, \
                     play_mode, direction, velocity_sensitivity, fine_tune, start_offset"
                )));
            }
        };

        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetModuleParameter {
                instrument_id: Some(inst_id),
                module_id: mid,
                param: Param::Sampler(param),
            })
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "set_sampler_parameter",
            });
        }

        Ok(())
    }

    // === Audio input ===
}
