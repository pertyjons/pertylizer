# Synth Modules Catalog

This document is a catalog of all 71 synth modules and effects in Pertylizer, grouped by their category (`ModuleCategory`). It serves as a central reference mapping each module's Rust struct to its `ModuleType` identifier and implementation file.

---

## 1. Oscillators & Sound Generators (`ModuleCategory::Oscillator`)

Sound generators and oscillators that serve as primary audio sources.

| Module | Type | File |
| :--- | :--- | :--- |
| [Oscillator](file:///home/per/github/pertylizer/crates/synth_modules/src/oscillator.rs) | `Oscillator` | [oscillator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/oscillator.rs) |
| [MathOscillator](file:///home/per/github/pertylizer/crates/synth_modules/src/math_oscillator.rs) | `MathOscillator` | [math_oscillator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/math_oscillator.rs) |
| [SubOscillator](file:///home/per/github/pertylizer/crates/synth_modules/src/sub_osc.rs) | `SubOscillator` | [sub_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/sub_osc.rs) |
| [NoiseGenerator](file:///home/per/github/pertylizer/crates/synth_modules/src/noise.rs) | `Noise` | [noise.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/noise.rs) |
| [WavetableOsc](file:///home/per/github/pertylizer/crates/synth_modules/src/wavetable_osc.rs) | `WavetableOsc` | [wavetable_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/wavetable_osc.rs) |
| [AdditiveOsc](file:///home/per/github/pertylizer/crates/synth_modules/src/additive_osc.rs) | `AdditiveOsc` | [additive_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/additive_osc.rs) |
| [GranularOsc](file:///home/per/github/pertylizer/crates/synth_modules/src/granular_osc.rs) | `GranularOsc` | [granular_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/granular_osc.rs) |
| [LaSynth](file:///home/per/github/pertylizer/crates/synth_modules/src/la_synth.rs) | `LaSynth` | [la_synth.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/la_synth.rs) |
| [FractalOscillator](file:///home/per/github/pertylizer/crates/synth_modules/src/fractal_osc.rs) | `FractalOsc` | [fractal_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/fractal_osc.rs) |
| [PadSynth](file:///home/per/github/pertylizer/crates/synth_modules/src/padsynth.rs) | `PadSynth` | [padsynth.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/padsynth.rs) |
| [AmFormant](file:///home/per/github/pertylizer/crates/synth_modules/src/am_formant.rs) | `AmFormant` | [am_formant.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/am_formant.rs) |
| [VoiceSynth](file:///home/per/github/pertylizer/crates/synth_modules/src/voice_synth.rs) | `VoiceSynth` | [voice_synth.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/voice_synth.rs) |
| [VocalTract](file:///home/per/github/pertylizer/crates/synth_modules/src/vocal_tract.rs) | `VocalTract` | [vocal_tract.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/vocal_tract.rs) |
| [Fof](file:///home/per/github/pertylizer/crates/synth_modules/src/fof.rs) | `Fof` | [fof.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/fof.rs) |
| [RingMod](file:///home/per/github/pertylizer/crates/synth_modules/src/ring_mod.rs) | `RingMod` | [ring_mod.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/ring_mod.rs) |
| [Fooglers](file:///home/per/github/pertylizer/crates/synth_modules/src/fooglers.rs) | `Fooglers` | [fooglers.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/fooglers.rs) |

---

## 2. Filters (`ModuleCategory::Filter`)

Frequency shaping and filtering modules.

| Module | Type | File |
| :--- | :--- | :--- |
| [Filter](file:///home/per/github/pertylizer/crates/synth_modules/src/filter.rs) | `Filter` | [filter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/filter.rs) |
| [LadderFilter](file:///home/per/github/pertylizer/crates/synth_modules/src/filter.rs) | `LadderFilter` | [filter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/filter.rs) |
| [FormantFilter](file:///home/per/github/pertylizer/crates/synth_modules/src/formant_filter.rs) | `FormantFilter` | [formant_filter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/formant_filter.rs) |

---

## 3. Envelopes (`ModuleCategory::Envelope`)

Envelope generators for modulation and shaping contours.

| Module | Type | File |
| :--- | :--- | :--- |
| [Envelope](file:///home/per/github/pertylizer/crates/synth_modules/src/envelope.rs) | `Envelope` | [envelope.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/envelope.rs) |
| [Mseg](file:///home/per/github/pertylizer/crates/synth_modules/src/mseg.rs) | `Mseg` | [mseg.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/mseg.rs) |

---

## 4. LFOs & Modulation Sources (`ModuleCategory::LFO`)

Low-frequency oscillators and generative modulation generators.

| Module | Type | File |
| :--- | :--- | :--- |
| [Lfo](file:///home/per/github/pertylizer/crates/synth_modules/src/lfo.rs) | `Lfo` | [lfo.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/lfo.rs) |
| [DriftGenerator](file:///home/per/github/pertylizer/crates/synth_modules/src/drift_generator.rs) | `DriftGenerator` | [drift_generator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/drift_generator.rs) |
| [Euclidean](file:///home/per/github/pertylizer/crates/synth_modules/src/euclidean.rs) | `Euclidean` | [euclidean.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/euclidean.rs) |
| [KineticModulator](file:///home/per/github/pertylizer/crates/synth_modules/src/kinetic_modulator.rs) | `KineticModulator` | [kinetic_modulator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/kinetic_modulator.rs) |
| [RandomGates](file:///home/per/github/pertylizer/crates/synth_modules/src/random_gates.rs) | `RandomGates` | [random_gates.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/random_gates.rs) |
| [TuringMachine](file:///home/per/github/pertylizer/crates/synth_modules/src/turing_machine.rs) | `TuringMachine` | [turing_machine.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/turing_machine.rs) |
| [ChaoticOsc](file:///home/per/github/pertylizer/crates/synth_modules/src/chaotic_osc.rs) | `ChaoticOsc` | [chaotic_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/chaotic_osc.rs) |

---

## 5. Amplifiers & VCAs (`ModuleCategory::Amplifier`)

Level control modules.

| Module | Type | File |
| :--- | :--- | :--- |
| [Amplifier](file:///home/per/github/pertylizer/crates/synth_modules/src/amplifier.rs) | `Amplifier` | [amplifier.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/amplifier.rs) |

---

## 6. Mixers (`ModuleCategory::Mixer`)

Signal summing and mixing modules.

| Module | Type | File |
| :--- | :--- | :--- |
| [Mixer](file:///home/per/github/pertylizer/crates/synth_modules/src/amplifier.rs) | `Mixer` | [amplifier.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/amplifier.rs) |

---

## 7. Output (`ModuleCategory::Output`)

Stereo output stage routing.

| Module | Type | File |
| :--- | :--- | :--- |
| [StereoOutput](file:///home/per/github/pertylizer/crates/synth_modules/src/output.rs) | `StereoOutput` | [output.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/output.rs) |

---

## 8. Sampler (`ModuleCategory::Sampler`)

Sample playbacks and loading utilities.

| Module | Type | File |
| :--- | :--- | :--- |
| [Sampler](file:///home/per/github/pertylizer/crates/synth_modules/src/sampler.rs) | `Sampler` | [sampler.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/sampler.rs) |
| [AudioInput](file:///home/per/github/pertylizer/crates/synth_modules/src/audio_input.rs) | `AudioInput` | [audio_input.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/audio_input.rs) |

---

## 9. Physical Modeling (`ModuleCategory::PhysicalModeling`)

Acoustic simulation and physical model generators.

| Module | Type | File |
| :--- | :--- | :--- |
| [KeyboardPanner](file:///home/per/github/pertylizer/crates/synth_modules/src/keyboard_panner.rs) | `KeyboardPanner` | [keyboard_panner.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/keyboard_panner.rs) |
| [BodyResonance](file:///home/per/github/pertylizer/crates/synth_modules/src/body_resonance.rs) | `BodyResonance` | [body_resonance.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/body_resonance.rs) |
| [MechanicalNoise](file:///home/per/github/pertylizer/crates/synth_modules/src/mechanical_noise.rs) | `MechanicalNoise` | [mechanical_noise.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/mechanical_noise.rs) |

---

## 10. Utilities & Routing (`ModuleCategory::Utility`)

Routing, scripting, and modular signal translation tools.

| Module | Type | File |
| :--- | :--- | :--- |
| [ModMatrix](file:///home/per/github/pertylizer/crates/synth_modules/src/mod_matrix.rs) | `ModMatrix` | [mod_matrix.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/mod_matrix.rs) |
| [EnvelopeFollower](file:///home/per/github/pertylizer/crates/synth_modules/src/envelope_follower.rs) | `EnvelopeFollower` | [envelope_follower.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/envelope_follower.rs) |
| [PitchTracker](file:///home/per/github/pertylizer/crates/synth_modules/src/pitch_tracker.rs) | `PitchTracker` | [pitch_tracker.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/pitch_tracker.rs) |
| [SignalMonitor](file:///home/per/github/pertylizer/crates/synth_modules/src/signal_monitor.rs) | `SignalMonitor` | [signal_monitor.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/signal_monitor.rs) |
| [VectorMixer](file:///home/per/github/pertylizer/crates/synth_modules/src/vector_mixer.rs) | `VectorMixer` | [vector_mixer.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/vector_mixer.rs) |
| [ScriptModule](file:///home/per/github/pertylizer/crates/synth_modules/src/script_module.rs) | `Script` | [script_module.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/script_module.rs) |
| [BeatDetector](file:///home/per/github/pertylizer/crates/synth_modules/src/beat_detector.rs) | `BeatDetector` | [beat_detector.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/beat_detector.rs) |

---

## 11. Visualizers (`ModuleCategory::Visualizer`)

Signal visualizers that process no audio in-place.

| Module | Type | File |
| :--- | :--- | :--- |
| [Oscilloscope](file:///home/per/github/pertylizer/crates/synth_engine/src/visualizers/oscilloscope.rs) | `Oscilloscope` | [oscilloscope.rs](file:///home/per/github/pertylizer/crates/synth_engine/src/visualizers/oscilloscope.rs) |
| [LevelMeter](file:///home/per/github/pertylizer/crates/synth_engine/src/visualizers/level_meter.rs) | `LevelMeter` | [level_meter.rs](file:///home/per/github/pertylizer/crates/synth_engine/src/visualizers/level_meter.rs) |
| [SpectrumAnalyzer](file:///home/per/github/pertylizer/crates/synth_engine/src/visualizers/spectrum_analyzer.rs) | `SpectrumAnalyzer` | [spectrum_analyzer.rs](file:///home/per/github/pertylizer/crates/synth_engine/src/visualizers/spectrum_analyzer.rs) |

---

## 12. Audio Effects (`ModuleCategory::Effect`)

A series of audio processing units placed post-voice mixing.

| Module | Type | File |
| :--- | :--- | :--- |
| [Delay](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/delay.rs) | `Delay` | [delay.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/delay.rs) |
| [Reverb](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/reverb.rs) | `Reverb` | [reverb.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/reverb.rs) |
| [Distortion](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/distortion.rs) | `Distortion` | [distortion.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/distortion.rs) |
| [Chorus](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/chorus.rs) | `Chorus` | [chorus.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/chorus.rs) |
| [Phaser](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/phaser.rs) | `Phaser` | [phaser.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/phaser.rs) |
| [Flanger](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/flanger.rs) | `Flanger` | [flanger.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/flanger.rs) |
| [Compressor](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/compressor.rs) | `Compressor` | [compressor.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/compressor.rs) |
| [Eq](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/eq.rs) | `Eq` | [eq.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/eq.rs) |
| [Waveshaper](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/waveshaper.rs) | `Waveshaper` | [waveshaper.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/waveshaper.rs) |
| [BbdDelay](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/bbd_delay.rs) | `BbdDelay` | [bbd_delay.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/bbd_delay.rs) |
| [MidSide](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/mid_side.rs) | `MidSide` | [mid_side.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/mid_side.rs) |
| [Limiter](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/limiter.rs) | `Limiter` | [limiter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/limiter.rs) |
| [Convolver](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/convolver.rs) | `Convolver` | [convolver.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/convolver.rs) |
| [PhaseVocoder](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/phase_vocoder.rs) | `PhaseVocoder` | [phase_vocoder.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/phase_vocoder.rs) |
| [FrequencyShifter](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/frequency_shifter.rs) | `FrequencyShifter` | [frequency_shifter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/frequency_shifter.rs) |
| [EnsembleChorus](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/ensemble_chorus.rs) | `EnsembleChorus` | [ensemble_chorus.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/ensemble_chorus.rs) |
| [ShimmerReverb](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/shimmer_reverb.rs) | `ShimmerReverb` | [shimmer_reverb.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/shimmer_reverb.rs) |
| [GranularFx](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/granular_fx.rs) | `GranularFx` | [granular_fx.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/granular_fx.rs) |
| [SpectralBlur](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/spectral_blur.rs) | `SpectralBlur` | [spectral_blur.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/spectral_blur.rs) |
| [ModalResonator](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/modal_resonator.rs) | `ModalResonator` | [modal_resonator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/modal_resonator.rs) |
| [ReverseGateReverb](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/reverse_gate_reverb.rs) | `ReverseGateReverb` | [reverse_gate_reverb.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/reverse_gate_reverb.rs) |
| [TiltEq](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/tilt_eq.rs) | `TiltEq` | [tilt_eq.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/tilt_eq.rs) |
| [Univibe](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/univibe.rs) | `Univibe` | [univibe.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/univibe.rs) |
| [CrossoverSplitter](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/crossover.rs) | `CrossoverSplitter` | [crossover.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/crossover.rs) |
| [Vocoder](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/vocoder.rs) | `Vocoder` | [vocoder.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/vocoder.rs) |
