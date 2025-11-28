//! Comprehensive tests for the modular synthesizer.
//!
//! Test categories:
//! - Audio safety: no NaN, no infinity, output clamping
//! - Parameter handling: type-safe params, clamping, edge cases
//! - Module processing: oscillator, filter, envelope, effects
//! - Voice allocation: polyphony, stealing, mono/legato modes
//! - Graph processing: connections, topological sort, cycle detection

#[cfg(test)]
mod audio_safety {
    use crate::modules::{Oscillator, Filter, Envelope, AudioBuffer, ProcessContext, VoiceModule, EffectModule};
    use crate::effects::{Delay, Reverb, Chorus, Distortion};
    use std::collections::HashMap;

    fn make_context(samples: usize) -> ProcessContext {
        ProcessContext {
            sample_rate: 48000.0,
            samples,
            tempo: 120.0,
            is_playing: false,
            position_beats: 0.0,
        }
    }

    /// Helper to check if a buffer contains valid audio (no NaN or Inf).
    fn is_valid_audio(buffer: &[f32]) -> bool {
        buffer.iter().all(|&x| x.is_finite())
    }

    /// Helper to check if audio is within safe range.
    fn is_in_range(buffer: &[f32], min: f32, max: f32) -> bool {
        buffer.iter().all(|&x| x >= min && x <= max)
    }

    #[test]
    fn test_oscillator_no_nan() {
        let mut osc = Oscillator::new();
        let context = make_context(256);
        let inputs = HashMap::new();
        let mut outputs = HashMap::new();
        outputs.insert("out".to_string(), AudioBuffer::new(256));

        // Process multiple times to catch any state accumulation issues
        for _ in 0..100 {
            osc.process(&inputs, &mut outputs, &context);
            let out = outputs.get("out").unwrap();
            assert!(is_valid_audio(out.as_slice()), "Oscillator output contains NaN or Inf");
        }
    }

    #[test]
    fn test_oscillator_output_range() {
        let mut osc = Oscillator::new();
        let context = make_context(256);
        let inputs = HashMap::new();
        let mut outputs = HashMap::new();
        outputs.insert("out".to_string(), AudioBuffer::new(256));

        osc.process(&inputs, &mut outputs, &context);
        let out = outputs.get("out").unwrap();
        
        // Oscillator output should be in [-1, 1] range
        assert!(is_in_range(out.as_slice(), -1.1, 1.1), 
            "Oscillator output outside expected range");
    }

    #[test]
    fn test_filter_no_nan_with_extreme_resonance() {
        let mut filter = Filter::new();
        let context = make_context(256);
        
        // Set extreme resonance
        use crate::engine::typed_params::{TypedParam, TypedValue, FilterParam};
        filter.set_param(TypedParam::Filter(FilterParam::Resonance), TypedValue::Float(1.0));
        
        // Create input with impulse
        let mut input_buf = AudioBuffer::new(256);
        input_buf[0] = 1.0;
        
        let mut inputs = HashMap::new();
        inputs.insert("in".to_string(), &input_buf);
        
        let mut outputs = HashMap::new();
        outputs.insert("out".to_string(), AudioBuffer::new(256));

        // Process multiple times
        for _ in 0..100 {
            filter.process(&inputs, &mut outputs, &context);
            let out = outputs.get("out").unwrap();
            assert!(is_valid_audio(out.as_slice()), 
                "Filter output contains NaN or Inf with high resonance");
        }
    }

    #[test]
    fn test_envelope_no_nan() {
        let mut env = Envelope::new();
        let context = make_context(256);
        let inputs = HashMap::new();
        let mut outputs = HashMap::new();
        outputs.insert("out".to_string(), AudioBuffer::new(256));

        // Trigger note on
        env.note_on(60, 1.0);
        
        for _ in 0..50 {
            env.process(&inputs, &mut outputs, &context);
            let out = outputs.get("out").unwrap();
            assert!(is_valid_audio(out.as_slice()), "Envelope output contains NaN or Inf");
            assert!(is_in_range(out.as_slice(), 0.0, 1.1), "Envelope output outside [0, 1] range");
        }

        // Trigger note off
        env.note_off();
        
        for _ in 0..50 {
            env.process(&inputs, &mut outputs, &context);
            let out = outputs.get("out").unwrap();
            assert!(is_valid_audio(out.as_slice()), "Envelope output contains NaN or Inf during release");
        }
    }

