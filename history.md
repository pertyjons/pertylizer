# Version History

## [0.13.0] - 2024

### Added
- **Math Oscillator Module** - Advanced oscillator with 18 mathematical synthesis algorithms:
  - **Phase-based (Stateless):**
    - SineFM - Basic FM synthesis with carrier/modulator
    - TanChaos - Tan distortion with noise
    - SuperSaw - Multiple detuned sawtooth waves
    - BitWise - Digital glitch/bytebeat style
    - WaveFolder - West coast style wave folding
    - Formant - Vocal formant simulation
    - PhaseDist - Casio CZ style phase distortion
    - Metallic - Ring modulation for metallic tones
    - Fractal - Weierstrass-like fractal function
    - Chebyshev - Chebyshev polynomial waveshaping
    - Walsh - Walsh function synthesis
    - Pulsar - Pulsar synthesis (windowed sine bursts)
    - Shepard - Infinite rising/falling tone illusion
  - **Iterative/Chaotic (Stateful):**
    - Bytebeat - Classic bytebeat formula synthesis
    - Lorenz - Lorenz strange attractor chaos
    - Logistic - Logistic map chaos
    - FeedbackFM - Self-modulating FM
  - **Buffer-based:**
    - KarplusStrong - Physical modeling plucked string

- **GUI Integration** - Math Oscillator available in module palette under Oscillator submenu
- **6 New Example Patches:**
  - Chaos Drone - Evolving textures using Lorenz attractor
  - Karplus Guitar - Physical modeling plucked string
  - Shepard Riser - Infinite rising tone effect
  - Bytebeat Glitch - Retro digital algorithmic music
  - Wave Folder Bass - West coast synthesis bass
  - Formant Voice - Vocal-like synthesis

### Technical Details
- New `MathAlgo` enum in typed_params.rs with 18 algorithm variants
- New `MathOscillatorParam` enum for module parameters
- Internal state management for chaos attractors and delay lines
- CV modulation inputs for Param A and Param B

---

## [0.12.0] - 2024

### Added
- Initial modular synthesizer implementation
- Basic modules: Oscillator, Filter, Envelope, LFO, Amplifier, Mixer
- Effects: Delay, Reverb, Distortion, Chorus, Phaser, Flanger, Compressor, EQ
- Visualizers: Oscilloscope, Level Meter
- Stereo Output module
- Patch save/load system (JSON format)
- 10 example patches:
  - Deep Space Pad
  - Aggressive Bass
  - Vintage Lead
  - Ambient Keys
  - Kick Drum
  - Snare Drum
  - Hi-Hat
  - Pluck Synth
  - FM Bell
  - Noise Sweep
- Piano keyboard for note input
- Module palette for adding modules
- Visual cable connections
- Typed parameter system with compile-time safety
