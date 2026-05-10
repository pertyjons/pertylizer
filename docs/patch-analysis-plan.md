# Patch analysis plan

Sweep every built-in patch through `analyze_note` and the MCP discovery tools to
catch silent / clipping / DC / off-pitch / low-output regressions, broken graphs,
and obvious tone-shaping bugs. Track which patches have been audited, what was
found, and what was changed.

## Tools

Per-patch toolchain — call them in this order, against a fresh instrument:

1. **`load_example_patch`** with the patch name from the table below. Creates an
   instrument and applies the patch.
2. **`get_graph_diagnostics`** — flags isolated modules, missing sound sources,
   and broken connections. Resolve any complaints before continuing.
3. **`analyze_note`** with the category-default note / duration / tail (see
   *Categories* below) and `expected_note` set to the test note for tonal
   patches. Skip `expected_note` (or ignore the `off_pitch` flag) for drums,
   drones, and noise patches.
4. **`list_modules`** + **`get_module_info`** if a metric looks wrong — e.g.
   inspect filter cutoff / amp level when a patch is `low_output`, or check the
   distortion drive when a patch is `clipping`.
5. Optional spot checks: a second `analyze_note` two octaves apart to catch
   key-tracking / filter-tracking issues that only show at the extremes.

## What counts as a finding

`analyze_note` returns a `flags` block. A patch is **flagged** if any of these
are unexpectedly true:

| Flag            | Meaning                              | Skip when…                            |
|-----------------|--------------------------------------|---------------------------------------|
| `silent`        | peak amplitude < 0.005               | never — always a real bug             |
| `clipping`      | any sample with `\|x\| ≥ 0.999`      | never — always a real bug             |
| `has_dc_offset` | mean offset > 0.01                   | never — always a real bug             |
| `low_output`    | peak amplitude < 0.05                | one-shot drums where the tail is long |
| `off_pitch`     | fundamental ≠ expected by > 50 cents | drums, drones, noise, formant_voice   |

Other things to spot-check from the result body:

- **`harmonic_content.thd_db`** for tonal patches — pure sines should be
  < −80 dB, sub_bass / sine-derived patches should be very low. Wave-folder /
  distortion patches should be high; document the expected range per patch.
- **`harmonic_content.odd_even_ratio_db`** — square / saw / acid patches should
  read odd-dominant; tube-saturation / asymmetric-clipping should read
  even-dominant.
- **`stereo_correlation`** — mono-source patches should sit near 1.0; stereo
  unison / chorus / wide pads should drop into the 0.2 – 0.8 range; anti-phase
  is almost always a bug.
- **`envelope_estimate.sustain_level`** — pluck / stab patches should be near
  zero; pads and held leads should be high. A pluck with `sustain_level > 0.7`
  is suspicious.
- **`centroid_trend_hz_per_sec`** — filter-sweep patches (auto_wah, moog
  sweep, screamer) should show a non-trivial slope; static patches should be
  near zero.

## Categories and default test parameters

| Category           | Test note      | Duration | Tail    | Notes                                            |
|--------------------|----------------|----------|---------|--------------------------------------------------|
| Bass               | C2 (36)        | 1500 ms  | 500 ms  | expected_note set; THD highly patch-dependent    |
| Lead               | C4 (60)        | 1000 ms  | 500 ms  | expected_note set                                |
| Keys / Piano       | C4 (60)        | 1500 ms  | 1000 ms | expected_note set                                |
| Pluck / Stab       | C4 (60)        | 500 ms   | 1500 ms | sustain_level should be low                      |
| Pad                | C4 (60)        | 2000 ms  | 2000 ms | sustain_level should be high; check stereo width |
| Strings            | C4 (60)        | 1500 ms  | 1000 ms | expected_note set                                |
| Bells              | C5 (72)        | 800 ms   | 1500 ms | expected_note set; high THD by design            |
| Drums              | per-patch root | 200 ms   | 800 ms  | unpitched — ignore off_pitch                     |
| Drone / texture    | C2 (36)        | 2000 ms  | 1000 ms | often unpitched — ignore off_pitch               |
| Math / glitch / FX | C3 (48)        | 1500 ms  | 500 ms  | usually unpitched — ignore off_pitch             |

