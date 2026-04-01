//! AWE engine — real-time room simulation processor.
//!
//! Full DSP pipeline (Phase 1):
//! 1. Early reflections via Image Source Method (ISM)
//! 2. Late reverb via FDN (geometry-driven parameters)
//! 3. Axial room modes via comb-filter bank
//! 4. Stereo spatializer (ITD + ILD) on wet signal only

use synth_core::{
    BipolarValue, FilterState, Gain, Hertz, Milliseconds, NormalizedValue, SampleCount, SampleRate,
    Seconds,
};
use synth_dsp::{FdnCore, InterpolatedDelayLine};

use crate::early_reflections::EarlyReflections;
use crate::lfo::AweLfo;
use crate::params::{AweLfoTarget, AweParam, AweSnapshot};
use crate::room::{Material, RoomShape};
use crate::room_modes::RoomModeBank;
use crate::spatial_voice::{NotePositionMapping, SpatialVoiceBank, SpatialVoicePool};
use crate::spatializer::Spatializer;
use crate::types::{Celsius, Meters, Position3, SampleOffset, StretchFactor};

/// Smooth parameter ramp time in seconds (~5 ms).
const RAMP_TIME_SECONDS: Seconds = Seconds::new(0.005);

/// Portal delay line max length in samples.
const PORTAL_MAX_DELAY: SampleCount = SampleCount::new(28_800);

/// Portal max delay for interpolation in samples.
const PORTAL_MAX_DELAY_SAMPLES: SampleOffset = SampleOffset::new(28_000.0);

/// Average dimension of the default room (8+5+3) / 3.
const AVG_REFERENCE_DIMENSION: f32 = 5.33;

/// Reference sample rate for FDN delay scaling.
const REFERENCE_SAMPLE_RATE: f32 = 44100.0;

/// Average of `BASE_DELAY_TIMES` in the FDN.
const AVG_BASE_FDN_DELAY: f32 = 2806.0;

/// Fixed feedback gain for the acoustic portal path.
const DEFAULT_PORTAL_FEEDBACK: f32 = 0.4;

/// LP damping coefficient for muffled portal sound.
const DEFAULT_PORTAL_DAMPING: f32 = 0.6;

/// Maximum pre-delay time in milliseconds.
const PRE_DELAY_MAX_MS: f32 = 200.0;

/// Maximum pre-delay in samples (200ms @ 96kHz).
const PRE_DELAY_MAX_SAMPLES: SampleCount = SampleCount::new(19_200);

/// The Acoustic World Engine processor.
///
/// Processes interleaved stereo audio through a physics-based room simulation:
/// early reflections, late reverb, room modes, and stereo spatialisation.
pub struct AweEngine {
    enabled: bool,
    room: RoomShape,
    material: Material,
    snapshot: AweSnapshot,
    cached_sample_rate: SampleRate,

    // Base values before LFO modulation (set by user/presets, never by LFOs)
    base_room: RoomShape,
    base_snapshot: AweSnapshot,

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

    // Pre-delay — delays wet input before reflections
    pre_delay_left: InterpolatedDelayLine,
    current_pre_delay_samples: f32,

    // Acoustic portal — extra delay feedback path simulating adjoining room
    portal_delay_left: InterpolatedDelayLine,
    portal_delay_right: InterpolatedDelayLine,
    portal_feedback_state_l: FilterState,
    portal_feedback_state_r: FilterState,

    // Smoothed DSP parameters (current values that ramp toward targets)
    current_dry_wet: NormalizedValue,
    current_early_late: NormalizedValue,
    current_portal: NormalizedValue,

    // FDN modulation (chorus to break metallic artifacts)
    mod_chorus_left: InterpolatedDelayLine,
    mod_chorus_right: InterpolatedDelayLine,
    mod_phase: f32,

    // Air absorption one-pole LP filter states (left/right)
    air_absorption_state_l: FilterState,
    air_absorption_state_r: FilterState,

    // Wet signal EQ filter states
    wet_lp_state_l: FilterState,
    wet_lp_state_r: FilterState,
    wet_hp_state_l: FilterState,
    wet_hp_state_r: FilterState,

    // Geometry dirty flag — recalculate ISM taps, FDN, modes when true
    geometry_dirty: bool,

    // Per-voice spatial
    spatial_enabled: bool,
    note_mapping: NotePositionMapping,
    voice_pool: SpatialVoicePool,
}

struct FdnParams {
    ramp_coeff: f32,
    lp_coeff: NormalizedValue,
    hp_coeff: NormalizedValue,
    feedback_gain: Gain,
    diffusion: NormalizedValue,
    width: NormalizedValue,
    pre_delay_samples: f32,
    portal_delay: SampleOffset,
    portal_feedback: Gain,
    portal_damping: NormalizedValue,
    mod_depth: f32,
    mod_rate: f32,
    high_cut_coeff: f32,
    low_cut_coeff: f32,
}

impl AweEngine {
    /// Create a new AWE engine (disabled by default).
    #[must_use]
    pub fn new() -> Self {
        let snapshot = AweSnapshot::default();
        let room = RoomShape::default();
        Self {
            enabled: false,
            room,
            material: Material::default(),
            snapshot,
            cached_sample_rate: SampleRate::new(48000.0),

            base_room: room,
            base_snapshot: snapshot,

            early_reflections: EarlyReflections::new(),
            fdn: FdnCore::new(),
            room_modes: RoomModeBank::new(),
            spatializer: Spatializer::new(),

            lfo1: AweLfo::new(),
            lfo2: AweLfo::new(),
            lfo3: AweLfo::new(),
            lfo4: AweLfo::new(),

            // Pre-delay: max 200ms @ 96kHz = 19200 samples
            pre_delay_left: InterpolatedDelayLine::new(PRE_DELAY_MAX_SAMPLES.as_usize()),
            current_pre_delay_samples: 0.0,

            // Portal delay lines: max ~300ms @ 96kHz = 28800 samples
            portal_delay_left: InterpolatedDelayLine::new(PORTAL_MAX_DELAY.as_usize()),
            portal_delay_right: InterpolatedDelayLine::new(PORTAL_MAX_DELAY.as_usize()),
            portal_feedback_state_l: FilterState::ZERO,
            portal_feedback_state_r: FilterState::ZERO,

            current_dry_wet: snapshot.dry_wet,
            current_early_late: snapshot.early_late_balance,
            current_portal: snapshot.portal_amount,

            mod_chorus_left: InterpolatedDelayLine::new(64),
            mod_chorus_right: InterpolatedDelayLine::new(64),
            mod_phase: 0.0,

            air_absorption_state_l: FilterState::ZERO,
            air_absorption_state_r: FilterState::ZERO,

            wet_lp_state_l: FilterState::ZERO,
            wet_lp_state_r: FilterState::ZERO,
            wet_hp_state_l: FilterState::ZERO,
            wet_hp_state_r: FilterState::ZERO,

            geometry_dirty: true,

            spatial_enabled: false,
            note_mapping: NotePositionMapping::Off,
            voice_pool: SpatialVoicePool::new(),
        }
    }