    #[test]
    fn test_delay_no_nan() {
        let mut delay = Delay::new();
        delay.set_sample_rate(48000.0);
        let context = make_context(256);

        let mut input = vec![0.0f32; 512];
        let mut output = vec![0.0f32; 512];
        
        // Add impulse
        input[0] = 1.0;
        input[1] = 1.0;

        for _ in 0..100 {
            delay.process(&input, &mut output, &context);
            assert!(is_valid_audio(&output), "Delay output contains NaN or Inf");
        }
    }

    #[test]
    fn test_reverb_no_nan() {
        let mut reverb = Reverb::new();
        reverb.set_sample_rate(48000.0);
        let context = make_context(256);

        let mut input = vec![0.0f32; 512];
        let mut output = vec![0.0f32; 512];
        
        // Add impulse
        input[0] = 1.0;
        input[1] = 1.0;

        for _ in 0..100 {
            reverb.process(&input, &mut output, &context);
            assert!(is_valid_audio(&output), "Reverb output contains NaN or Inf");
        }
    }

    #[test]
    fn test_chorus_no_nan() {
        let mut chorus = Chorus::new();
        chorus.set_sample_rate(48000.0);
        let context = make_context(256);

        let mut input = vec![0.0f32; 512];
        let mut output = vec![0.0f32; 512];
        
        input[0] = 1.0;
        input[1] = 1.0;

        for _ in 0..100 {
            chorus.process(&input, &mut output, &context);
            assert!(is_valid_audio(&output), "Chorus output contains NaN or Inf");
        }
    }

    #[test]
    fn test_distortion_no_nan() {
        let mut dist = Distortion::new();
        dist.set_sample_rate(48000.0);
        let context = make_context(256);

        let mut input = vec![0.0f32; 512];
        let mut output = vec![0.0f32; 512];
        
        // Test with hot input
        for i in 0..256 {
            input[i * 2] = ((i as f32) * 0.1).sin() * 2.0;
            input[i * 2 + 1] = ((i as f32) * 0.1).sin() * 2.0;
        }

        for _ in 0..100 {
            dist.process(&input, &mut output, &context);
            assert!(is_valid_audio(&output), "Distortion output contains NaN or Inf");
        }
    }
}

#[cfg(test)]
mod parameter_handling {
    use crate::engine::typed_params::{TypedParam, TypedValue, OscillatorParam, FilterParam, EnvelopeParam};
    use crate::modules::{Oscillator, Filter, Envelope, VoiceModule, Waveform};

    #[test]
    fn test_oscillator_frequency_clamping() {
        let mut osc = Oscillator::new();

        // Try to set frequency below minimum
        osc.set_param(TypedParam::Oscillator(OscillatorParam::Frequency), TypedValue::Float(-100.0));
        if let Some(TypedValue::Float(f)) = osc.get_param(TypedParam::Oscillator(OscillatorParam::Frequency)) {
            assert!(f >= 20.0, "Frequency should be clamped to minimum");
        }

        // Try to set frequency above maximum
        osc.set_param(TypedParam::Oscillator(OscillatorParam::Frequency), TypedValue::Float(100000.0));
        if let Some(TypedValue::Float(f)) = osc.get_param(TypedParam::Oscillator(OscillatorParam::Frequency)) {
            assert!(f <= 20000.0, "Frequency should be clamped to maximum");
        }
    }

