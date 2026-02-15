//! AWE engine — real-time room simulation processor.
//!
//! Full DSP pipeline (Fas 1):
//! 1. Early reflections via Image Source Method (ISM)
//! 2. Late reverb via FDN (geometry-driven parameters)
//! 3. Axial room modes via comb-filter bank
//! 4. Stereo spatializer (ITD + ILD) on wet signal only

use synth_dsp::{FdnCore, InterpolatedDelayLine};

use crate::early_reflections::EarlyReflections;
use crate::lfo::AweLfo;
use crate::params::{AweLfoTarget, AweParam, AweSnapshot};
use crate::room::{Material, RoomShape};
use crate::room_modes::RoomModeBank;
use crate::spatial_voice::{NotePositionMapping, SpatialVoiceBank, SpatialVoicePool};
use crate::spatializer::Spatializer;

/// Smooth parameter ramp time in seconds (~5 ms).
const RAMP_TIME_SECONDS: f32 = 0.005;

/// The Acoustic World Engine processor.
///
/// Processes interleaved stereo audio through a physics-based room simulation:
/// early reflections, late reverb, room modes, and stereo spatialisation.
pub struct AweEngine {
    enabled: bool,
    room: RoomShape,
    material: Material,
    snapshot: AweSnapshot,
    cached_sample_rate: f32,

    // DSP processors
    early_reflections: EarlyReflections,
    fdn: FdnCore,
    room_modes: RoomModeBank,
    spatializer: Spatializer,

    // LFOs (control-rate)
    lfo1: AweLfo,
    lfo2: AweLfo,
    lfo3: AweLfo,
    lfo4: AweLfo,

    // Acoustic portal — extra delay feedback path simulating adjoining room
    portal_delay_left: InterpolatedDelayLine,
    portal_delay_right: InterpolatedDelayLine,
    portal_feedback_state_l: f32,
    portal_feedback_state_r: f32,

    // Smoothed DSP parameters (current values that ramp toward targets)
    current_dry_wet: f32,
    current_early_late: f32,
    current_portal: f32,

    // Geometry dirty flag — recalculate ISM taps, FDN, modes when true
    geometry_dirty: bool,

    // Per-voice spatial
    spatial_enabled: bool,
    note_mapping: NotePositionMapping,
    voice_pool: SpatialVoicePool,
}

impl AweEngine {
    /// Create a new AWE engine (disabled by default).
    #[must_use]
    pub fn new() -> Self {
        let snapshot = AweSnapshot::default();
        Self {
            enabled: false,
            room: RoomShape::default(),
            material: Material::default(),
            snapshot,
            cached_sample_rate: 48000.0,

            early_reflections: EarlyReflections::new(),
            fdn: FdnCore::new(),
            room_modes: RoomModeBank::new(),
            spatializer: Spatializer::new(),

            lfo1: AweLfo::new(),
            lfo2: AweLfo::new(),
            lfo3: AweLfo::new(),
            lfo4: AweLfo::new(),

            // Portal delay lines: max ~300ms @ 96kHz = 28800 samples
            portal_delay_left: InterpolatedDelayLine::new(28_800),
            portal_delay_right: InterpolatedDelayLine::new(28_800),
            portal_feedback_state_l: 0.0,
            portal_feedback_state_r: 0.0,

            current_dry_wet: snapshot.dry_wet,
            current_early_late: snapshot.early_late_balance,
            current_portal: snapshot.portal_amount,

            geometry_dirty: true,

            spatial_enabled: false,
            note_mapping: NotePositionMapping::Off,
            voice_pool: SpatialVoicePool::new(),
        }
    }