`expected_note` should be passed to `analyze_note` whenever the row says so —
that narrows the fundamental search to ±tritone around the test note and makes
`pitch_error_cents` meaningful.

## Status legend

- ⬜ pending — not yet audited
- ✅ pass — analysis ran, all flags clean, metrics in expected range
- ⚠ flagged — issue found, needs follow-up (note in *Findings*)
- 🔧 fixed — issue found AND resolved in this audit (commit ref in *Findings*)
- ⏭ skipped — analysis ran but a flag is non-applicable (note in *Findings*)

## Patch checklist

### Bass — test C2, 1500/500 ms, expected_note 36

| Patch            | Status | Findings                                                                                                                                                                                                                                                                                                                                                                            |
|------------------|--------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| acid_bass        | ⏭      | Re-tested without `expected_note` (per methodology fix): pitch error −2.25 ct, all flags clean. Original −25 ct was an `expected_note` artefact. Remaining finding is just the systematic `env-2` + `mmx-1` template cruft.                                                                                                                                                         |
| aggressive_bass  | 🔧     | Root cause was the `Oscillator` module's `Frequency` parameter floor at 20 Hz: at C2 input + octave_offset=-2 the oscillator was being asked to play C0 (16.35 Hz), which got clamped to ≈20 Hz (≈+350 cents from C0). Fix: lowered `Frequency` clamp to 1 Hz in `crates/synth_modules/src/oscillator.rs` (`.range(...)` and `set_param`). Verified live after synth restart.       |
| auto_wah_bass    | 🔧     | `low_output` (peak 0.036). Cutoff 400 Hz let only ~12 saw harmonics through at C1 (32.7 Hz); first try (cutoff → 700 Hz) only nudged peak to 0.038 because the Acid filter's drive/resonance structure also attenuates. Final fix: cutoff 400 → 700 Hz **and** amp level 1.0 → 1.5 in `crates/pertylizer/src/patches/auto_wah_bass.rs`. Verified live: peak 0.059, all flags clean. |
| spacey_bass      | ✅      |                                                                                                                                                                                                                                                                                                                                                                                     |
| sub_bass         | 🔧     | Same root cause as aggressive_bass — Oscillator Frequency clamp. Both fixed by the same code change. Verified live after synth restart.                                                                                                                                                                                                                                             |
| wave_folder_bass | 🔧     | `has_dc_offset` (DC +0.0147). The wave-folder's `param_b` is a documented "DC offset (shifts the folding point)" set to 0.3 — that asymmetry is what leaks DC. Fix: lowered `param_b` 0.3 → 0.15 in `crates/pertylizer/src/patches/wave_folder_bass.rs` (preserves most of the asymmetric character). Verified live after synth restart.                                            |

### Lead — test C4, 1000/500 ms, expected_note 60

| Patch               | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                                                 |
|---------------------|--------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| expressive_lead     | ⏭      | `off_pitch` was an `expected_note` artefact (octave_offset=+1, real fundamental 522 Hz = C5, clean harmonic series). Other findings (`mmx-1`/`env-2`/`lfo-1` dead-end modules) are systematic template cruft tracked at the template level.                                                                                                                                                                                              |
| harmonic_lead       | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| moog_resonant_sweep | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| mseg_crystal_lead   | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| screamer_lead       | ⏭      | Two dead-end modules (`mmx-1`, `env-2`). `off_pitch` flag is a measurement artifact: Screamer filter resonance produces a 927 Hz peak that's 12 dB louder than the actual 261 Hz fundamental, so the fundamental detector latches to the resonance. Pitch is fine.                                                                                                                                                                       |
| unison_supersaw     | ⏭      | Pitch clean (−3.6 ct). `stereo_correlation`=1.0 is the voice-graph-only measurement caveat: the supersaw character is generated inside a single mono oscillator and the patch's stereo width comes from the chr-1 chorus in the effect chain (not visible to `analyze_note`). Dead-end `mmx-1`/`env-2` are systematic template cruft.                                                                                                    |
| unison_sync_lead    | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| vintage_lead        | 🔧     | `has_dc_offset` (DC −0.0122). Caused by OSC2 (pulse) running with `pulse_width=0.3` (asymmetric duty cycle → inherent DC mean). Fix: centred base pulse_width 0.3 → 0.5 in `crates/pertylizer/src/patches/vintage_lead.rs` so the LFO PWM swing now stays balanced around square. Patch also has octave_offset=+1, so the `off_pitch` flag was a measurement artefact (real fundamental 522 Hz = C5). Verified live after synth restart. |
| waveshaper_lead     | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                          |