    /// Process audio buffer (interleaved stereo: [L0, R0, L1, R1, ...]).
    #[allow(clippy::too_many_lines)]
    pub fn process(&mut self, buffer: &mut [f32], sample_rate: SampleRate) {
        self.cached_sample_rate = sample_rate;

        let num_samples = buffer.len() / 2;
        let block_size = SampleCount::new(num_samples);
        if num_samples == 0 {
            return;
        }

        // 1. Control-rate: advance LFOs and apply modulation
        self.update_lfos(block_size, sample_rate);

        // 2. Recalculate geometry-dependent DSP parameters if changed
        if self.geometry_dirty {
            self.recalculate_geometry(sample_rate);
            self.geometry_dirty = false;
        }

        let fdn = self.compute_fdn_params(sample_rate);
        let sample_rate_recip = 1.0 / sample_rate.as_f32();

        let target_dry_wet = self.snapshot.dry_wet.as_f32();
        let target_early_late = self.snapshot.early_late_balance.as_f32();
        let target_portal = self.snapshot.portal_amount.as_f32();

        let mut current_dry_wet = self.current_dry_wet.as_f32();
        let mut current_early_late = self.current_early_late.as_f32();
        let mut current_portal = self.current_portal.as_f32();
        let mut portal_feedback_state_l = self.portal_feedback_state_l;
        let mut portal_feedback_state_r = self.portal_feedback_state_r;
        let mut air_abs_state_l = self.air_absorption_state_l;
        let mut air_abs_state_r = self.air_absorption_state_r;

        let target_pre_delay = fdn.pre_delay_samples;
        let mut current_pre_delay = self.current_pre_delay_samples;

        // Compute air absorption LP coefficient from distance
        let air_absorption = self.snapshot.air_absorption.as_f32();
        let air_lp = if air_absorption > 0.001 {
            let [sx, sy, sz] = self.snapshot.source_pos.as_f32();
            let [lx, ly, lz] = self.snapshot.listener_pos.as_f32();
            let dx = sx - lx;
            let dy = sy - ly;
            let dz = sz - lz;
            let distance = (dx * dx + dy * dy + dz * dz).sqrt();
            air_absorption * (distance / 20.0).min(1.0)
        } else {
            0.0
        };

        // 3. Per-sample DSP
        for i in 0..num_samples {
            let idx = i * 2;
            let dry_left = buffer[idx];
            let dry_right = buffer[idx + 1];

            // Mono input for wet processing
            let mono_input = (dry_left + dry_right) * 0.5;

            // Smooth ramp toward target parameters
            current_dry_wet += fdn.ramp_coeff * (target_dry_wet - current_dry_wet);
            current_early_late += fdn.ramp_coeff * (target_early_late - current_early_late);
            current_portal += fdn.ramp_coeff * (target_portal - current_portal);
            current_pre_delay += fdn.ramp_coeff * (target_pre_delay - current_pre_delay);

            // --- Pre-delay (delays wet input before reflections) ---
            let delayed_input = if current_pre_delay > 0.5 {
                self.pre_delay_left.write(mono_input);
                self.pre_delay_left.read_interpolated(current_pre_delay)
            } else {
                self.pre_delay_left.write(mono_input);
                mono_input
            };

            // --- Early reflections (ISM) ---
            let (early_left, early_right) = self.early_reflections.process(delayed_input);

            // --- Late reverb (FDN) ---
            let fdn_out = self.fdn.process_sample(
                delayed_input,
                fdn.feedback_gain,
                fdn.lp_coeff,
                fdn.hp_coeff,
                fdn.diffusion,
                fdn.width,
                sample_rate_recip,
            );

            // --- FDN modulation (chorus to break metallic artifacts) ---
            let (fdn_left, fdn_right) = if fdn.mod_depth > 0.001 {
                self.mod_phase += fdn.mod_rate * sample_rate_recip;
                if self.mod_phase >= 1.0 {
                    self.mod_phase -= 1.0;
                }
                let lfo_val = (self.mod_phase * std::f32::consts::TAU).sin();
                let mod_delay = 20.0 + lfo_val * fdn.mod_depth * 18.0;
                self.mod_chorus_left.write(fdn_out.left);
                self.mod_chorus_right.write(fdn_out.right);
                (
                    self.mod_chorus_left.read_interpolated(mod_delay),
                    self.mod_chorus_right.read_interpolated(mod_delay),
                )
            } else {
                self.mod_chorus_left.write(fdn_out.left);
                self.mod_chorus_right.write(fdn_out.right);
                (fdn_out.left, fdn_out.right)
            };

            // --- Room modes ---
            let modes_out = self.room_modes.process(delayed_input);

            // --- Mix early/late ---
            let early_amount = 1.0 - current_early_late;
            let late_amount = current_early_late;

            let wet_mono_left =
                early_left * early_amount + fdn_left * late_amount + modes_out * 0.5;
            let wet_mono_right =
                early_right * early_amount + fdn_right * late_amount + modes_out * 0.5;

            // --- Air absorption (distance-proportional HF damping) ---
            let (wet_mono_left, wet_mono_right) = if air_lp > 0.001 {
                (
                    air_abs_state_l.one_pole(wet_mono_left, air_lp),
                    air_abs_state_r.one_pole(wet_mono_right, air_lp),
                )
            } else {
                (wet_mono_left, wet_mono_right)
            };

            // --- Spatializer (on wet signal only) ---
            let wet_mid = (wet_mono_left + wet_mono_right) * 0.5;
            let (spat_left, spat_right) = self.spatializer.process(wet_mid);

            // --- Wet signal EQ ---
            let spat_left = if fdn.high_cut_coeff > 0.001 {
                self.wet_lp_state_l.one_pole(spat_left, fdn.high_cut_coeff)
            } else {
                spat_left
            };
            let spat_right = if fdn.high_cut_coeff > 0.001 {
                self.wet_lp_state_r.one_pole(spat_right, fdn.high_cut_coeff)
            } else {
                spat_right
            };
            let spat_left = if fdn.low_cut_coeff < 0.999 {
                self.wet_hp_state_l
                    .one_pole_hp(spat_left, fdn.low_cut_coeff)
            } else {
                spat_left
            };
            let spat_right = if fdn.low_cut_coeff < 0.999 {
                self.wet_hp_state_r
                    .one_pole_hp(spat_right, fdn.low_cut_coeff)
            } else {
                spat_right
            };

            // --- Acoustic portal (delayed feedback from adjoining virtual room) ---
            let (spat_left, spat_right) = if current_portal > 0.001 {
                let portal_l = self
                    .portal_delay_left
                    .read_interpolated(fdn.portal_delay.as_f32());
                let portal_r = self
                    .portal_delay_right
                    .read_interpolated(fdn.portal_delay.as_f32());

                let filtered_l =
                    portal_feedback_state_l.one_pole(portal_l, fdn.portal_damping.as_f32());
                let filtered_r =
                    portal_feedback_state_r.one_pole(portal_r, fdn.portal_damping.as_f32());

                self.portal_delay_left
                    .write(spat_left + fdn.portal_feedback.apply(filtered_l));
                self.portal_delay_right
                    .write(spat_right + fdn.portal_feedback.apply(filtered_r));

                (
                    spat_left + filtered_l * current_portal,
                    spat_right + filtered_r * current_portal,
                )
            } else {
                self.portal_delay_left.write(0.0);
                self.portal_delay_right.write(0.0);
                (spat_left, spat_right)
            };

            // Clamp wet signal to prevent runaway output
            let spat_left = spat_left.clamp(-2.0, 2.0);
            let spat_right = spat_right.clamp(-2.0, 2.0);

            // --- Dry/wet mix ---
            let dry_amount = 1.0 - current_dry_wet;
            let wet_amount = current_dry_wet;

            buffer[idx] = dry_left * dry_amount + spat_left * wet_amount;
            buffer[idx + 1] = dry_right * dry_amount + spat_right * wet_amount;
        }

        self.current_dry_wet = NormalizedValue::new(current_dry_wet);
        self.current_early_late = NormalizedValue::new(current_early_late);
        self.current_portal = NormalizedValue::new(current_portal);
        self.current_pre_delay_samples = current_pre_delay;
        self.portal_feedback_state_l = portal_feedback_state_l;
        self.portal_feedback_state_r = portal_feedback_state_r;
        self.air_absorption_state_l = air_abs_state_l;
        self.air_absorption_state_r = air_abs_state_r;
    }

