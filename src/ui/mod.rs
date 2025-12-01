//! UI abstraction layer.
//!
//! This module provides a framework-agnostic interface for building
//! synthesizer user interfaces. It defines traits and types that can
//! be implemented by different UI frameworks (egui, iced, vizia, etc.).

use crate::engine::{EngineHandle, ModuleId};
use crate::engine::typed_params::Param;
use crate::modules::{ModuleDescriptor, ParameterDescriptor, WidgetHint};
use crate::types::{Gain, NormalizedValue};

// ============================================================================
// UI Events
// ============================================================================

/// Events that the UI can send to the synth engine.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Trigger a note on.
    NoteOn { note: u8, velocity: NormalizedValue },
    /// Trigger a note off.
    NoteOff { note: u8 },
    /// All notes off.
    AllNotesOff,
    /// Change a parameter value.
    /// The Param carries both the parameter type and its value.
    ParameterChange {
        module: ModuleId,
        param: Param,
    },
    /// Set master volume.
    SetMasterVolume(Gain),
    /// Panic - kill all voices immediately.
    Panic,
}

/// Events that the synth engine sends to the UI.
#[derive(Debug, Clone)]
pub enum SynthEvent {
    /// Peak meter update.
    PeakMeter { left: f32, right: f32 },
    /// RMS meter update.
    RmsMeter { left: f32, right: f32 },
    /// Voice count changed.
    VoiceCount(u32),
    /// CPU usage update.
    CpuUsage(f32),
    /// Buffer underrun occurred.
    BufferUnderrun,
}

// ============================================================================
// UI State
// ============================================================================

/// Shared state accessible by the UI.
/// 
/// This struct provides a clean interface between the audio engine
/// and any UI implementation.
pub struct UiState {
    /// Handle to the audio engine.
    handle: EngineHandle,
    /// Cached peak meters.
    peak_left: f32,
    peak_right: f32,
    /// Cached RMS meters.
    rms_left: f32,
    rms_right: f32,
    /// Current voice count.
    voice_count: u32,
    /// Current CPU usage.
    cpu_usage: f32,
}

impl UiState {
    /// Create a new UI state from an engine handle.
    pub fn new(handle: EngineHandle) -> Self {
        Self {
            handle,
            peak_left: 0.0,
            peak_right: 0.0,
            rms_left: 0.0,
            rms_right: 0.0,
            voice_count: 0,
            cpu_usage: 0.0,
        }
    }

    /// Update cached values from the engine state.
    /// Call this once per frame.
    pub fn update(&mut self) {
        let (pl, pr) = self.handle.peak_meters();
        self.peak_left = pl;
        self.peak_right = pr;

        let (rl, rr) = self.handle.rms_meters();
        self.rms_left = rl;
        self.rms_right = rr;

        self.voice_count = self.handle.voice_count();
        self.cpu_usage = self.handle.cpu_usage();
    }

    /// Get peak meter values.
    pub fn peak_meters(&self) -> (f32, f32) {
        (self.peak_left, self.peak_right)
    }

    /// Get RMS meter values.
    pub fn rms_meters(&self) -> (f32, f32) {
        (self.rms_left, self.rms_right)
    }

    /// Get current voice count.
    pub fn voice_count(&self) -> u32 {
        self.voice_count
    }

    /// Get CPU usage (0.0 - 1.0).
    pub fn cpu_usage(&self) -> f32 {
        self.cpu_usage
    }

    /// Get master volume.
    pub fn master_volume(&self) -> f32 {
        self.handle.master_volume()
    }

    /// Send a note on event.
    pub fn note_on(&mut self, note: u8, velocity: NormalizedValue) {
        self.handle.note_on(note, velocity);
    }

    /// Send a note off event.
    pub fn note_off(&mut self, note: u8) {
        self.handle.note_off(note);
    }

    /// Set master volume.
    pub fn set_master_volume(&mut self, volume: Gain) {
        self.handle.set_master_volume(volume);
    }

    // NOTE: Legacy set_parameter removed - use typed API instead:
    // - For voice modules: handle.send(SetVoiceParameter { target, param, value })
    // - For effects: handle.send(SetEffectParameter { effect_type, param, value })

    /// Process a UI event.
    pub fn process_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::NoteOn { note, velocity } => {
                self.handle.note_on(note, velocity);
            }
            UiEvent::NoteOff { note } => {
                self.handle.note_off(note);
            }
            UiEvent::AllNotesOff => {
                self.handle.send(crate::engine::EngineCommand::AllNotesOff);
            }
            UiEvent::ParameterChange { module: _, param: _ } => {
                // Legacy parameter change - not supported
                // Use typed API: SetVoiceParameter or SetEffectParameter
                eprintln!("Warning: Legacy ParameterChange event not supported");
            }
            UiEvent::SetMasterVolume(vol) => {
                self.handle.set_master_volume(vol);
            }
            UiEvent::Panic => {
                self.handle.send(crate::engine::EngineCommand::Reset);
            }
        }
    }

    /// Poll for engine events.
    pub fn poll_events(&mut self) -> Vec<SynthEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.handle.poll_event() {
            match event {
                crate::engine::EngineEvent::PeakMeter { left, right } => {
                    events.push(SynthEvent::PeakMeter { left, right });
                }
                crate::engine::EngineEvent::BufferUnderrun => {
                    events.push(SynthEvent::BufferUnderrun);
                }
                _ => {}
            }
        }
        events
    }
}