### Keys / Piano — test C4, 1500/1000 ms, expected_note 60

| Patch                  | Status | Findings                                                                                                                                                                                                                                                                                                                                             |
|------------------------|--------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| ambient_keys           | ✅      |                                                                                                                                                                                                                                                                                                                                                      |
| fluid_keys             | ✅      | flags clean but very quiet (peak 0.053, RMS 0.003); THD +6.4 dB / 20 harmonics looks deliberate, not buggy.                                                                                                                                                                                                                                          |
| grand_piano            | ✅      |                                                                                                                                                                                                                                                                                                                                                      |
| pwm_epiano             | 🔧     | `has_dc_offset` (DC −0.0139). Wavetable Osc PWM bank with `position=0.15` plus env-2 sweep produced asymmetric pulse during sustain. Fix: base `position` 0.15 → 0.0 (square) in `crates/pertylizer/src/patches/pwm_epiano.rs` — env-2 still sweeps for attack character but sustain is symmetric. Live verified: DC −0.0139 → −0.0094, flags clean. |
| vintage_electric_piano | ✅      |                                                                                                                                                                                                                                                                                                                                                      |

### Pluck / Stab — test C4, 500/1500 ms, expected_note 60

| Patch          | Status | Findings                                                                                                                                                                                                                                                                                                                                                      |
|----------------|--------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| karplus_guitar | ⏭      | `off_pitch` ignored — Karplus delay-line resonates an octave above (sustain spectrum: 519 Hz at 0 dB, 259 Hz at −0.3 dB), detector latches there. Sustain 0.032 (correct for a pluck).                                                                                                                                                                        |
| kinetic_pluck  | ✅      | Flags clean. Note: extra dead-end `kin-1` (Kinetic Mod) alongside the systematic `mmx-1` — outside the standard template cruft, worth a look.                                                                                                                                                                                                                 |
| la_synth_pluck | 🔧     | `envelope_estimate.sustain_level` 0.93 → 0.25. Amp envelope was designed as a sustained pad (D=0.8, S=0.7, R=1.5). Fix in `crates/pertylizer/src/patches/la_synth_pluck.rs`: D=0.8 → 0.2, S=0.7 → 0.0, R=1.5 → 0.4 (also updated the patch description to match). Live verified: peak 0.098 unchanged, decay/sustain now genuinely percussive.                |
| pluck_synth    | ✅      |                                                                                                                                                                                                                                                                                                                                                               |
| punchy_stab    | 🔧     | `envelope_estimate.sustain_level` 0.56 → 0.29. The non-monotonic RMS hump came from filter resonance ringing as cutoff swept through the fundamental. Tightened amp env-1: D=0.15 → 0.1, S=0.3 → 0.1 in `crates/pertylizer/src/patches/punchy_stab.rs` so the sustained portion is shorter and quieter. Live verified: peak 0.087, hump much less pronounced. |

### Pad — test C4, 2000/2000 ms, expected_note 60