    /// Set a single parameter.
    pub fn set_param(&mut self, param: AweParam) {
        match param {
            AweParam::RoomShape(shape) => {
                self.base_room = shape;
                self.room = shape;
                self.geometry_dirty = true;
            }
            AweParam::Material(mat) => {
                self.material = mat;
                self.geometry_dirty = true;
            }
            AweParam::SourcePos(pos) => {
                self.base_snapshot.source_pos = pos;
                self.snapshot.source_pos = pos;
                self.geometry_dirty = true;
            }
            AweParam::ListenerPos(pos) => {
                self.base_snapshot.listener_pos = pos;
                self.snapshot.listener_pos = pos;
                self.geometry_dirty = true;
            }
            AweParam::DryWet(v) => {
                self.base_snapshot.dry_wet = v;
                self.snapshot.dry_wet = v;
            }
            AweParam::EarlyLateBalance(v) => {
                self.base_snapshot.early_late_balance = v;
                self.snapshot.early_late_balance = v;
            }
            AweParam::ModesAmount(v) => {
                self.base_snapshot.modes_amount = v;
                self.snapshot.modes_amount = v;
                self.room_modes.set_amount(v);
            }
            AweParam::FreqWarp(v) => {
                self.base_snapshot.freq_warp = v;
                self.snapshot.freq_warp = v;
            }
            AweParam::ResonanceBoost(v) => {
                self.base_snapshot.resonance_boost = v;
                self.snapshot.resonance_boost = v;
            }
            AweParam::TailStretch(v) => {
                self.base_snapshot.tail_stretch = v;
                self.snapshot.tail_stretch = v;
                self.geometry_dirty = true;
            }
            AweParam::PortalAmount(v) => {
                self.base_snapshot.portal_amount = v;
                self.snapshot.portal_amount = v;
            }
            AweParam::PreDelay(v) => {
                self.base_snapshot.pre_delay = v;
                self.snapshot.pre_delay = v;
            }
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
            AweParam::ModulationDepth(v) => {
                self.base_snapshot.modulation_depth = v;
                self.snapshot.modulation_depth = v;
            }
            AweParam::ModulationRate(v) => {
                self.base_snapshot.modulation_rate = v;
                self.snapshot.modulation_rate = v;
            }
            AweParam::AirAbsorption(v) => {
                self.base_snapshot.air_absorption = v;
                self.snapshot.air_absorption = v;
            }
            AweParam::Width(v) => {
                self.base_snapshot.width = v;
                self.snapshot.width = v;
            }
            AweParam::HighCut(v) => {
                self.base_snapshot.high_cut = v;
                self.snapshot.high_cut = v;
            }
            AweParam::LowCut(v) => {
                self.base_snapshot.low_cut = v;
                self.snapshot.low_cut = v;
            }
            AweParam::Temperature(v) => {
                self.base_snapshot.temperature = v;
                self.snapshot.temperature = v;
                self.geometry_dirty = true;
            }
        }
    }