// ============================================================================
// Widget helpers
// ============================================================================

/// Helper for building parameter widgets from descriptors.
pub struct ParameterWidget<'a> {
    pub descriptor: &'a ParameterDescriptor,
    /// Current value as f32 (use Param::as_f32() to get this).
    pub current_value: f32,
}

impl<'a> ParameterWidget<'a> {
    pub fn new(descriptor: &'a ParameterDescriptor, current_value: f32) -> Self {
        Self {
            descriptor,
            current_value,
        }
    }

    /// Get the current value as a normalized float (0.0 - 1.0).
    pub fn normalized_value(&self) -> f32 {
        (self.current_value - self.descriptor.min)
            / (self.descriptor.max - self.descriptor.min)
    }

    /// Convert a normalized value back to the parameter's actual value (as f32).
    pub fn denormalize(&self, normalized: f32) -> f32 {
        let clamped = normalized.clamp(0.0, 1.0);

        if self.descriptor.choices.is_some() {
            // For choice parameters, return as index
            let choices = self.descriptor.choices.as_ref().unwrap();
            let index = (clamped * (choices.len() - 1) as f32).round() as usize;
            index.min(choices.len() - 1) as f32
        } else {
            // For continuous parameters, apply response curve
            self.descriptor.response_curve.denormalize(
                clamped,
                self.descriptor.min,
                self.descriptor.max,
            )
        }
    }

    /// Get suggested widget type.
    pub fn widget_hint(&self) -> WidgetHint {
        self.descriptor.widget_hint
    }

    /// Get display name.
    pub fn name(&self) -> &str {
        &self.descriptor.name
    }

    /// Get description (for tooltips).
    pub fn description(&self) -> &str {
        &self.descriptor.description
    }

    /// Get choice options (if this is a choice parameter).
    pub fn choices(&self) -> Option<&Vec<crate::modules::ChoiceOption>> {
        self.descriptor.choices.as_ref()
    }

    /// Format the current value as a display string.
    pub fn format_value(&self) -> String {
        // Check if it's a choice parameter
        if let Some(choices) = &self.descriptor.choices {
            let idx = self.current_value as usize;
            if idx < choices.len() {
                return choices[idx].name.clone();
            }
            return format!("{}", idx);
        }

        // Format float value with appropriate unit
        let v = self.current_value;
        let unit = match self.descriptor.unit {
            crate::modules::ParameterUnit::Hertz => " Hz",
            crate::modules::ParameterUnit::Decibels => " dB",
            crate::modules::ParameterUnit::Percent => "%",
            crate::modules::ParameterUnit::Seconds => " s",
            crate::modules::ParameterUnit::Milliseconds => " ms",
            crate::modules::ParameterUnit::Semitones => " st",
            crate::modules::ParameterUnit::Cents => " cents",
            crate::modules::ParameterUnit::Octaves => " oct",
            crate::modules::ParameterUnit::Beats => " beats",
            crate::modules::ParameterUnit::BeatsPerMinute => " BPM",
            crate::modules::ParameterUnit::Samples => " smp",
            crate::modules::ParameterUnit::Ratio => ":1",
            crate::modules::ParameterUnit::None => "",
        };

        // Format based on range
        if self.descriptor.max - self.descriptor.min > 100.0 {
            format!("{:.0}{}", v, unit)
        } else if self.descriptor.max - self.descriptor.min > 10.0 {
            format!("{:.1}{}", v, unit)
        } else {
            format!("{:.2}{}", v, unit)
        }
    }
}

// ============================================================================
// Module panel helpers
// ============================================================================

/// Helper for building module UI panels from descriptors.
pub struct ModulePanel<'a> {
    pub descriptor: &'a ModuleDescriptor,
    pub module_id: ModuleId,
}

impl<'a> ModulePanel<'a> {
    pub fn new(descriptor: &'a ModuleDescriptor, module_id: ModuleId) -> Self {
        Self {
            descriptor,
            module_id,
        }
    }

    /// Get module name.
    pub fn name(&self) -> &str {
        &self.descriptor.name
    }

    /// Get module description.
    pub fn description(&self) -> &str {
        &self.descriptor.description
    }

    /// Get all parameter descriptors.
    pub fn parameters(&self) -> &[ParameterDescriptor] {
        &self.descriptor.parameters
    }

    /// Get category.
    pub fn category(&self) -> crate::modules::ModuleCategory {
        self.descriptor.category
    }
}