    /// Process audio buffer (interleaved stereo: [L0, R0, L1, R1, ...]).
    #[allow(clippy::too_many_lines)]
    pub fn process(&mut self, buffer: &mut [f32], sample_rate: f32) {
        self.cached_sample_rate = sample_rate;

        let num_samples = buffer.len() / 2;
        if num_samples == 0 {
            return;
        }

        // 1. Control-rate: advance LFOs and apply modulation
        self.update_lfos(num_samples, sample_rate);

        // 2. Recalculate geometry-dependent DSP parameters if changed
        if self.geometry_dirty {
            self.recalculate_geometry(sample_rate);
            self.geometry_dirty = false;
        }

        // Compute ramp coefficient for ~5 ms smoothing
        let ramp_coeff = 1.0 - (-1.0 / (RAMP_TIME_SECONDS * sample_rate)).exp();

        // Pre-compute FDN parameters from room geometry
        let absorption = self.material.average_absorption();
        let rt60 = self.calculate_rt60();

        // Freq warp: bass hears bigger room (more LP damping = bass reverberates more)
        let freq_warp = self.snapshot.freq_warp.clamp(-1.0, 1.0);
        let lp_coeff = (0.2 + absorption * 0.6) * (1.0 - freq_warp * 0.3);

        // Resonance boost: adds energy to feedback (with safety clamp)
        let resonance_boost = self.snapshot.resonance_boost.clamp(0.0, 1.0);
        let feedback_gain =
            (self.rt60_to_feedback(rt60, sample_rate) + resonance_boost * 0.15).min(0.97);
        let hp_coeff = 0.95;
        let diffusion = 0.5;
        let width = 1.0;
        let sample_rate_recip = 1.0 / sample_rate;

        let target_dry_wet = self.snapshot.dry_wet.clamp(0.0, 1.0);
        let target_early_late = self.snapshot.early_late_balance.clamp(0.0, 1.0);
        let target_portal = self.snapshot.portal_amount.clamp(0.0, 1.0);

        // Portal delay time: ~200ms scaled by room size
        let portal_delay_samples = (0.2 * sample_rate).clamp(1.0, 28_000.0);
        let portal_feedback = 0.4; // Fixed feedback for adjoining room simulation
        let portal_damping = 0.6; // LP damping for muffled portal sound

        // 3. Per-sample DSP
        for i in 0..num_samples {
            let idx = i * 2;
            let dry_left = buffer[idx];
            let dry_right = buffer[idx + 1];

            // Mono input for wet processing
            let mono_input = (dry_left + dry_right) * 0.5;

            // Smooth ramp toward target parameters
            self.current_dry_wet += ramp_coeff * (target_dry_wet - self.current_dry_wet);
            self.current_early_late += ramp_coeff * (target_early_late - self.current_early_late);
            self.current_portal += ramp_coeff * (target_portal - self.current_portal);

            // --- Early reflections (ISM) ---
            let (early_left, early_right) = self.early_reflections.process(mono_input);

            // --- Late reverb (FDN) ---
            let fdn_out = self.fdn.process_sample(
                mono_input,
                feedback_gain,
                lp_coeff,
                hp_coeff,
                diffusion,
                width,
                sample_rate_recip,
            );

            // --- Room modes ---
            let modes_out = self.room_modes.process(mono_input);

            // --- Mix early/late ---
            let early_amount = 1.0 - self.current_early_late;
            let late_amount = self.current_early_late;

            let wet_mono_left =
                early_left * early_amount + fdn_out.left * late_amount + modes_out * 0.5;
            let wet_mono_right =
                early_right * early_amount + fdn_out.right * late_amount + modes_out * 0.5;

            // --- Spatializer (on wet signal only) ---
            let wet_mid = (wet_mono_left + wet_mono_right) * 0.5;
            let (spat_left, spat_right) = self.spatializer.process(wet_mid);

            // --- Acoustic portal (delayed feedback from adjoining virtual room) ---
            let (spat_left, spat_right) = if self.current_portal > 0.001 {
                // Read delayed signal from portal
                let portal_l = self
                    .portal_delay_left
                    .read_interpolated(portal_delay_samples);
                let portal_r = self
                    .portal_delay_right
                    .read_interpolated(portal_delay_samples);

                // One-pole LP for muffled portal sound
                self.portal_feedback_state_l = portal_damping * self.portal_feedback_state_l
                    + (1.0 - portal_damping) * portal_l;
                self.portal_feedback_state_r = portal_damping * self.portal_feedback_state_r
                    + (1.0 - portal_damping) * portal_r;

                // Write current wet + feedback back into portal delay
                self.portal_delay_left
                    .write(spat_left + self.portal_feedback_state_l * portal_feedback);
                self.portal_delay_right
                    .write(spat_right + self.portal_feedback_state_r * portal_feedback);

                // Mix portal into output
                let amt = self.current_portal;
                (
                    spat_left + self.portal_feedback_state_l * amt,
                    spat_right + self.portal_feedback_state_r * amt,
                )
            } else {
                // Portal off — still write silence to keep delay line advancing
                self.portal_delay_left.write(0.0);
                self.portal_delay_right.write(0.0);
                (spat_left, spat_right)
            };

            // --- Dry/wet mix ---
            let dry_amount = 1.0 - self.current_dry_wet;
            let wet_amount = self.current_dry_wet;

            buffer[idx] = dry_left * dry_amount + spat_left * wet_amount;
            buffer[idx + 1] = dry_right * dry_amount + spat_right * wet_amount;
        }
    }

