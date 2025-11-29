# Version History

## [0.13.2] - 2024

### Fixed
- **Command Queue Saturation** - Critical fix for patch loading reliability
  - Increased command buffer from 1024 to 16384 entries
  - Added `send_blocking()` method for reliable patch loading
  - Commands no longer silently dropped when buffer is full
- **Real-time Safety** - Modules now dropped on main thread, not audio thread
  - New return channel sends removed modules back to GUI for cleanup
  - Prevents audio dropouts (glitches) during module removal
  - `cleanup_dropped_modules()` called each frame in GUI

### Technical Details
- `COMMAND_BUFFER_SIZE` increased to 16384
- `EngineHandle::send_blocking()` waits for queue space with timeout protection
- `DroppedModule` wrapper and return channel (`RETURN_BUFFER_SIZE: 256`)
- `ModuleGraph::remove_module_and_return()` for deferred cleanup
- `load_patch_data()` uses blocking sends for ClearAllModules, Connect, and settings

---

## [0.13.1] - 2024

### Added
- **Pink Noise** - New waveform for the Oscillator module using Voss-McCartney algorithm
  - Softer, warmer noise with equal energy per octave (-3dB/octave slope)
  - Great for wind effects, percussion, and pads
- **Linear FM Mode** - New FM mode option for Oscillator
  - Exponential: Classic 1V/octave style (pitch tracking)
  - Linear: Hz-based FM for stable harmonic ratios across all pitches
  - Essential for bell-like and metallic FM tones
- **Velocity Sensitivity** - Exposed in Envelope module
  - Control how much note velocity affects envelope amplitude
  - Range 0 (ignore) to 1 (full response)
- **Filter Envelope Amount** - New parameter for Filter module
  - Scale envelope CV from -1.0 to +1.0
  - Enables inverted envelope response for "rubber band" bass sounds

### Technical Details
- `Waveform::PinkNoise` variant added with 16-row Voss-McCartney generator
- `FmMode` enum (Exponential/Linear) with `OscillatorParam::FmMode`
- `EnvelopeParam::VelocitySensitivity` exposed in descriptor
- `FilterParam::EnvAmount` scales `cutoff_cv` input

---

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