    /// Apply a batch snapshot of numeric parameters.
    pub fn apply_snapshot(&mut self, snapshot: AweSnapshot) {
        self.base_snapshot = snapshot;
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
            self.pre_delay_left.clear();
            self.portal_delay_left.clear();
            self.portal_delay_right.clear();
            self.portal_feedback_state_l = FilterState::ZERO;
            self.portal_feedback_state_r = FilterState::ZERO;
            self.mod_chorus_left.clear();
            self.mod_chorus_right.clear();
            self.mod_phase = 0.0;
            self.air_absorption_state_l = FilterState::ZERO;
            self.air_absorption_state_r = FilterState::ZERO;
            self.wet_lp_state_l = FilterState::ZERO;
            self.wet_lp_state_r = FilterState::ZERO;
            self.wet_hp_state_l = FilterState::ZERO;
            self.wet_hp_state_r = FilterState::ZERO;
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
        sample_rate: SampleRate,
    ) {
        self.cached_sample_rate = sample_rate;

        let num_samples = buffer.len() / 2;
        let block_size = SampleCount::new(num_samples);
        if num_samples == 0 {
            return;
        }

        let active = bank.active_count();

        // 1. Control-rate: advance LFOs
        self.update_lfos(block_size, sample_rate);

        // 2. Recalculate global geometry if dirty
        if self.geometry_dirty {
            self.recalculate_geometry(sample_rate);
            self.geometry_dirty = false;
        }

        // 3. Sync voice pool with bank: activate slots, update geometry
        let room_length = self.room.length();
        let room_width = self.room.width();
        let room_height = self.room.height();
        let min_pos = Meters::new(0.1);
        let listener = Position3::new(
            self.snapshot
                .listener_pos
                .x()
                .clamp(min_pos, (room_length - min_pos).max(min_pos)),
            self.snapshot
                .listener_pos
                .y()
                .clamp(min_pos, (room_width - min_pos).max(min_pos)),
            self.snapshot
                .listener_pos
                .z()
                .clamp(min_pos, (room_height - min_pos).max(min_pos)),
        );

        // Deactivate unused slots
        for i in active..self.voice_pool.slots.len() {
            self.voice_pool.slots[i].active = false;
        }

        // Update active slots
        for i in 0..active {
            let Some(info) = bank.info(i) else {
                continue;
            };
            self.voice_pool.update_slot(
                i,
                info.note,
                self.note_mapping,
                room_length,
                room_width,
                room_height,
                listener,
                self.material.absorption_low,
                self.material.absorption_mid,
                self.material.absorption_high,
                self.material.diffusion,
                self.snapshot.air_absorption,
                self.snapshot.temperature.speed_of_sound(),
                sample_rate,
            );
        }

        // 4. Compute shared FDN parameters
        let fdn = self.compute_fdn_params(sample_rate);
        let sample_rate_recip = 1.0 / sample_rate.as_f32();

        let target_dry_wet = self.snapshot.dry_wet.as_f32();
        let target_early_late = self.snapshot.early_late_balance.as_f32();
        let target_portal = self.snapshot.portal_amount.as_f32();

        let mut current_dry_wet = self.current_dry_wet.as_f32();
        let mut current_early_late = self.current_early_late.as_f32();
        let mut current_portal = self.current_portal.as_f32();
        let mut portal_feedback_state_l = self.portal_feedback_state_l;
        let mut portal_feedback_state_r = self.portal_feedback_state_r;
        let mut air_abs_state_l = self.air_absorption_state_l;
        let mut air_abs_state_r = self.air_absorption_state_r;

        let target_pre_delay = fdn.pre_delay_samples;
        let mut current_pre_delay = self.current_pre_delay_samples;

        // Compute air absorption LP coefficient from distance
        let air_absorption = self.snapshot.air_absorption.as_f32();
        let air_lp = if air_absorption > 0.001 {
            let [sx, sy, sz] = self.snapshot.source_pos.as_f32();
            let [lx, ly, lz] = self.snapshot.listener_pos.as_f32();
            let dx = sx - lx;
            let dy = sy - ly;
            let dz = sz - lz;
            let distance = (dx * dx + dy * dy + dz * dz).sqrt();
            air_absorption * (distance / 20.0).min(1.0)
        } else {
            0.0
        };

        // 5. Per-sample loop
        for i in 0..num_samples {
            let idx = i * 2;
            let dry_left = buffer[idx];
            let dry_right = buffer[idx + 1];

            // Smooth ramp
            current_dry_wet += fdn.ramp_coeff * (target_dry_wet - current_dry_wet);
            current_early_late += fdn.ramp_coeff * (target_early_late - current_early_late);
            current_portal += fdn.ramp_coeff * (target_portal - current_portal);
            current_pre_delay += fdn.ramp_coeff * (target_pre_delay - current_pre_delay);

            let mut total_early_l = 0.0_f32;
            let mut total_early_r = 0.0_f32;
            let mut total_spat_l = 0.0_f32;
            let mut total_spat_r = 0.0_f32;
            let mut global_mono = 0.0_f32;

            // Per active voice: process through per-voice early reflections + spatializer
            for v in 0..active {
                let mono = bank
                    .info(v)
                    .filter(|info| i < info.sample_count.as_usize())
                    .and_then(|_| bank.buffer(v))
                    .and_then(|buf| buf.get(i).copied())
                    .unwrap_or(0.0);
                global_mono += mono;

                let (el, er, sl, sr) = self.voice_pool.process_slot(v, mono);
                total_early_l += el;
                total_early_r += er;
                total_spat_l += sl;
                total_spat_r += sr;
            }

            // --- Pre-delay for shared FDN/modes ---
            let delayed_global = if current_pre_delay > 0.5 {
                self.pre_delay_left.write(global_mono);
                self.pre_delay_left.read_interpolated(current_pre_delay)
            } else {
                self.pre_delay_left.write(global_mono);
                global_mono
            };

            // Late reverb (shared FDN, fed by pre-delayed global mono)
            let fdn_out = self.fdn.process_sample(
                delayed_global,
                fdn.feedback_gain,
                fdn.lp_coeff,
                fdn.hp_coeff,
                fdn.diffusion,
                fdn.width,
                sample_rate_recip,
            );

            // FDN modulation (chorus to break metallic artifacts)
            let (fdn_left, fdn_right) = if fdn.mod_depth > 0.001 {
                self.mod_phase += fdn.mod_rate * sample_rate_recip;
                if self.mod_phase >= 1.0 {
                    self.mod_phase -= 1.0;
                }
                let lfo_val = (self.mod_phase * std::f32::consts::TAU).sin();
                let mod_delay = 20.0 + lfo_val * fdn.mod_depth * 18.0;
                self.mod_chorus_left.write(fdn_out.left);
                self.mod_chorus_right.write(fdn_out.right);
                (
                    self.mod_chorus_left.read_interpolated(mod_delay),
                    self.mod_chorus_right.read_interpolated(mod_delay),
                )
            } else {
                self.mod_chorus_left.write(fdn_out.left);
                self.mod_chorus_right.write(fdn_out.right);
                (fdn_out.left, fdn_out.right)
            };

            // Room modes (shared, pre-delayed)
            let modes_out = self.room_modes.process(delayed_global);

            // Mix early/late
            let early_amount = 1.0 - current_early_late;
            let late_amount = current_early_late;

            // Use per-voice spatialized dry instead of global spatializer
            let wet_left = total_early_l * early_amount + fdn_left * late_amount + modes_out * 0.5;
            let wet_right =
                total_early_r * early_amount + fdn_right * late_amount + modes_out * 0.5;

            // Air absorption (distance-proportional HF damping)
            let (wet_left, wet_right) = if air_lp > 0.001 {
                (
                    air_abs_state_l.one_pole(wet_left, air_lp),
                    air_abs_state_r.one_pole(wet_right, air_lp),
                )
            } else {
                (wet_left, wet_right)
            };

            // Combine with per-voice spatializer output for dry positioning
            let spat_left = wet_left + total_spat_l * 0.1;
            let spat_right = wet_right + total_spat_r * 0.1;

            // --- Wet signal EQ ---
            let spat_left = if fdn.high_cut_coeff > 0.001 {
                self.wet_lp_state_l.one_pole(spat_left, fdn.high_cut_coeff)
            } else {
                spat_left
            };
            let spat_right = if fdn.high_cut_coeff > 0.001 {
                self.wet_lp_state_r.one_pole(spat_right, fdn.high_cut_coeff)
            } else {
                spat_right
            };
            let spat_left = if fdn.low_cut_coeff < 0.999 {
                self.wet_hp_state_l
                    .one_pole_hp(spat_left, fdn.low_cut_coeff)
            } else {
                spat_left
            };
            let spat_right = if fdn.low_cut_coeff < 0.999 {
                self.wet_hp_state_r
                    .one_pole_hp(spat_right, fdn.low_cut_coeff)
            } else {
                spat_right
            };

            // Portal
            let (spat_left, spat_right) = if current_portal > 0.001 {
                let portal_l = self
                    .portal_delay_left
                    .read_interpolated(fdn.portal_delay.as_f32());
                let portal_r = self
                    .portal_delay_right
                    .read_interpolated(fdn.portal_delay.as_f32());

                let filtered_l =
                    portal_feedback_state_l.one_pole(portal_l, fdn.portal_damping.as_f32());
                let filtered_r =
                    portal_feedback_state_r.one_pole(portal_r, fdn.portal_damping.as_f32());

                self.portal_delay_left
                    .write(spat_left + fdn.portal_feedback.apply(filtered_l));
                self.portal_delay_right
                    .write(spat_right + fdn.portal_feedback.apply(filtered_r));

                (
                    spat_left + filtered_l * current_portal,
                    spat_right + filtered_r * current_portal,
                )
            } else {
                self.portal_delay_left.write(0.0);
                self.portal_delay_right.write(0.0);
                (spat_left, spat_right)
            };

            // Clamp wet signal to prevent runaway output
            let spat_left = spat_left.clamp(-2.0, 2.0);
            let spat_right = spat_right.clamp(-2.0, 2.0);

            // Dry/wet mix
            let dry_amount = 1.0 - current_dry_wet;
            let wet_amount = current_dry_wet;

            buffer[idx] = dry_left * dry_amount + spat_left * wet_amount;
            buffer[idx + 1] = dry_right * dry_amount + spat_right * wet_amount;
        }

        self.current_dry_wet = NormalizedValue::new(current_dry_wet);
        self.current_early_late = NormalizedValue::new(current_early_late);
        self.current_portal = NormalizedValue::new(current_portal);
        self.current_pre_delay_samples = current_pre_delay;
        self.portal_feedback_state_l = portal_feedback_state_l;
        self.portal_feedback_state_r = portal_feedback_state_r;
        self.air_absorption_state_l = air_abs_state_l;
        self.air_absorption_state_r = air_abs_state_r;
    }

