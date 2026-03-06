# Plan: 0.5 — Utilize all telemetry data in effects

> Referenced from [TODO.md](TODO.md) section 0.5

## Current telemetry usage audit

| Telemetry field      | Effects using it | Count |
|----------------------|------------------|-------|
| FFT bands            | fft_bars, waveform_ring, spectral_waterfall, spectral_cathedral, pulse_terrain, ferrofluid_tendrils, spectral_origami, centroid_nebula (indirect), flux_supernova (indirect) | 9 |
| RMS                  | rms_light, beat_pulse, centroid_nebula, flux_supernova, fractal_pulse, ferrofluid_tendrils, pulse_terrain, spectral_cathedral, spectral_origami, harmonic_ribbons, chord_bloom | 11 |
| Notes/Velocity       | note_particles, velocity_meteors, harmonic_ribbons, chord_bloom, neon_calligraphy, instrument_cubes + (last_note_on in several) | 9 |
| Centroid             | centroid_nebula, spectral_origami, debug_hud | 3 |
| Flux                 | flux_supernova, debug_hud | 2 |
| Beat phase           | beat_pulse | 1 |
| Voice count          | cpu_overdrive_core | 1 |
| Peak levels          | (none) | 0 |
| CC / Pitch bend      | Not even stored in SynthTelemetry | 0 |
| Instrument category  | instrument_cubes | 1 |
| Transport state      | beat_pulse, phase_rings | 2 |
| Tempo                | fractal_pulse | 1 |
| CPU usage            | cpu_overdrive_core | 1 |

## Theme extension needed

Currently `ThemeConfig` provides uniform material offsets (saturation_offset, lightness_offset, emissive_multiplier, metallic, roughness). To make telemetry-driven color changes theme-aware, the theme must specify *how* telemetry maps to visual changes per theme:

- **Ember** (warm): centroid hue 0-60 (red-yellow)
- **Arctic** (cold): centroid hue 180-240 (cyan-blue)
- **Neon**: centroid hue 270-360 (magenta-red)
- **Void**: centroid hue 0-0 (monochrome white)

Same logic applies to flux burst colors, beat pulse intensity, peak flash hues, etc.

---

## Step 1: Extend `ThemeConfig` with telemetry-reactive parameters

Add to `ThemeConfig` and propagate through `ThemeMaterialPolicy`:

```rust
// Centroid-driven hue mapping (spectral brightness -> color)
pub centroid_hue_low: f32,       // Hue when centroid is low (dark/bass)
pub centroid_hue_high: f32,      // Hue when centroid is high (bright/treble)

// Flux-driven burst
pub flux_burst_hue: f32,         // Hue for flux spike accents
pub flux_intensity_scale: f32,   // How much flux amplifies emissive (1.0 = normal)

// Beat-driven pulse
pub beat_pulse_strength: f32,    // How much beat phase modulates brightness (0.0-1.0)

// RMS/Peak dynamics
pub peak_flash_hue: f32,         // Hue for transient flashes
pub rms_emissive_scale: f32,     // How much RMS scales emissive (0.5-2.0)
```

Lerp these during theme transitions and bump `policy.version`.

**Files:** `theme.rs` — `ThemeConfig`, `ThemeMaterialPolicy`, `apply_theme()`

## Step 2: Store CC/Pitch bend in `SynthTelemetry`

Currently `/synth/event/cc` is received but discarded. Add:

```rust
pub last_cc: Option<(u8, f32, u8)>,  // (cc_number, value, channel)
pub pitch_bend: f32,                  // Normalized -1.0 to 1.0
pub aftertouch: f32,                  // Normalized 0.0 to 1.0
```

**Files:** `telemetry.rs`, `osc_receiver.rs`

## Step 3: Shared `telemetry_color` helpers

Create utility functions that effects call instead of hardcoding telemetry-to-visual mappings:

```rust
/// Map centroid to hue using active theme's range
pub fn centroid_to_hue(centroid_hz: f32, policy: &ThemeMaterialPolicy) -> f32;

/// Compute flux-driven emissive boost
pub fn flux_emissive_boost(flux: f32, policy: &ThemeMaterialPolicy) -> f32;

/// Compute beat-phase pulse factor (0.0-1.0)
pub fn beat_pulse_factor(beat_phase: f32, policy: &ThemeMaterialPolicy) -> f32;

/// Map RMS to emissive scale
pub fn rms_to_emissive(rms: f32, policy: &ThemeMaterialPolicy) -> f32;
```

**File:** new `visuals/telemetry_color.rs` (or add to `effects.rs`)

## Step 4: Integrate telemetry into effects