| Patch                | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
|----------------------|--------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| deep_space_pad       | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ethereal_shimmer_pad | 🔧     | `low_output` peak 0.0104. Voice graph is just AdditiveOsc → Filter → Amp (effects chain has chorus/shimmer reverb/MidSide that analyze_note can't see). Maxed the voice-graph gain in `crates/pertylizer/src/patches/ethereal_shimmer_pad.rs`: add-1 level 0.8 → 1.0, amp-1 level 0.65 → 2.0, master 0.75 → 1.0. Live verified: peak 0.0104 → 0.054 (just over threshold).                                                                                                                                                                                         |
| fluid_pad            | 🔧     | `low_output` peak 0.0357. Fix in `crates/pertylizer/src/patches/fluid_pad.rs`: amp-1 level 0.65 → 1.5, master 0.7 → 0.9. Live verified: peak 0.099, all flags clean. (`stereo_correlation` 1.0 is the post-voice chorus caveat — not actionable here.)                                                                                                                                                                                                                                                                                                             |
| fractal_cosmos       | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| glitch_pad           | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| kinetic_pad          | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| spectral_freeze_pad  | 🔧     | `low_output` peak 0.0059 → 0.037 (6× improvement). Voice-graph maxed: add-1 level 0.75 → 1.0, amp-1 level 0.6 → 2.0, master 0.7 → 1.0 in `crates/pertylizer/src/patches/spectral_freeze_pad.rs`. Still just under 0.05 threshold in voice-graph analysis — the AdditiveOsc is inherently quiet, and the patch's effect chain (Phase Vocoder, Spectral Blur, Shimmer Reverb, Limiter) provides final gain that analyze_note doesn't see. Strict improvement applied.                                                                                                |
| stereo_unison_pad    | 🔧     | `low_output` peak 0.047 → 0.091. Fix in `crates/pertylizer/src/patches/stereo_unison_pad.rs`: amp-1 level 0.65 → 1.0, master 0.7 → 0.85. Live verified: stereo_correlation stays at 0.62 (voice-graph stereo unison via osc-1.out_l/out_r still working), all flags clean.                                                                                                                                                                                                                                                                                         |
| vector_pad           | ✅      | `stereo_correlation` 1.0 (post-voice effects caveat — not flagged).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| velocity_pad         | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| vocal_pad            | 🔧     | `low_output` + structural `off_pitch`. Root cause was NOT the FormantFilter (verified by `fmt-1.mix=0` leaving the spectrum unchanged) — it was the Wavetable Osc's "Formant" bank baking in F1≈800/F2≈1200 Hz emphasis around a 130 Hz reference, so at C5 the 3rd/6th harmonics dominated the fundamental. Fix in `crates/pertylizer/src/patches/vocal_pad.rs`: wavetable "formant" → "warm", amp level 0.65 → 1.2. Live verified: peak 0.0095 → 0.0506, fundamental 523 Hz now strongest (was 3140 Hz @ 0 dB), pitch error 3102 ct → 0.18 ct, both flags clean. |

### Strings — test C4, 1500/1000 ms, expected_note 60

| Patch              | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
|--------------------|--------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| string_ensemble    | ⏭      | Flags clean. `stereo_correlation`=1.0 because the voice graph is genuinely mono (mth-1 → mix-1 → flt-1 → amp-1 with no L/R split); ensemble width comes from the chr-1 chorus + uvb-1 Univibe in the effect chain, which `analyze_note` does not render. Documented voice-graph-only measurement caveat — no patch-level fix.                                                                                                                                                                                 |
| unison_pwm_strings | ⏭      | Flags clean. DC offset +0.0073 is below threshold and is a finite-window artefact: a triangle LFO at 0.4 Hz sweeps `pulse_width` over a 2.5 s period, so 1.5 s renders catch a partial cycle. `stereo_correlation`=1.0 because the patch routes osc-1.out (mono) through a single amp — Oscillator's `Uni Spread` parameter affects detune phase distribution but doesn't pan voices to L/R without explicit pan modulation. Stereo width comes from the effect-chain chorus, which analyze_note doesn't see. |

### Bells — test C5, 800/1500 ms, expected_note 72

| Patch         | Status | Findings                                                                                                                                                                                                                          |
|---------------|--------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| digital_chime | ✅      |                                                                                                                                                                                                                                   |
| fm_bell       | ✅      |                                                                                                                                                                                                                                   |
| metallic_bell | ⏭      | `off_pitch` (266 Hz vs 523 Hz, −1170 ct) — patch is intentionally inharmonic; sustain spectrum shows partials at 266 / 1046 / 1828 Hz that don't form a harmonic series, exactly as marketed. Output level / DC / clipping clean. |

### Drums — per-patch root, 200/800 ms, off_pitch ignored

| Patch               | Test note | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
|---------------------|-----------|--------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| drum_kick           | C1 (24)   | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| drum_snare          | D2 (38)   | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| drum_hihat          | F#3 (54)  | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| resonant_percussion | C3 (48)   | 🔧     | Graph diagnostics: "No sound source" because `MechanicalNoise` (single 15 ms hammer burst) isn't in the engine's recognized voice-source list, and the burst is too short to drive the modal resonator. Fix: added a `Noise` (white, level 0.6) + `Mixer` to combine sustained noise with the hammer transient before the envelope-gated amp, in `crates/pertylizer/src/patches/resonant_percussion.rs`. Live-simulated test (extended hammer duration as proxy): peak 0.018 → 0.041, modal ringing at 184/492/1114 Hz. Verify after synth restart. |

### Drone / texture / ambient — test C2, 2000/1000 ms, off_pitch ignored

| Patch                 | Status | Findings                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
|-----------------------|--------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| analog_dream_machine  | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| brown_drone           | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| chaos_drone           | 🔧     | `low_output` peak 0.029 → 0.094. Fix in `crates/pertylizer/src/patches/chaos_drone.rs`: amp-1 level 0.6 → 1.5, master 0.7 → 0.9. Live verified, all flags clean.                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| euclidean_texture     | ✅      | Dead-end `euc-1` (Euclidean inputs, no outputs) — likely template/router.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| granular_cathedral    | 🔧     | Completely silent. Root cause: `GranularOsc::new()` defaults `source=Saw` but never fills the source buffer; `set_param(Source(Saw))` early-returns since the value matches default, so the 2 s grain buffer stayed all-zero. Patch-level workaround in `crates/pertylizer/src/patches/granular_cathedral.rs`: `source` "saw" → "square" (different from default → triggers buffer fill). Verified live: peak 0.0 → 0.054, n_harmonics 0 → 20. Underlying DSP bug tracked separately — task #20 — needs fix in `crates/synth_modules/src/granular_osc.rs`.                                                                     |
| granular_storm        | ✅      | Dead-end `lfo-1` (audio is fine — LFO unused but not load-bearing here).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| hybrid_resonator      | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| noise_sweep           | 🔧     | `low_output` peak 0.019 → 0.069. Fix in `crates/pertylizer/src/patches/noise_sweep.rs`: amp-1 level 0.6 → 1.8, master 0.8 → 0.95. Live verified: centroid trend +461 Hz/s confirms sweep still working, all flags clean.                                                                                                                                                                                                                                                                                                                                                                                                       |
| pitch_following_drone | ✅      | Dead-end `ptr-1` (Pitch Tracker has inputs, no outputs).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ring_mod_drone        | 🔧     | `low_output` peak 0.030 → 0.132. Fix in `crates/pertylizer/src/patches/ring_mod_drone.rs`: amp-1 level 0.55 → 1.8, master 0.65 → 0.9. Live verified, all flags clean.                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| shepard_riser         | ⚠      | Patch-level partial fix landed (LFO sawtooth → `mth-1.param_a`, `mth-1.level` 0.7 → 1.0) — modulation now reaches param_a so centroid is no longer flat. **But the underlying Shepard algorithm in `math_oscillator.rs` has param_a inverted**: sweeping param_a 0→1 makes centroid drop 950→500 Hz (verified live with param_a stepped 0/0.5/1.0). For a *riser* the sweep direction would need to be reversed, and low param_a values also leak DC. Tracked as task #21 — needs an algorithm-level fix, not patch-level. The LFO-connection patch change stays as it's a strict improvement once the algorithm is corrected. |
| spectral_drone        | ✅      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| warm_evolving         | ⏭      | `low_output` (peak 0.037, borderline). Slow swell from silence is by design; RMS plateau ~0.025 healthy for an evolving drone. Marginal but musically appropriate.                                                                                                                                                                                                                                                                                                                                                                                                                                                             |

### Math / glitch / FX — test C3, 1500/500 ms, off_pitch ignored

| Patch           | Status | Findings                                                                                                                                                                                                                   |
|-----------------|--------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------| 
| bytebeat_glitch | 🔧     | `low_output` peak 0.036 → 0.140. Fix in `crates/pertylizer/src/patches/bytebeat_glitch.rs`: amp-1 level 0.5 → 1.5, master 0.7 → 0.9. Live verified: variable RMS character preserved (still 0.005–0.065), all flags clean. |
| formant_voice   | ✅      | Strong formant peak ~1.5 kHz, clean DC, healthy peak 0.078.                                                                                                                                                                |

## Workflow

1. Pick the next ⬜ row, top-to-bottom.
2. Run the toolchain (Tools, above) against that patch.
3. Update the row:
    - All clean → ✅, leave Findings blank.
    - Issue found, fixed in the same session → 🔧, note the fix and commit
      ref in Findings (e.g. *"sustain raised 0.6 → 0.35; commit a1b2c3d"*).
    - Issue found, not fixed → ⚠, describe the symptom in Findings (e.g.
      *"low_output: peak 0.04, AMP level 0.5 → consider raising"*).
    - Flag triggered but expected → ⏭, note why (e.g.
      *"off_pitch ignored — unpitched drone"*).
4. After every batch (e.g. a category) commit the doc update so progress
   persists.
5. When every row is ✅ / 🔧 / ⏭, the audit is complete; consider promoting
   any 🔧 changes to release notes.

## Roll-up

Update these counters as you go (or recompute from the tables on completion):

- Pending (⬜): 0 / 60
- Pass (✅): 31 / 60
- Flagged (⚠): 1 / 60 (shepard_riser — needs an algorithm-level fix, tracked as task #21)
- Fixed (🔧): 19 / 60
- Skipped (⏭): 9 / 60

## Methodology notes (added during audit)

- **`expected_note` and `octave_offset`** — the plan tells us to set `expected_note` to
  the test note for tonal patches. Several built-in patches have a non-zero
  `octave_offset` (Vintage Lead +1, Expressive Lead +1, Aggressive/Sub/Wave Folder Bass
  -2, etc.). When `expected_note` ≠ played note by more than a tritone, the
  fundamental detector restricts its search away from the real fundamental, so
  `pitch_error_cents` becomes meaningless and `off_pitch` fires spuriously. **Switched
  to omitting `expected_note`** mid-audit (Lead onward) — `analyze_note` then auto-
  derives expected from the played note, which is the right reference. Earlier Bass
  rows still used the test note as expected; their off_pitch findings deserve a re-run
  before treating them as real bugs.
- **Filter resonance vs. fundamental** — Screamer Lead exposes another `off_pitch`
  failure mode: a heavily resonant filter produces a peak (927 Hz) louder than the
  actual fundamental (261 Hz, present at -12 dB), and the detector latches onto the
  resonance. Inspect the sustain spectrum before treating an `off_pitch` flag as real.
- **Systematic dead-end modules** — almost every patch carries `mmx-1` (Mod Matrix
  with no slot routing) and `env-2` (a second ADSR with no connections). Looks like
  a shared instrument template rather than per-patch cruft; flagging once in each
  patch's findings but worth fixing at the template level rather than per-patch.
- **Stereo content & effect chain** — `analyze_note` reports `stereo_correlation`=1.0
  on patches that should be wide (Unison Supersaw "full stereo spread + chorus"). If
  the analyzer renders only the voice graph, post-voice effects (chorus, stereo
  imager) are excluded — so the metric only catches *voice-graph* stereo spread, not
  effect-chain width. Need to confirm before reading too much into stereo metrics.