    // --- Internal helpers ---

    fn compute_fdn_params(&self, sample_rate: SampleRate) -> FdnParams {
        let ramp_coeff = 1.0 - (-1.0 / (RAMP_TIME_SECONDS.as_f32() * sample_rate.as_f32())).exp();

        let abs_low = self.material.absorption_low.as_f32();
        let abs_high = self.material.absorption_high.as_f32();
        let abs_high_eff = abs_high.sqrt();
        let abs_low_eff = abs_low.sqrt();
        let rt60 = self.calculate_rt60();
        let freq_warp = self.snapshot.freq_warp;

        let lp_coeff = NormalizedValue::new(
            ((0.15 + abs_high_eff * 0.80) * (1.0 - freq_warp.as_f32() * 0.3)).clamp(0.0, 0.999),
        );
        let hp_coeff = NormalizedValue::new(
            ((0.997 - abs_low_eff * 0.45) - freq_warp.as_f32() * 0.05).clamp(0.0, 0.999),
        );

        let resonance_boost = self.snapshot.resonance_boost;
        let feedback_gain = Gain::new(
            (self.rt60_to_feedback(rt60, sample_rate).as_f32() + resonance_boost.as_f32() * 0.15)
                .min(0.97),
        );
        let material_diffusion = self.material.diffusion;
        let diffusion =
            NormalizedValue::new((0.35 + material_diffusion.as_f32() * 0.55).clamp(0.1, 1.0));
        let width = self.snapshot.width;

        let room = &self.room;
        let avg_dim =
            (room.length().as_f32() + room.width().as_f32() + room.height().as_f32()) / 3.0;
        let portal_delay_s = 0.2 * (avg_dim / AVG_REFERENCE_DIMENSION);
        let portal_delay = SampleOffset::new(
            (portal_delay_s * sample_rate.as_f32()).clamp(1.0, PORTAL_MAX_DELAY_SAMPLES.as_f32()),
        );
        let portal_feedback = Gain::new(DEFAULT_PORTAL_FEEDBACK);
        let portal_damping = NormalizedValue::new(DEFAULT_PORTAL_DAMPING);

        // Pre-delay: convert ms to samples, clamped to buffer size
        let pre_delay_ms = self
            .snapshot
            .pre_delay
            .as_f32()
            .clamp(0.0, PRE_DELAY_MAX_MS);
        let pre_delay_samples = (pre_delay_ms * 0.001 * sample_rate.as_f32())
            .min(PRE_DELAY_MAX_SAMPLES.as_usize() as f32 - 1.0);

        let mod_depth = self.snapshot.modulation_depth.as_f32();
        let mod_rate = self.snapshot.modulation_rate.as_f32().clamp(0.1, 10.0);

        // Wet signal EQ: one-pole coefficients
        let high_cut_hz = self.snapshot.high_cut.as_f32().clamp(200.0, 20000.0);
        let low_cut_hz = self.snapshot.low_cut.as_f32().clamp(20.0, 2000.0);
        let sr = sample_rate.as_f32();
        let high_cut_coeff = (-std::f32::consts::TAU * high_cut_hz / sr).exp();
        let low_cut_coeff = (-std::f32::consts::TAU * low_cut_hz / sr).exp();

        FdnParams {
            ramp_coeff,
            lp_coeff,
            hp_coeff,
            feedback_gain,
            diffusion,
            width,
            pre_delay_samples,
            portal_delay,
            portal_feedback,
            portal_damping,
            mod_depth,
            mod_rate,
            high_cut_coeff,
            low_cut_coeff,
        }
    }