### 4a. Centroid -> hue-shift (15+ effects)

All effects using `create_hue_materials()` get a centroid-driven hue offset applied during material updates. Centroid shift is an *offset on top of* existing hue range, not a replacement.

**Effects:** fft_bars, waveform_ring, spectral_waterfall, spectral_cathedral, pulse_terrain, ferrofluid_tendrils, spectral_origami, harmonic_ribbons, chord_bloom, velocity_meteors, note_particles, neon_calligraphy, centroid_nebula

### 4b. Flux -> intensity spikes (10+ effects)

Effects already reading RMS add flux-driven emissive burst:

```rust
let burst = flux_emissive_boost(tel.flux, &policy);
emissive *= 1.0 + burst;
```

**Effects:** All terrain + ambient + hero effects

### 4c. Beat phase -> pulsing (8+ effects)

Rhythmic effects sync scale/brightness to beat phase:

```rust
let pulse = beat_pulse_factor(tel.beat_phase, &policy);
transform.scale *= 1.0 + pulse * 0.1;
```

**Effects:** fft_bars, phase_rings, fractal_pulse, spectral_cathedral, pulse_terrain, waveform_ring, beat_pulse, ferrofluid_tendrils

### 4d. Velocity -> brightness/size (terrain + ambient)

Extend velocity usage beyond transient-only effects.

**Effects:** rms_light (flash on high velocity), spectral_cathedral (arch brightness), pulse_terrain (ground intensity)

### 4e. Voice count -> visual density

Scale particle counts, geometry density, recursion depth.

**Effects:** centroid_nebula (more particles), note_particles (burst size), fractal_pulse (recursion depth)

### 4f. Peak levels -> transient flashes

```rust
let peak_mono = (tel.peak[0] + tel.peak[1]) * 0.5;
if peak_mono > PEAK_THRESHOLD {
    // Camera shake / flash
}
```

**Effects:** rms_light (flash), beat_pulse (brightness spike), all terrain effects

### 4g. CC/Pitch bend -> continuous modulation

Pitch bend -> rotate/scale, CC (filter) -> visual sweep.

**Effects:** harmonic_ribbons (ribbon width), spectral_origami (fold angle), neon_calligraphy (stroke width)

### 4h. CPU / transport / instrument -> scene-level

- CPU -> global glitch/distortion hint (all effects via policy)
- Transport stopped -> effects slow down / freeze
- Instrument category -> color layer per instrument (spread instrument_cubes pattern)

## Step 5: Theme values for all 8 themes

| Theme     | centroid_hue low/high | flux_burst_hue | beat_pulse | peak_flash | rms_emissive |
|-----------|-----------------------|----------------|------------|------------|--------------|
| Neon      | 270 / 360             | 60 (yellow)    | 0.8        | 120 (green)| 1.5          |
| Metal     | 200 / 260             | 40 (orange)    | 0.3        | 0 (white)  | 0.8          |
| Glass     | 180 / 300             | 200 (cyan)     | 0.5        | 180 (cyan) | 1.0          |
| Space     | 220 / 280             | 270 (purple)   | 0.6        | 60 (yellow)| 1.2          |
| Synthwave | 300 / 360             | 180 (cyan)     | 0.9        | 300 (pink) | 1.4          |
| Ember     | 0 / 60                | 30 (orange)    | 0.7        | 45 (amber) | 1.6          |
| Arctic    | 180 / 240             | 200 (ice-blue) | 0.4        | 210 (light)| 0.7          |
| Void      | 0 / 0 (mono)          | 0 (white)      | 0.2        | 0 (white)  | 2.0          |

## Step 6: Testing
 
- Unit tests: `centroid_to_hue()` returns correct hue for each theme
- Integration: every effect compiles and renders without regression
- `cargo build && cargo clippy && cargo test && cargo fmt --check`

---

## Implementation order

1. **Steps 1 + 2** — Extend theme + store CC (infrastructure, nothing visible yet)
2. **Step 3** — Shared helpers
3. **Step 4a** — Centroid in all effects (biggest visual impact)
4. **Step 4b** — Flux spikes
5. **Step 4c** — Beat phase sync
6. **Steps 4d-4h** — Remaining telemetry
7. **Step 5** — Fine-tune theme values
8. **Step 6** — Tests

## Risks

- **Visual chaos**: Too many reactive parameters simultaneously can make effects messy. Mitigation: subtle defaults, theme-specific tuning.
- **Performance**: More material updates per frame. Mitigation: same epsilon-check and version-tracking already in place.
- **Centroid-hue conflicts with existing hue-range**: Centroid shift should be an *offset* on top of existing hue, not replace it.