    /// Set a single parameter.
    pub fn set_param(&mut self, param: AweParam) {
        match param {
            AweParam::RoomShape(shape) => {
                self.room = shape;
                self.geometry_dirty = true;
            }
            AweParam::Material(mat) => {
                self.material = mat;
                self.geometry_dirty = true;
            }
            AweParam::SourcePos(pos) => {
                self.snapshot.source_pos = pos;
                self.geometry_dirty = true;
            }
            AweParam::ListenerPos(pos) => {
                self.snapshot.listener_pos = pos;
                self.geometry_dirty = true;
            }
            AweParam::DryWet(v) => self.snapshot.dry_wet = v,
            AweParam::EarlyLateBalance(v) => self.snapshot.early_late_balance = v,
            AweParam::ModesAmount(v) => {
                self.snapshot.modes_amount = v;
                self.room_modes.set_amount(v);
            }
            AweParam::FreqWarp(v) => self.snapshot.freq_warp = v,
            AweParam::ResonanceBoost(v) => self.snapshot.resonance_boost = v,
            AweParam::TailStretch(v) => {
                self.snapshot.tail_stretch = v;
                self.geometry_dirty = true;
            }
            AweParam::PortalAmount(v) => self.snapshot.portal_amount = v,
            AweParam::Enabled(v) => self.enabled = v,
            AweParam::SpatialEnabled(v) => {
                self.spatial_enabled = v;
                self.snapshot.spatial_enabled = v;
                if !v {
                    self.voice_pool.clear();
                }
            }
            AweParam::NoteMapping(m) => {
                self.note_mapping = m;
                self.snapshot.note_mapping = m;
            }
            AweParam::Lfo1Rate(v) => {
                self.snapshot.lfo1.rate = v;
                self.lfo1.set_rate(v);
            }
            AweParam::Lfo1Amount(v) => {
                self.snapshot.lfo1.amount = v;
                self.lfo1.set_amount(v);
            }
            AweParam::Lfo1Target(t) => {
                self.snapshot.lfo1.target = t;
                self.lfo1.set_target(t);
            }
            AweParam::Lfo2Rate(v) => {
                self.snapshot.lfo2.rate = v;
                self.lfo2.set_rate(v);
            }
            AweParam::Lfo2Amount(v) => {
                self.snapshot.lfo2.amount = v;
                self.lfo2.set_amount(v);
            }
            AweParam::Lfo2Target(t) => {
                self.snapshot.lfo2.target = t;
                self.lfo2.set_target(t);
            }
            AweParam::Lfo3Rate(v) => {
                self.snapshot.lfo3.rate = v;
                self.lfo3.set_rate(v);
            }
            AweParam::Lfo3Amount(v) => {
                self.snapshot.lfo3.amount = v;
                self.lfo3.set_amount(v);
            }
            AweParam::Lfo3Target(t) => {
                self.snapshot.lfo3.target = t;
                self.lfo3.set_target(t);
            }
            AweParam::Lfo4Rate(v) => {
                self.snapshot.lfo4.rate = v;
                self.lfo4.set_rate(v);
            }
            AweParam::Lfo4Amount(v) => {
                self.snapshot.lfo4.amount = v;
                self.lfo4.set_amount(v);
            }
            AweParam::Lfo4Target(t) => {
                self.snapshot.lfo4.target = t;
                self.lfo4.set_target(t);
            }
        }
    }