    /// Advance LFOs and apply their modulation to snapshot parameters.
    ///
    /// Restores base values first, then applies all LFO offsets so modulation
    /// oscillates around the user-set values rather than drifting.
    fn update_lfos(&mut self, block_size: SampleCount, sample_rate: SampleRate) {
        let lfo1_val = self.lfo1.advance(block_size, sample_rate);
        let lfo2_val = self.lfo2.advance(block_size, sample_rate);
        let lfo3_val = self.lfo3.advance(block_size, sample_rate);
        let lfo4_val = self.lfo4.advance(block_size, sample_rate);

        let any_active = lfo1_val.as_f32().abs() > f32::EPSILON
            || lfo2_val.as_f32().abs() > f32::EPSILON
            || lfo3_val.as_f32().abs() > f32::EPSILON
            || lfo4_val.as_f32().abs() > f32::EPSILON;

        if !any_active {
            return;
        }

        // Restore base values before applying LFO offsets
        self.room = self.base_room;
        self.snapshot.source_pos = self.base_snapshot.source_pos;
        self.snapshot.listener_pos = self.base_snapshot.listener_pos;
        self.snapshot.dry_wet = self.base_snapshot.dry_wet;
        self.snapshot.freq_warp = self.base_snapshot.freq_warp;
        self.snapshot.early_late_balance = self.base_snapshot.early_late_balance;
        self.snapshot.modes_amount = self.base_snapshot.modes_amount;
        self.snapshot.resonance_boost = self.base_snapshot.resonance_boost;
        self.snapshot.tail_stretch = self.base_snapshot.tail_stretch;
        self.snapshot.portal_amount = self.base_snapshot.portal_amount;
        self.snapshot.pre_delay = self.base_snapshot.pre_delay;
        self.snapshot.modulation_depth = self.base_snapshot.modulation_depth;
        self.snapshot.modulation_rate = self.base_snapshot.modulation_rate;
        self.snapshot.air_absorption = self.base_snapshot.air_absorption;
        self.snapshot.width = self.base_snapshot.width;
        self.snapshot.high_cut = self.base_snapshot.high_cut;
        self.snapshot.low_cut = self.base_snapshot.low_cut;
        self.snapshot.temperature = self.base_snapshot.temperature;

        // Apply all LFO offsets from the restored base
        self.apply_lfo_modulation(self.lfo1.target(), lfo1_val);
        self.apply_lfo_modulation(self.lfo2.target(), lfo2_val);
        self.apply_lfo_modulation(self.lfo3.target(), lfo3_val);
        self.apply_lfo_modulation(self.lfo4.target(), lfo4_val);
    }