    #[test]
    fn test_filter_resonance_clamping() {
        let mut filter = Filter::new();

        // Try extreme values
        filter.set_param(TypedParam::Filter(FilterParam::Resonance), TypedValue::Float(10.0));
        if let Some(TypedValue::Float(r)) = filter.get_param(TypedParam::Filter(FilterParam::Resonance)) {
            assert!(r <= 1.0, "Resonance should be clamped to maximum 1.0");
        }

        filter.set_param(TypedParam::Filter(FilterParam::Resonance), TypedValue::Float(-1.0));
        if let Some(TypedValue::Float(r)) = filter.get_param(TypedParam::Filter(FilterParam::Resonance)) {
            assert!(r >= 0.0, "Resonance should be clamped to minimum 0.0");
        }
    }

    #[test]
    fn test_envelope_time_parameters() {
        let mut env = Envelope::new();

        // Attack should be clamped to reasonable range
        env.set_param(TypedParam::Envelope(EnvelopeParam::Attack), TypedValue::Float(-1.0));
        if let Some(TypedValue::Float(a)) = env.get_param(TypedParam::Envelope(EnvelopeParam::Attack)) {
            assert!(a >= 0.0, "Attack should not be negative");
        }

        // Very long attack should still work
        env.set_param(TypedParam::Envelope(EnvelopeParam::Attack), TypedValue::Float(100.0));
        if let Some(TypedValue::Float(a)) = env.get_param(TypedParam::Envelope(EnvelopeParam::Attack)) {
            assert!(a > 0.0 && a.is_finite(), "Attack should be valid");
        }
    }

    #[test]
    fn test_typed_param_api() {
        let mut osc = Oscillator::new();

        // Test waveform setting via typed API
        osc.set_param(
            TypedParam::Oscillator(OscillatorParam::Waveform),
            TypedValue::Waveform(Waveform::Square)
        );

        if let Some(TypedValue::Waveform(w)) = osc.get_param(
            TypedParam::Oscillator(OscillatorParam::Waveform)
        ) {
            assert_eq!(w, Waveform::Square);
        } else {
            panic!("Expected waveform value");
        }
    }

    #[test]
    fn test_typed_param_float_conversion() {
        let mut filter = Filter::new();

        // Set via typed API
        filter.set_param(
            TypedParam::Filter(FilterParam::Cutoff),
            TypedValue::Float(5000.0)
        );

        // Read back
        if let Some(TypedValue::Float(c)) = filter.get_param(
            TypedParam::Filter(FilterParam::Cutoff)
        ) {
            assert!((c - 5000.0).abs() < 0.01);
        } else {
            panic!("Expected float value");
        }
    }
}

#[cfg(test)]
mod voice_allocation {
    use crate::engine::{VoiceAllocator, AllocatorConfig, AllocationMode, StealingStrategy};