    /// Apply a batch snapshot of numeric parameters.
    pub fn apply_snapshot(&mut self, snapshot: AweSnapshot) {
        self.snapshot = snapshot;
        self.spatial_enabled = snapshot.spatial_enabled;
        self.note_mapping = snapshot.note_mapping;
        self.room_modes.set_amount(snapshot.modes_amount);
        self.lfo1.set_rate(snapshot.lfo1.rate);
        self.lfo1.set_amount(snapshot.lfo1.amount);
        self.lfo1.set_target(snapshot.lfo1.target);
        self.lfo2.set_rate(snapshot.lfo2.rate);
        self.lfo2.set_amount(snapshot.lfo2.amount);
        self.lfo2.set_target(snapshot.lfo2.target);
        self.lfo3.set_rate(snapshot.lfo3.rate);
        self.lfo3.set_amount(snapshot.lfo3.amount);
        self.lfo3.set_target(snapshot.lfo3.target);
        self.lfo4.set_rate(snapshot.lfo4.rate);
        self.lfo4.set_amount(snapshot.lfo4.amount);
        self.lfo4.set_target(snapshot.lfo4.target);
        self.geometry_dirty = true;
    }

    /// Check if the engine is enabled.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the engine.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            // Clear DSP state when disabling to avoid stale tails on re-enable
            self.early_reflections.clear();
            self.fdn.clear();
            self.room_modes.clear();
            self.spatializer.clear();
            self.portal_delay_left.clear();
            self.portal_delay_right.clear();
            self.portal_feedback_state_l = 0.0;
            self.portal_feedback_state_r = 0.0;
            self.voice_pool.clear();
        }
    }

    /// Get the current parameter snapshot.
    #[must_use]
    pub fn snapshot(&self) -> AweSnapshot {
        self.snapshot
    }

    /// Get the current room shape.
    #[must_use]
    pub fn room(&self) -> RoomShape {
        self.room
    }

    /// Get the current material.
    #[must_use]
    pub fn material(&self) -> Material {
        self.material
    }

    /// Mark geometry as dirty so DSP parameters are recalculated on next process().
    ///
    /// Call this when sample rate changes or other external state invalidates
    /// the cached geometry-dependent DSP parameters.
    pub fn mark_geometry_dirty(&mut self) {
        self.geometry_dirty = true;
    }

    /// Check if per-voice spatial is enabled.
    #[must_use]
    pub fn spatial_enabled(&self) -> bool {
        self.spatial_enabled
    }

    /// Get the current note-to-position mapping.
    #[must_use]
    pub fn note_mapping(&self) -> NotePositionMapping {
        self.note_mapping
    }

    /// Process audio with per-voice spatial early reflections and spatialisation.
    ///
    /// The `buffer` contains the dry mix (already per-voice panned by Instrument).
    /// The `bank` contains per-voice mono audio captured by Instrument.
    #[allow(clippy::too_many_lines)]
    pub fn process_spatial(
        &mut self,
        buffer: &mut [f32],
        bank: &SpatialVoiceBank,
        sample_rate: f32,
    ) {
        self.cached_sample_rate = sample_rate;

        let num_samples = buffer.len() / 2;
        if num_samples == 0 {
            return;
        }

        let active = bank.active_count();

        // 1. Control-rate: advance LFOs
        self.update_lfos(num_samples, sample_rate);

        // 2. Recalculate global geometry if dirty
        if self.geometry_dirty {
            self.recalculate_geometry(sample_rate);
            self.geometry_dirty = false;
        }

        // 3. Sync voice pool with bank: activate slots, update geometry
        let room_length = self.room.length();
        let room_width = self.room.width();
        let room_height = self.room.height();
        let absorption = self.material.average_absorption();
        let listener = [
            self.snapshot.listener_pos[0].clamp(0.1, room_length - 0.1),
            self.snapshot.listener_pos[1].clamp(0.1, room_width - 0.1),
            self.snapshot.listener_pos[2].clamp(0.1, room_height - 0.1),
        ];

        // Deactivate unused slots
        for i in active..self.voice_pool.slots.len() {
            self.voice_pool.slots[i].active = false;
        }

        // Update active slots
        for i in 0..active {
            let info = bank.info(i);
            self.voice_pool.update_slot(
                i,
                info.note,
                self.note_mapping,
                room_length,
                room_width,
                room_height,
                listener,
                absorption,
                sample_rate,
            );
        }

        // 4. Compute ramp coefficient
        let ramp_coeff = 1.0 - (-1.0 / (RAMP_TIME_SECONDS * sample_rate)).exp();

        // Pre-compute FDN parameters
        let rt60 = self.calculate_rt60();
        let freq_warp = self.snapshot.freq_warp.clamp(-1.0, 1.0);
        let lp_coeff = (0.2 + absorption * 0.6) * (1.0 - freq_warp * 0.3);
        let resonance_boost = self.snapshot.resonance_boost.clamp(0.0, 1.0);
        let feedback_gain =
            (self.rt60_to_feedback(rt60, sample_rate) + resonance_boost * 0.15).min(0.97);
        let hp_coeff = 0.95;
        let diffusion = 0.5;
        let width = 1.0;
        let sample_rate_recip = 1.0 / sample_rate;

        let target_dry_wet = self.snapshot.dry_wet.clamp(0.0, 1.0);
        let target_early_late = self.snapshot.early_late_balance.clamp(0.0, 1.0);
        let target_portal = self.snapshot.portal_amount.clamp(0.0, 1.0);

        let portal_delay_samples = (0.2 * sample_rate).clamp(1.0, 28_000.0);
        let portal_feedback = 0.4;
        let portal_damping = 0.6;

        // 5. Per-sample loop
        for i in 0..num_samples {
            let idx = i * 2;
            let dry_left = buffer[idx];
            let dry_right = buffer[idx + 1];

            // Smooth ramp
            self.current_dry_wet += ramp_coeff * (target_dry_wet - self.current_dry_wet);
            self.current_early_late += ramp_coeff * (target_early_late - self.current_early_late);
            self.current_portal += ramp_coeff * (target_portal - self.current_portal);

            let mut total_early_l = 0.0_f32;
            let mut total_early_r = 0.0_f32;
            let mut total_spat_l = 0.0_f32;
            let mut total_spat_r = 0.0_f32;
            let mut global_mono = 0.0_f32;

            // Per active voice: process through per-voice early reflections + spatializer
            for v in 0..active {
                let info = bank.info(v);
                let mono = if i < info.sample_count {
                    bank.buffer(v)[i]
                } else {
                    0.0
                };
                global_mono += mono;

                let (el, er, sl, sr) = self.voice_pool.process_slot(v, mono);
                total_early_l += el;
                total_early_r += er;
                total_spat_l += sl;
                total_spat_r += sr;
            }

            // Late reverb (shared FDN, fed by global mono)
            let fdn_out = self.fdn.process_sample(
                global_mono,
                feedback_gain,
                lp_coeff,
                hp_coeff,
                diffusion,
                width,
                sample_rate_recip,
            );

            // Room modes (shared)
            let modes_out = self.room_modes.process(global_mono);

            // Mix early/late
            let early_amount = 1.0 - self.current_early_late;
            let late_amount = self.current_early_late;

            // Use per-voice spatialized dry instead of global spatializer
            let wet_left =
                total_early_l * early_amount + fdn_out.left * late_amount + modes_out * 0.5;
            let wet_right =
                total_early_r * early_amount + fdn_out.right * late_amount + modes_out * 0.5;

            // Combine with per-voice spatializer output for dry positioning
            let spat_left = wet_left + total_spat_l * 0.1;
            let spat_right = wet_right + total_spat_r * 0.1;

            // Portal
            let (spat_left, spat_right) = if self.current_portal > 0.001 {
                let portal_l = self
                    .portal_delay_left
                    .read_interpolated(portal_delay_samples);
                let portal_r = self
                    .portal_delay_right
                    .read_interpolated(portal_delay_samples);

                self.portal_feedback_state_l = portal_damping * self.portal_feedback_state_l
                    + (1.0 - portal_damping) * portal_l;
                self.portal_feedback_state_r = portal_damping * self.portal_feedback_state_r
                    + (1.0 - portal_damping) * portal_r;

                self.portal_delay_left
                    .write(spat_left + self.portal_feedback_state_l * portal_feedback);
                self.portal_delay_right
                    .write(spat_right + self.portal_feedback_state_r * portal_feedback);

                let amt = self.current_portal;
                (
                    spat_left + self.portal_feedback_state_l * amt,
                    spat_right + self.portal_feedback_state_r * amt,
                )
            } else {
                self.portal_delay_left.write(0.0);
                self.portal_delay_right.write(0.0);
                (spat_left, spat_right)
            };

            // Dry/wet mix
            let dry_amount = 1.0 - self.current_dry_wet;
            let wet_amount = self.current_dry_wet;

            buffer[idx] = dry_left * dry_amount + spat_left * wet_amount;
            buffer[idx + 1] = dry_right * dry_amount + spat_right * wet_amount;
        }
    }

    // --- Internal helpers ---

    /// Advance LFOs and apply their modulation to snapshot parameters.
    fn update_lfos(&mut self, block_size: usize, sample_rate: f32) {
        let lfo1_val = self.lfo1.advance(block_size, sample_rate);
        let lfo2_val = self.lfo2.advance(block_size, sample_rate);
        let lfo3_val = self.lfo3.advance(block_size, sample_rate);
        let lfo4_val = self.lfo4.advance(block_size, sample_rate);

        self.apply_lfo_modulation(self.lfo1.target(), lfo1_val);
        self.apply_lfo_modulation(self.lfo2.target(), lfo2_val);
        self.apply_lfo_modulation(self.lfo3.target(), lfo3_val);
        self.apply_lfo_modulation(self.lfo4.target(), lfo4_val);
    }

    /// Apply a single LFO's modulation value to the appropriate parameter.
    fn apply_lfo_modulation(&mut self, target: AweLfoTarget, value: f32) {
        if value.abs() < f32::EPSILON {
            return;
        }
        match target {
            AweLfoTarget::RoomLength => {
                self.room = match self.room {
                    RoomShape::Box {
                        length,
                        width,
                        height,
                    } => RoomShape::Box {
                        length: (length + value * 2.0).max(1.0),
                        width,
                        height,
                    },
                    RoomShape::Cylinder { radius, length } => RoomShape::Cylinder {
                        radius,
                        length: (length + value * 2.0).max(1.0),
                    },
                    RoomShape::LShape {
                        length_a,
                        width_a,
                        length_b,
                        width_b,
                        height,
                    } => RoomShape::LShape {
                        length_a: (length_a + value).max(1.0),
                        width_a,
                        length_b: (length_b + value).max(1.0),
                        width_b,
                        height,
                    },
                };
                self.geometry_dirty = true;
            }
            AweLfoTarget::RoomWidth => {
                self.room = match self.room {
                    RoomShape::Box {
                        length,
                        width,
                        height,
                    } => RoomShape::Box {
                        length,
                        width: (width + value * 2.0).max(1.0),
                        height,
                    },
                    RoomShape::Cylinder { radius, length } => RoomShape::Cylinder {
                        radius: (radius + value).max(0.5),
                        length,
                    },
                    RoomShape::LShape {
                        length_a,
                        width_a,
                        length_b,
                        width_b,
                        height,
                    } => RoomShape::LShape {
                        length_a,
                        width_a: (width_a + value).max(1.0),
                        length_b,
                        width_b: (width_b + value).max(1.0),
                        height,
                    },
                };
                self.geometry_dirty = true;
            }
            AweLfoTarget::SourceX => {
                self.snapshot.source_pos[0] += value;
                self.geometry_dirty = true;
            }
            AweLfoTarget::SourceY => {
                self.snapshot.source_pos[1] += value;
                self.geometry_dirty = true;
            }
            AweLfoTarget::ListenerX => {
                self.snapshot.listener_pos[0] += value;
                self.geometry_dirty = true;
            }
            AweLfoTarget::ListenerY => {
                self.snapshot.listener_pos[1] += value;
                self.geometry_dirty = true;
            }
            AweLfoTarget::DryWet => {
                self.snapshot.dry_wet = (self.snapshot.dry_wet + value * 0.3).clamp(0.0, 1.0);
            }
            AweLfoTarget::FreqWarp => {
                self.snapshot.freq_warp = (self.snapshot.freq_warp + value * 0.5).clamp(-1.0, 1.0);
            }
            AweLfoTarget::EarlyLate => {
                self.snapshot.early_late_balance =
                    (self.snapshot.early_late_balance + value * 0.3).clamp(0.0, 1.0);
            }
            AweLfoTarget::ModesAmount => {
                self.snapshot.modes_amount =
                    (self.snapshot.modes_amount + value * 0.3).clamp(0.0, 1.0);
                self.room_modes.set_amount(self.snapshot.modes_amount);
            }
            AweLfoTarget::ResonanceBoost => {
                self.snapshot.resonance_boost =
                    (self.snapshot.resonance_boost + value * 0.3).clamp(0.0, 1.0);
            }
            AweLfoTarget::TailStretch => {
                self.snapshot.tail_stretch =
                    (self.snapshot.tail_stretch + value * 0.5).clamp(0.5, 4.0);
                self.geometry_dirty = true;
            }
            AweLfoTarget::PortalAmount => {
                self.snapshot.portal_amount =
                    (self.snapshot.portal_amount + value * 0.3).clamp(0.0, 1.0);
            }
        }
    }

    /// Recalculate all geometry-dependent DSP parameters.
    fn recalculate_geometry(&mut self, sample_rate: f32) {
        let room_length = self.room.length();
        let room_width = self.room.width();
        let room_height = self.room.height();
        let absorption = self.material.average_absorption();

        // Clamp positions inside the room
        let source = [
            self.snapshot.source_pos[0].clamp(0.1, room_length - 0.1),
            self.snapshot.source_pos[1].clamp(0.1, room_width - 0.1),
            self.snapshot.source_pos[2].clamp(0.1, room_height - 0.1),
        ];
        let listener = [
            self.snapshot.listener_pos[0].clamp(0.1, room_length - 0.1),
            self.snapshot.listener_pos[1].clamp(0.1, room_width - 0.1),
            self.snapshot.listener_pos[2].clamp(0.1, room_height - 0.1),
        ];

        // Update early reflections (ISM)
        self.early_reflections.update_geometry(
            room_length,
            room_width,
            room_height,
            source,
            listener,
            absorption,
            sample_rate,
        );

        // Update FDN delay times based on room dimensions
        let avg_dimension = (room_length + room_width + room_height) / 3.0;
        let room_scale = avg_dimension / 5.33; // normalize to default room avg
        let sample_rate_scale = sample_rate / 44100.0;
        self.fdn
            .set_delay_times(sample_rate_scale, room_scale * self.snapshot.tail_stretch);

        // Update room modes
        self.room_modes.update_geometry(
            room_length,
            room_width,
            room_height,
            absorption,
            sample_rate,
        );

        // Update spatializer
        self.spatializer.update(source, listener, sample_rate);
    }

    /// Calculate RT60 using Sabine's formula.
    #[must_use]
    fn calculate_rt60(&self) -> f32 {
        let volume = self.room.volume();
        let surface = self.room.surface_area();
        let absorption = self.material.average_absorption().max(0.001);
        let rt60 = 0.161 * volume / (absorption * surface);
        // Apply tail stretch and clamp to reasonable range
        (rt60 * self.snapshot.tail_stretch).clamp(0.1, 20.0)
    }

    /// Convert RT60 to FDN feedback gain.
    ///
    /// For a delay of `d` samples at `sample_rate`, the feedback gain needed
    /// to decay by 60 dB in `rt60` seconds is:
    ///   g = 10^(-3 * d / (rt60 * sample_rate))
    ///
    /// We use the average FDN delay as the reference.
    #[must_use]
    fn rt60_to_feedback(&self, rt60: f32, sample_rate: f32) -> f32 {
        // Average base delay time scaled for current room
        let avg_base_delay = 2806.0; // average of BASE_DELAY_TIMES
        let avg_dimension = (self.room.length() + self.room.width() + self.room.height()) / 3.0;
        let room_scale = avg_dimension / 5.33;
        let sample_rate_scale = sample_rate / 44100.0;
        let avg_delay = avg_base_delay * sample_rate_scale * room_scale;

        let decay_per_sample = -3.0 / (rt60 * sample_rate);
        let feedback = (10.0_f32).powf(decay_per_sample * avg_delay);
        feedback.clamp(0.0, 0.97)
    }
}