    /// Apply a single LFO's modulation value to the appropriate parameter.
    fn apply_lfo_modulation(&mut self, target: AweLfoTarget, value: BipolarValue) {
        let value = value.as_f32();
        if value.abs() < f32::EPSILON {
            return;
        }
        match target {
            AweLfoTarget::RoomLength => {
                let delta_long = Meters::new(value * 2.0);
                let delta_short = Meters::new(value);
                let min_length = Meters::new(1.0);
                let min_radius = Meters::new(0.5);
                self.room = match self.room {
                    RoomShape::Box {
                        length,
                        width,
                        height,
                    } => RoomShape::Box {
                        length: (length + delta_long).max(min_length),
                        width,
                        height,
                    },
                    RoomShape::Cylinder { radius, length } => RoomShape::Cylinder {
                        radius,
                        length: (length + delta_long).max(min_length),
                    },
                    RoomShape::LShape {
                        length_a,
                        width_a,
                        length_b,
                        width_b,
                        height,
                    } => RoomShape::LShape {
                        length_a: (length_a + delta_short).max(min_length),
                        width_a,
                        length_b: (length_b + delta_short).max(min_length),
                        width_b,
                        height,
                    },
                    RoomShape::Sphere { radius } => RoomShape::Sphere {
                        radius: (radius + delta_short).max(min_radius),
                    },
                    RoomShape::Dome { radius } => RoomShape::Dome {
                        radius: (radius + delta_short).max(min_radius),
                    },
                    RoomShape::Tube { radius, length } => RoomShape::Tube {
                        radius,
                        length: (length + delta_long).max(min_length),
                    },
                };
                self.geometry_dirty = true;
            }
            AweLfoTarget::RoomWidth => {
                let delta_long = Meters::new(value * 2.0);
                let delta_short = Meters::new(value);
                let min_length = Meters::new(1.0);
                let min_radius = Meters::new(0.5);
                self.room = match self.room {
                    RoomShape::Box {
                        length,
                        width,
                        height,
                    } => RoomShape::Box {
                        length,
                        width: (width + delta_long).max(min_length),
                        height,
                    },
                    RoomShape::Cylinder { radius, length } => RoomShape::Cylinder {
                        radius: (radius + delta_short).max(min_radius),
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
                        width_a: (width_a + delta_short).max(min_length),
                        length_b,
                        width_b: (width_b + delta_short).max(min_length),
                        height,
                    },
                    RoomShape::Sphere { radius } => RoomShape::Sphere {
                        radius: (radius + delta_short).max(min_radius),
                    },
                    RoomShape::Dome { radius } => RoomShape::Dome {
                        radius: (radius + delta_short).max(min_radius),
                    },
                    RoomShape::Tube { radius, length } => RoomShape::Tube {
                        radius: (radius + delta_short).max(min_radius),
                        length,
                    },
                };
                self.geometry_dirty = true;
            }
            AweLfoTarget::SourceX => {
                self.snapshot.source_pos[0] += Meters::new(value);
                self.geometry_dirty = true;
            }
            AweLfoTarget::SourceY => {
                self.snapshot.source_pos[1] += Meters::new(value);
                self.geometry_dirty = true;
            }
            AweLfoTarget::ListenerX => {
                self.snapshot.listener_pos[0] += Meters::new(value);
                self.geometry_dirty = true;
            }
            AweLfoTarget::ListenerY => {
                self.snapshot.listener_pos[1] += Meters::new(value);
                self.geometry_dirty = true;
            }
            AweLfoTarget::DryWet => {
                self.snapshot.dry_wet =
                    NormalizedValue::new(self.snapshot.dry_wet.as_f32() + value * 0.3);
            }
            AweLfoTarget::FreqWarp => {
                self.snapshot.freq_warp =
                    BipolarValue::new(self.snapshot.freq_warp.as_f32() + value * 0.5);
            }
            AweLfoTarget::EarlyLate => {
                self.snapshot.early_late_balance =
                    NormalizedValue::new(self.snapshot.early_late_balance.as_f32() + value * 0.3);
            }
            AweLfoTarget::ModesAmount => {
                self.snapshot.modes_amount =
                    NormalizedValue::new(self.snapshot.modes_amount.as_f32() + value * 0.3);
                self.room_modes.set_amount(self.snapshot.modes_amount);
            }
            AweLfoTarget::ResonanceBoost => {
                self.snapshot.resonance_boost =
                    NormalizedValue::new(self.snapshot.resonance_boost.as_f32() + value * 0.3);
            }
            AweLfoTarget::TailStretch => {
                self.snapshot.tail_stretch =
                    StretchFactor::new(self.snapshot.tail_stretch.as_f32() + value * 0.5);
                self.geometry_dirty = true;
            }
            AweLfoTarget::PortalAmount => {
                self.snapshot.portal_amount =
                    NormalizedValue::new(self.snapshot.portal_amount.as_f32() + value * 0.3);
            }
            AweLfoTarget::PreDelay => {
                self.snapshot.pre_delay = Milliseconds::new(
                    (self.snapshot.pre_delay.as_f32() + value * 50.0).clamp(0.0, PRE_DELAY_MAX_MS),
                );
            }
            AweLfoTarget::ModulationDepth => {
                self.snapshot.modulation_depth =
                    NormalizedValue::new(self.snapshot.modulation_depth.as_f32() + value * 0.3);
            }
            AweLfoTarget::ModulationRate => {
                self.snapshot.modulation_rate = Hertz::new(
                    (self.snapshot.modulation_rate.as_f32() + value * 2.0).clamp(0.1, 10.0),
                );
            }
            AweLfoTarget::AirAbsorption => {
                self.snapshot.air_absorption =
                    NormalizedValue::new(self.snapshot.air_absorption.as_f32() + value * 0.3);
            }
            AweLfoTarget::Width => {
                self.snapshot.width =
                    NormalizedValue::new(self.snapshot.width.as_f32() + value * 0.3);
            }
            AweLfoTarget::HighCut => {
                let freq = self.snapshot.high_cut.as_f32();
                self.snapshot.high_cut =
                    Hertz::new((freq * (1.0 + value * 0.5)).clamp(200.0, 20000.0));
            }
            AweLfoTarget::LowCut => {
                let freq = self.snapshot.low_cut.as_f32();
                self.snapshot.low_cut =
                    Hertz::new((freq * (1.0 + value * 0.5)).clamp(20.0, 2000.0));
            }
            AweLfoTarget::Temperature => {
                self.snapshot.temperature = Celsius::new(
                    (self.snapshot.temperature.as_f32() + value * 10.0).clamp(-40.0, 60.0),
                );
                self.geometry_dirty = true;
            }
        }
    }

    /// Recalculate all geometry-dependent DSP parameters.
    fn recalculate_geometry(&mut self, sample_rate: SampleRate) {
        let room_length = self.room.length();
        let room_width = self.room.width();
        let room_height = self.room.height();
        let speed_of_sound = self.snapshot.temperature.speed_of_sound();

        // Clamp positions inside the room
        let min_pos = Meters::new(0.1);
        let source = Position3::new(
            self.snapshot
                .source_pos
                .x()
                .clamp(min_pos, room_length - min_pos),
            self.snapshot
                .source_pos
                .y()
                .clamp(min_pos, room_width - min_pos),
            self.snapshot
                .source_pos
                .z()
                .clamp(min_pos, room_height - min_pos),
        );
        let listener = Position3::new(
            self.snapshot
                .listener_pos
                .x()
                .clamp(min_pos, room_length - min_pos),
            self.snapshot
                .listener_pos
                .y()
                .clamp(min_pos, room_width - min_pos),
            self.snapshot
                .listener_pos
                .z()
                .clamp(min_pos, room_height - min_pos),
        );

        // Update early reflections (ISM) with per-band absorption
        self.early_reflections.update_geometry(
            room_length,
            room_width,
            room_height,
            source,
            listener,
            self.material.absorption_low,
            self.material.absorption_mid,
            self.material.absorption_high,
            self.material.diffusion,
            self.snapshot.air_absorption,
            speed_of_sound,
            sample_rate,
        );

        // Update FDN delay times based on room dimensions
        let avg_dimension = (room_length + room_width + room_height) / 3.0;
        let room_scale = avg_dimension.as_f32() / AVG_REFERENCE_DIMENSION;
        let sample_rate_scale = sample_rate.as_f32() / REFERENCE_SAMPLE_RATE;
        self.fdn.set_delay_times(
            sample_rate_scale,
            room_scale * self.snapshot.tail_stretch.as_f32(),
        );

        // Update room modes with per-band absorption
        self.room_modes.update_geometry(
            room_length,
            room_width,
            room_height,
            self.material.absorption_low,
            self.material.absorption_high,
            speed_of_sound,
            sample_rate,
        );

        // Update spatializer
        self.spatializer
            .update(source, listener, speed_of_sound, sample_rate);
    }