    #[test]
    fn test_polyphonic_allocation() {
        let config = AllocatorConfig {
            max_voices: 4,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(60, 0.8);
        allocator.note_on(64, 0.8);
        allocator.note_on(67, 0.8);

        assert_eq!(allocator.active_voice_count(), 3);
    }

    #[test]
    fn test_voice_stealing() {
        let config = AllocatorConfig {
            max_voices: 2,
            mode: AllocationMode::Polyphonic,
            stealing: StealingStrategy::Oldest,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(60, 0.8);
        allocator.note_on(64, 0.8);
        // This should steal the oldest voice
        allocator.note_on(67, 0.8);

        assert_eq!(allocator.active_voice_count(), 2);
    }

    #[test]
    fn test_mono_mode() {
        let config = AllocatorConfig {
            max_voices: 4,
            mode: AllocationMode::Mono,
            ..Default::default()
        };
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(60, 0.8);
        allocator.note_on(64, 0.8);
        allocator.note_on(67, 0.8);

        // Mono mode should only have one active voice
        assert_eq!(allocator.active_voice_count(), 1);
    }

    #[test]
    fn test_note_off() {
        let config = AllocatorConfig::default();
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(60, 0.8);
        assert_eq!(allocator.active_voice_count(), 1);

        allocator.note_off(60);
        // Voice should still be active during release
        // (actual check depends on implementation)
    }

    #[test]
    fn test_all_notes_off() {
        let config = AllocatorConfig::default();
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(60, 0.8);
        allocator.note_on(64, 0.8);
        allocator.note_on(67, 0.8);

        allocator.all_notes_off();

        // All voices should be in release or idle
    }

    #[test]
    fn test_panic() {
        let config = AllocatorConfig::default();
        let mut allocator = VoiceAllocator::new(config);

        allocator.note_on(60, 0.8);
        allocator.note_on(64, 0.8);

        allocator.panic();

        // All voices should be immediately silent
        assert_eq!(allocator.active_voice_count(), 0);
    }
}

#[cfg(test)]
mod module_graph {
    use crate::engine::{ModuleGraph, GraphError};
    use crate::modules::{Oscillator, Filter, Amplifier};

    #[test]
    fn test_graph_add_module() {
        let mut graph = ModuleGraph::new();
        
        let osc_id = graph.add_module(Box::new(Oscillator::new()));
        let filter_id = graph.add_module(Box::new(Filter::new()));
        
        assert!(graph.get_module(osc_id).is_some());
        assert!(graph.get_module(filter_id).is_some());
    }

    #[test]
    fn test_graph_remove_module() {
        let mut graph = ModuleGraph::new();
        
        let osc_id = graph.add_module(Box::new(Oscillator::new()));
        assert!(graph.get_module(osc_id).is_some());
        
        graph.remove_module(osc_id);
        assert!(graph.get_module(osc_id).is_none());
    }

    #[test]
    fn test_graph_connection() {
        let mut graph = ModuleGraph::new();
        
        let osc_id = graph.add_module(Box::new(Oscillator::new()));
        let filter_id = graph.add_module(Box::new(Filter::new()));
        
        let result = graph.connect(osc_id, "out", filter_id, "in");
        assert!(result.is_ok(), "Connection should succeed");
    }

    #[test]
    fn test_graph_disconnect() {
        let mut graph = ModuleGraph::new();
        
        let osc_id = graph.add_module(Box::new(Oscillator::new()));
        let filter_id = graph.add_module(Box::new(Filter::new()));
        
        graph.connect(osc_id, "out", filter_id, "in").unwrap();
        graph.disconnect(osc_id, "out", filter_id, "in");
        
        // Verify disconnected (no direct API, but should not crash)
    }

    #[test]
    fn test_graph_invalid_connection() {
        let mut graph = ModuleGraph::new();
        
        let osc_id = graph.add_module(Box::new(Oscillator::new()));
        let filter_id = graph.add_module(Box::new(Filter::new()));
        
        // Try to connect non-existent ports
        let result = graph.connect(osc_id, "nonexistent", filter_id, "in");
        assert!(result.is_err(), "Invalid port connection should fail");
    }

    #[test]
    fn test_graph_self_loop_rejected() {
        let mut graph = ModuleGraph::new();
        
        let osc_id = graph.add_module(Box::new(Oscillator::new()));
        
        // Try to connect module to itself
        let result = graph.connect(osc_id, "out", osc_id, "fm");
        assert!(matches!(result, Err(GraphError::CycleDetected)), 
            "Self-loop should be rejected as cycle");
    }

    #[test]
    fn test_graph_cycle_detection() {
        let mut graph = ModuleGraph::new();
        
        let a = graph.add_module(Box::new(Oscillator::new()));
        let b = graph.add_module(Box::new(Filter::new()));
        let c = graph.add_module(Box::new(Amplifier::new()));
        
        // Create chain: A -> B -> C
        graph.connect(a, "out", b, "in").unwrap();
        graph.connect(b, "out", c, "in").unwrap();
        
        // Try to create cycle: C -> A (should fail)
        let result = graph.connect(c, "left", a, "fm");
        assert!(matches!(result, Err(GraphError::CycleDetected)), 
            "Cycle should be detected and rejected");
    }

    #[test]
    fn test_graph_no_false_cycle_detection() {
        let mut graph = ModuleGraph::new();
        
        let a = graph.add_module(Box::new(Oscillator::new()));
        let b = graph.add_module(Box::new(Filter::new()));
        let c = graph.add_module(Box::new(Amplifier::new()));
        
        // Create: A -> B, A -> C (fan-out, no cycle)
        graph.connect(a, "out", b, "in").unwrap();
        let result = graph.connect(a, "out", c, "in");
        assert!(result.is_ok(), "Fan-out should not be detected as cycle");
    }
}

#[cfg(test)]
mod engine_integration {
    use crate::engine::SynthEngine;
    use crate::audio::{AudioProcessor, AudioCallbackContext, StreamInfo, SampleRate, BufferSize, ChannelCount};
    use std::time::Duration;

    fn make_stream_info() -> StreamInfo {
        StreamInfo {
            sample_rate: SampleRate(48000),
            channels: ChannelCount::Stereo,
            buffer_size: BufferSize(256),
            output_latency: Duration::from_millis(5),
            input_latency: None,
        }
    }

    fn make_callback_context() -> AudioCallbackContext {
        AudioCallbackContext {
            sample_rate: SampleRate(48000),
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: 0.005,
        }
    }

    #[test]
    fn test_engine_creation() {
        let (_engine, handle) = SynthEngine::new();
        assert_eq!(handle.voice_count(), 0);
        assert!((handle.master_volume() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_engine_note_on_off() {
        let (_engine, mut handle) = SynthEngine::new();

        handle.note_on(60, 0.8);
        handle.note_off(60);
        // Commands are queued, processing happens in audio thread
    }

    #[test]
    fn test_engine_process_audio() {
        let (mut engine, mut handle) = SynthEngine::new();

        // Start stream
        let info = make_stream_info();
        engine.on_stream_start(&info);

        handle.note_on(60, 0.8);

        // Process some audio
        let mut output = vec![0.0f32; 512];
        let context = make_callback_context();

        engine.process(&mut output, &context);

        // All samples should be valid
        assert!(output.iter().all(|&x| x.is_finite()), "Output should not contain NaN or Inf");

        // All samples should be in valid range
        assert!(output.iter().all(|&x| x >= -1.0 && x <= 1.0),
            "Output should be clamped to [-1, 1]");
    }

    #[test]
    fn test_engine_master_volume() {
        let (mut engine, mut handle) = SynthEngine::new();

        // Start stream so engine can process commands
        let info = make_stream_info();
        engine.on_stream_start(&info);

        handle.set_master_volume(0.5);

        // Process audio to trigger command processing
        let mut output = vec![0.0f32; 512];
        let context = make_callback_context();
        engine.process(&mut output, &context);

        assert!((handle.master_volume() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_engine_parameter_changes() {
        use crate::engine::commands::VoiceModule;
        use crate::engine::typed_params::{TypedParam, TypedValue, OscillatorParam};
        use crate::modules::Waveform;

        let (_engine, mut handle) = SynthEngine::new();

        // Change oscillator waveform
        handle.set_voice_parameter(
            VoiceModule::Oscillator1,
            TypedParam::Oscillator(OscillatorParam::Waveform),
            TypedValue::Waveform(Waveform::Square),
        );
        // Parameter should be queued for application
    }
}

#[cfg(test)]
mod audio_buffer {
    use crate::modules::AudioBuffer;

    #[test]
    fn test_buffer_creation() {
        let buf = AudioBuffer::new(256);
        assert_eq!(buf.len(), 256);
        assert!(buf.as_slice().iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_buffer_resize() {
        let mut buf = AudioBuffer::new(128);
        buf.resize(256);
        assert_eq!(buf.len(), 256);
    }

    #[test]
    fn test_buffer_clear() {
        let mut buf = AudioBuffer::new(256);
        buf[0] = 1.0;
        buf[100] = 0.5;
        buf.clear();
        assert!(buf.as_slice().iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_buffer_copy() {
        let mut src = AudioBuffer::new(256);
        src[0] = 1.0;
        src[100] = 0.5;

        let mut dst = AudioBuffer::new(256);
        dst.copy_from(&src);

        assert_eq!(dst[0], 1.0);
        assert_eq!(dst[100], 0.5);
    }
}