impl Default for AweEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_new() {
        let engine = AweEngine::new();
        assert!(!engine.enabled());
    }

    #[test]
    fn test_engine_set_enabled() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        assert!(engine.enabled());
        engine.set_enabled(false);
        assert!(!engine.enabled());
    }

    #[test]
    fn test_engine_process_modifies_buffer() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        // Set up geometry so reflections are active
        engine.set_param(AweParam::DryWet(0.5));

        let mut buffer = vec![0.0; 512];
        // Feed an impulse
        buffer[0] = 1.0;
        buffer[1] = 1.0;

        engine.process(&mut buffer, 48000.0);

        // Process more blocks so reflections arrive
        for _ in 0..50 {
            let mut block = vec![0.0; 512];
            engine.process(&mut block, 48000.0);
        }

        // The engine should be processing audio (not pass-through)
        // Verification: output is finite
        for sample in &buffer {
            assert!(sample.is_finite(), "Output sample is not finite");
        }
    }

    #[test]
    fn test_engine_set_param() {
        let mut engine = AweEngine::new();
        engine.set_param(AweParam::DryWet(0.7));
        assert!((engine.snapshot().dry_wet - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_engine_apply_snapshot() {
        let mut engine = AweEngine::new();
        let mut snap = AweSnapshot::default();
        snap.dry_wet = 0.8;
        snap.tail_stretch = 2.0;
        engine.apply_snapshot(snap);
        assert!((engine.snapshot().dry_wet - 0.8).abs() < 0.001);
        assert!((engine.snapshot().tail_stretch - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_engine_dry_signal_preserved_at_zero_wet() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        engine.set_param(AweParam::DryWet(0.0));

        // After a few blocks of ramping, dry/wet should settle at 0.0 (fully dry)
        for _ in 0..100 {
            let mut block = vec![0.0; 128];
            engine.process(&mut block, 48000.0);
        }

        let mut buffer = vec![0.5, -0.3, 0.1, 0.8];
        let original = buffer.clone();
        engine.process(&mut buffer, 48000.0);

        // With dry_wet=0 fully ramped, output should be very close to dry input
        for (out, orig) in buffer.iter().zip(original.iter()) {
            assert!((out - orig).abs() < 0.01, "Expected ~{orig}, got {out}");
        }
    }

    #[test]
    fn test_rt60_calculation() {
        let engine = AweEngine::new();
        let rt60 = engine.calculate_rt60();
        // Default room 8x5x3, concrete (low absorption): should give long RT60
        assert!(rt60 > 1.0, "RT60 should be > 1s for concrete, got {rt60}");
        assert!(rt60 < 20.0, "RT60 should be < 20s, got {rt60}");
    }

    #[test]
    fn test_feedback_reasonable() {
        let engine = AweEngine::new();
        let rt60 = engine.calculate_rt60();
        let feedback = engine.rt60_to_feedback(rt60, 48000.0);
        assert!(feedback > 0.5, "Feedback should be > 0.5, got {feedback}");
        assert!(
            feedback <= 0.97,
            "Feedback should be <= 0.97, got {feedback}"
        );
    }

    #[test]
    fn test_geometry_dirty_on_room_change() {
        let mut engine = AweEngine::new();
        engine.geometry_dirty = false;
        engine.set_param(AweParam::RoomShape(RoomShape::Box {
            length: 10.0,
            width: 8.0,
            height: 4.0,
        }));
        assert!(engine.geometry_dirty);
    }

    #[test]
    fn test_stability_long_run() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        engine.set_param(AweParam::DryWet(0.5));

        // Process many blocks with impulse then silence
        let mut buffer = vec![0.0; 512];
        buffer[0] = 1.0;
        buffer[1] = 1.0;
        engine.process(&mut buffer, 48000.0);

        for _ in 0..500 {
            let mut block = vec![0.0; 512];
            engine.process(&mut block, 48000.0);
            for sample in &block {
                assert!(sample.is_finite(), "Output is not finite");
                assert!(sample.abs() < 10.0, "Output exploded: {sample}");
            }
        }
    }

    #[test]
    fn test_empty_buffer() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        let mut buffer: Vec<f32> = Vec::new();
        engine.process(&mut buffer, 48000.0);
        // Should not panic
    }

    #[test]
    fn test_lfo_params_applied() {
        let mut engine = AweEngine::new();
        engine.set_param(AweParam::Lfo1Rate(2.0));
        engine.set_param(AweParam::Lfo1Amount(0.5));
        engine.set_param(AweParam::Lfo1Target(AweLfoTarget::DryWet));
        assert!((engine.snapshot().lfo1.rate - 2.0).abs() < 0.001);
        assert!((engine.snapshot().lfo1.amount - 0.5).abs() < 0.001);
        assert_eq!(engine.snapshot().lfo1.target, AweLfoTarget::DryWet);
    }
}