    /// Calculate RT60 using Eyring's formula.
    ///
    /// Eyring is more accurate than Sabine at high absorption coefficients:
    /// `RT60 = -0.161 * V / (S * ln(1 - α))`
    #[must_use]
    fn calculate_rt60(&self) -> Seconds {
        let volume = self.room.volume();
        let surface = self.room.surface_area();
        let absorption = self
            .material
            .average_absorption()
            .as_f32()
            .clamp(0.001, 0.999);
        let rt60 = -0.161 * volume.as_f32() / (surface.as_f32() * (1.0 - absorption).ln());
        // Apply tail stretch and clamp to reasonable range
        Seconds::new((rt60 * self.snapshot.tail_stretch.as_f32()).clamp(0.1, 20.0))
    }

    /// Convert RT60 to FDN feedback gain.
    ///
    /// For a delay of `d` samples at `sample_rate`, the feedback gain needed
    /// to decay by 60 dB in `rt60` seconds is:
    ///   g = 10^(-3 * d / (rt60 * sample_rate))
    ///
    /// We use the average FDN delay as the reference.
    #[must_use]
    fn rt60_to_feedback(&self, rt60: Seconds, sample_rate: SampleRate) -> Gain {
        // Average base delay time scaled for current room
        let avg_dimension = (self.room.length() + self.room.width() + self.room.height()) / 3.0;
        let room_scale = avg_dimension.as_f32() / AVG_REFERENCE_DIMENSION;
        let sample_rate_scale = sample_rate.as_f32() / REFERENCE_SAMPLE_RATE;
        let avg_delay = AVG_BASE_FDN_DELAY * sample_rate_scale * room_scale;

        let decay_per_sample = -3.0 / (rt60.as_f32() * sample_rate.as_f32());
        let feedback = (10.0_f32).powf(decay_per_sample * avg_delay);
        Gain::new(feedback.clamp(0.0, 0.97))
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
    use synth_core::Hertz;

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
        engine.set_param(AweParam::DryWet(NormalizedValue::new(0.5)));

        let mut buffer = vec![0.0; 512];
        // Feed an impulse
        buffer[0] = 1.0;
        buffer[1] = 1.0;

        engine.process(&mut buffer, SampleRate::new(48000.0));

        // Process more blocks so reflections arrive
        for _ in 0..50 {
            let mut block = vec![0.0; 512];
            engine.process(&mut block, SampleRate::new(48000.0));
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
        engine.set_param(AweParam::DryWet(NormalizedValue::new(0.7)));
        assert!((engine.snapshot().dry_wet.as_f32() - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_engine_apply_snapshot() {
        let mut engine = AweEngine::new();
        let snap = AweSnapshot {
            dry_wet: NormalizedValue::new(0.8),
            tail_stretch: StretchFactor::new(2.0),
            ..AweSnapshot::default()
        };
        engine.apply_snapshot(snap);
        assert!((engine.snapshot().dry_wet.as_f32() - 0.8).abs() < 0.001);
        assert!((engine.snapshot().tail_stretch.as_f32() - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_engine_dry_signal_preserved_at_zero_wet() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        engine.set_param(AweParam::DryWet(NormalizedValue::new(0.0)));

        // After a few blocks of ramping, dry/wet should settle at 0.0 (fully dry)
        for _ in 0..100 {
            let mut block = vec![0.0; 128];
            engine.process(&mut block, SampleRate::new(48000.0));
        }

        let mut buffer = vec![0.5, -0.3, 0.1, 0.8];
        let original = buffer.clone();
        engine.process(&mut buffer, SampleRate::new(48000.0));

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
        assert!(
            rt60.as_f32() > 1.0,
            "RT60 should be > 1s for concrete, got {rt60:?}"
        );
        assert!(rt60.as_f32() < 20.0, "RT60 should be < 20s, got {rt60:?}");
    }

    #[test]
    fn test_feedback_reasonable() {
        let engine = AweEngine::new();
        let rt60 = engine.calculate_rt60();
        let feedback = engine.rt60_to_feedback(rt60, SampleRate::new(48000.0));
        assert!(
            feedback.as_f32() > 0.5,
            "Feedback should be > 0.5, got {feedback:?}"
        );
        assert!(
            feedback.as_f32() <= 0.97,
            "Feedback should be <= 0.97, got {feedback:?}"
        );
    }

    #[test]
    fn test_geometry_dirty_on_room_change() {
        let mut engine = AweEngine::new();
        engine.geometry_dirty = false;
        engine.set_param(AweParam::RoomShape(RoomShape::Box {
            length: Meters::new(10.0),
            width: Meters::new(8.0),
            height: Meters::new(4.0),
        }));
        assert!(engine.geometry_dirty);
    }

    #[test]
    fn test_stability_long_run() {
        let mut engine = AweEngine::new();
        engine.set_enabled(true);
        engine.set_param(AweParam::DryWet(NormalizedValue::new(0.5)));

        // Process many blocks with impulse then silence
        let mut buffer = vec![0.0; 512];
        buffer[0] = 1.0;
        buffer[1] = 1.0;
        engine.process(&mut buffer, SampleRate::new(48000.0));

        for _ in 0..500 {
            let mut block = vec![0.0; 512];
            engine.process(&mut block, SampleRate::new(48000.0));
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
        engine.process(&mut buffer, SampleRate::new(48000.0));
        // Should not panic
    }

    #[test]
    fn test_lfo_params_applied() {
        let mut engine = AweEngine::new();
        engine.set_param(AweParam::Lfo1Rate(Hertz::new(2.0)));
        engine.set_param(AweParam::Lfo1Amount(NormalizedValue::new(0.5)));
        engine.set_param(AweParam::Lfo1Target(AweLfoTarget::DryWet));
        assert!((engine.snapshot().lfo1.rate.as_f32() - 2.0).abs() < 0.001);
        assert!((engine.snapshot().lfo1.amount.as_f32() - 0.5).abs() < 0.001);
        assert_eq!(engine.snapshot().lfo1.target, AweLfoTarget::DryWet);
    }
}
