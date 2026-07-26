use super::*;

impl synth_mcp::bridge::AudioInputBridge for AppSynthBridge {
    fn list_input_devices(&self) -> Result<Vec<synth_mcp::types::InputDeviceInfo>, McpBridgeError> {
        let host = self.audio_input_host.lock();
        let host = host.as_ref().ok_or_else(|| {
            McpBridgeError::Other("audio input backend is unavailable".to_string())
        })?;
        let devices = host
            .devices()
            .map_err(|error| McpBridgeError::Other(error.to_string()))?;
        Ok(devices
            .into_iter()
            .filter(|device| {
                matches!(
                    device.device_type,
                    synth_core::audio::DeviceType::Input | synth_core::audio::DeviceType::Duplex
                )
            })
            .map(|device| synth_mcp::types::InputDeviceInfo {
                id: device.id,
                name: device.name,
                input_channels: device.input_channels.count(),
            })
            .collect())
    }

    fn get_input_state(&self) -> Result<synth_mcp::types::InputStateInfo, McpBridgeError> {
        let input = self.audio_input.lock();
        let state = match input.state() {
            crate::audio::input::InputState::Idle => "idle",
            crate::audio::input::InputState::Monitoring => "monitoring",
            crate::audio::input::InputState::Recording => "recording",
        };
        Ok(synth_mcp::types::InputStateInfo {
            state: state.to_string(),
            peak_level: input.peak_level(),
            recorded_seconds: input.recorded_seconds(),
            is_active: input.is_active(),
        })
    }

    fn set_input_device(&self, device_id: Option<String>) -> Result<(), McpBridgeError> {
        if self.audio_input.lock().state() != crate::audio::input::InputState::Idle {
            return Err(McpBridgeError::Other(
                "stop monitoring before changing the input device".to_string(),
            ));
        }
        let resolved_device_id = if let Some(requested) = device_id.as_deref() {
            let host = self.audio_input_host.lock();
            let host = host.as_ref().ok_or_else(|| {
                McpBridgeError::Other("audio input backend is unavailable".to_string())
            })?;
            let matched = host
                .devices()
                .map_err(|error| McpBridgeError::Other(error.to_string()))?
                .into_iter()
                .find(|device| device.id == requested || device.name == requested);
            let Some(device) = matched else {
                return Err(McpBridgeError::Other(format!(
                    "audio input device not found: '{requested}'"
                )));
            };
            Some(device.id)
        } else {
            None
        };
        *self.selected_input_device.lock() = resolved_device_id;
        Ok(())
    }

    fn start_monitoring(&self) -> Result<(), McpBridgeError> {
        let host = self.audio_input_host.lock();
        let host = host.as_ref().ok_or_else(|| {
            McpBridgeError::Other("audio input backend is unavailable".to_string())
        })?;
        let selected = self.selected_input_device.lock().clone();
        let config = synth_core::audio::StreamConfig {
            sample_rate: synth_core::audio::SampleRate::DVD_QUALITY,
            buffer_size: synth_core::audio::BufferSize::MEDIUM,
            channels: synth_core::audio::ChannelCount::Stereo,
        };
        let mut input = self.audio_input.lock();
        input
            .start_monitoring(host.as_ref(), selected.as_deref(), &config)
            .map_err(|error| McpBridgeError::Other(error.to_string()))?;
        let Some(consumer) = input.take_engine_consumer() else {
            input.stop_monitoring();
            return Err(McpBridgeError::Other(
                "audio input started without an engine consumer".to_string(),
            ));
        };
        if !self
            .session
            .command_sender()
            .send(EngineCommand::SetAudioInputConsumer { consumer })
        {
            input.stop_monitoring();
            return Err(McpBridgeError::CommandSendFailed {
                command: "start_monitoring",
            });
        }
        Ok(())
    }

    fn stop_monitoring(&self) -> Result<(), McpBridgeError> {
        if !self
            .session
            .command_sender()
            .send(EngineCommand::ClearAudioInputConsumer)
        {
            return Err(McpBridgeError::CommandSendFailed {
                command: "stop_monitoring",
            });
        }
        self.audio_input.lock().stop_monitoring();
        Ok(())
    }

    fn start_recording(&self) -> Result<(), McpBridgeError> {
        let mut input = self.audio_input.lock();
        if input.state() != crate::audio::input::InputState::Monitoring {
            return Err(McpBridgeError::Other(
                "start monitoring before recording".to_string(),
            ));
        }
        input.start_recording();
        Ok(())
    }

    fn stop_recording(
        &self,
        name: Option<String>,
    ) -> Result<synth_mcp::types::SampleInfo, McpBridgeError> {
        let mut input = self.audio_input.lock();
        let channels = input.channels();
        let sample_rate = input.sample_rate();
        let recorded_seconds = input.recorded_seconds();
        let data = input
            .stop_recording()
            .ok_or_else(|| McpBridgeError::Other("audio input is not recording".to_string()))?;
        let frame_count = data.len() / usize::from(channels.max(1));
        let recording_name = name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("Recording {recorded_seconds:.1}s"));
        drop(input);
        let sample = synth_sampler::Sample::new(
            synth_sampler::SampleMeta {
                id: synth_sampler::SampleId::new(0),
                name: recording_name,
                description: String::new(),
                sample_rate,
                channels: synth_core::ChannelCount::from(channels),
                frame_count: synth_core::SampleCount::new(frame_count),
                root_note: None,
                loop_region: None,
                crop: None,
                source: synth_sampler::SampleSource::Recorded,
            },
            data.into(),
        );
        let mut library = self
            .sample_library
            .write()
            .map_err(|_| McpBridgeError::Other("sample library lock poisoned".to_string()))?;
        let id = library.add(sample);
        let meta = &library
            .get(id)
            .ok_or_else(|| McpBridgeError::Other("recording commit failed".to_string()))?
            .meta;
        Ok(meta_to_sample_info(meta))
    }

    // === Discovery ===
}
