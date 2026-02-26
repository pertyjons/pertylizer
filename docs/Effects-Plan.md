# Effects Implementation Plan (No Code)

This document defines six new effects at the algorithm and parameter level. It is intended for another AI to implement without ambiguity. All formulas assume sample rate `sr` and interleaved stereo buffers unless noted. Each effect is designed to be real-time safe (no heap allocation in the audio thread).

## 1) Juno-Style Ensemble Chorus (BBD)

**Module id:** `ensemble_chorus`

**Goal:** Classic Juno/Jupiter-style ensemble width using short BBD delays, inverted LFO phases, and band-limited repeats.

**Inputs/Outputs:**
- Input: stereo interleaved `xL[n], xR[n]`
- Output: stereo interleaved `yL[n], yR[n]`

**Parameters (suggested ranges/defaults):**
- `rate_hz` (0.2..1.5, default 0.6)
- `depth_ms` (0.2..2.5, default 1.2)
- `base_delay_ms` (5..20, default 12)
- `mix` (0..1, default 0.5)
- `tone` (0..1, default 0.4) -> lowpass cutoff in delay path
- `noise` (0..1, default 0.1) -> BBD clock noise amount
- `stereo_width` (0..1, default 1.0)
- `voices` (2 or 3, default 2)

**Algorithm (per sample):**
1. Maintain 2 (or 3) short delay lines per channel. Delay line length = `max_delay_ms` * `sr`.
2. LFO: `lfo = sin(2*pi*rate_hz*t + phase)`.
3. Delay time in samples for left/right:
   - `dL = (base_delay_ms + depth_ms * lfo) * sr / 1000`
   - `dR = (base_delay_ms - depth_ms * lfo) * sr / 1000` (inverted phase)
4. Read delay with linear interpolation.
5. Apply band-limited tone inside delay loop (1-pole lowpass):
   - `c = exp(-2*pi*fc/sr)`, where `fc` mapped from `tone` (e.g., 1k..8k).
6. Add subtle noise to delayed samples: `noise_amt * white_noise()`.
7. Sum voices and mix: `y = dry*(1-mix) + wet*mix`.
8. Apply `stereo_width`: `mid = (L+R)/2`, `side = (L-R)/2`, `side *= stereo_width`.

**Notes:** Use the existing BBD delay building blocks for tone + noise, but constrain delay to 5-20 ms and keep feedback at 0.

---

## 2) Shimmer Reverb

**Module id:** `shimmer_reverb`

**Goal:** Large, glowing reverb with pitch-shifted feedback (octave or fifth up).

**Inputs/Outputs:**
- Input: stereo interleaved
- Output: stereo interleaved

**Parameters:**
- `room_size` (0..1, default 0.6)
- `decay` (0..1, default 0.7)
- `damping` (0..1, default 0.4)
- `pre_delay_s` (0..0.5, default 0.0)
- `pitch_semitones` (-12..+24, default +12)
- `shimmer_mix` (0..1, default 0.35) -> how much of the feedback is pitch shifted
- `mix` (0..1, default 0.4)

**Algorithm:**
1. Use the existing FDN reverb core (8 channels) as the base.
2. In the feedback path, add a pitch shifter (phase vocoder) on a copy of the reverb output.
3. Feedback signal:
   - `fb = reverb_out * (1 - shimmer_mix) + pitch_shift(reverb_out, pitch_semitones) * shimmer_mix`
4. Feed `fb` back into the FDN with gain `g = 0.3 + decay*0.67`.
5. Apply damping lowpass inside the FDN feedback path as in current reverb.
6. Output mix: `y = dry*(1-mix) + reverb_out*mix`.

**Pitch shift formula:**
- `ratio = 2^(semitones/12)`
- Use phase vocoder (existing module) with `ratio` and optional window size 1024..2048.

---

## 3) Granular FX / Cloud Delay (Input-Driven)

**Module id:** `granular_fx`

**Goal:** Capture incoming audio into a ring buffer and spawn grains for clouds, time-smear, and freeze.

**Inputs/Outputs:**
- Input: stereo interleaved
- Output: stereo interleaved

**Parameters:**
- `buffer_s` (0.5..4.0, default 2.0)
- `grain_ms` (10..120, default 40)
- `density` (0..1, default 0.6) -> grains/sec mapping 1..60
- `position` (0..1, default 0.5) -> base read pos
- `position_spread` (0..1, default 0.3)
- `pitch_spread` (0..1, default 0.2) -> +/- 12 semitones
- `pan_spread` (0..1, default 0.5)
- `freeze` (0/1, default 0)
- `mix` (0..1, default 1.0)

**Algorithm:**
1. Maintain a stereo ring buffer of length `buffer_s * sr`.
2. Continuously write input into the ring buffer (unless `freeze=1`, then stop writing).
3. Spawn grains at a rate `grains_per_sec = 1 + density*59`.
4. For each grain:
   - Choose start position: `pos = position + rand_bipolar()*position_spread*0.5`, clamp 0..1.
   - Start index: `start = pos * (buffer_len - grain_len)`.
   - Grain length: `grain_len = grain_ms * sr / 1000` (clamp >= 16 samples).
   - Pitch: `rate = 2^(rand_bipolar() * pitch_spread * 12 / 12)`.
   - Pan: `pan = rand_bipolar() * pan_spread`.
5. Per grain sample read:
   - `read_pos = start + pos_in_grain * rate` (linear interpolation).
6. Envelope: Hann window `w(t) = 0.5 * (1 - cos(2*pi*t))`, `t in [0,1]`.
7. Output = sum of active grains, then `mix` with dry signal.

---

## 4) Spectral Blur / Smear

**Module id:** `spectral_blur`

**Goal:** Smear transients into sustained, dreamy textures via STFT magnitude smoothing.

**Inputs/Outputs:**
- Input: stereo interleaved
- Output: stereo interleaved

**Parameters:**
- `fft_size` (512..4096, default 1024)
- `blur_time` (0..1, default 0.7) -> temporal smoothing
- `blur_freq` (0..1, default 0.3) -> spectral smoothing
- `freeze` (0/1, default 0)
- `mix` (0..1, default 1.0)

**Algorithm:**
1. Perform STFT per channel with Hann window and hop `H = N/4`.
2. For each frame `n`, bin `k`:
   - `mag = |X[k,n]|`, `phase = arg(X[k,n])`.
3. Temporal blur (IIR):
   - `mag_t[k,n] = (1-a) * mag + a * mag_t[k,n-1]`, where `a = blur_time`.
4. Spectral blur (FIR over frequency):
   - `mag_f[k] = sum_{i=-M..M} w_i * mag_t[k+i]`, where `M` from `blur_freq`.
5. If `freeze=1`, hold `mag_f` constant per bin until freeze ends.
6. Reconstruct spectrum with original phase:
   - `Y[k] = mag_f[k] * exp(i*phase)`.
7. ISTFT, overlap-add, then dry/wet mix.

**Notes:** If phase artifacts are too strong, optionally use phase accumulation from previous frames (phase vocoder style).

---

## 5) Modal Resonator Bank

**Module id:** `modal_resonator`

**Goal:** Turn any input into tuned, ringing resonances (Rings-like), great for drones and percussive hits.

**Inputs/Outputs:**
- Input: mono or stereo interleaved
- Output: stereo interleaved

**Parameters:**
- `base_note` (MIDI 24..96, default 60)
- `spread` (0..1, default 0.3) -> detune/inharmonicity
- `modes` (4..16, default 8)
- `decay` (0..1, default 0.6)
- `brightness` (0..1, default 0.5) -> controls Q and high modes level
- `mix` (0..1, default 1.0)

**Algorithm:**
1. Create `modes` bandpass resonators or 2-pole resonators.
2. Mode frequencies:
   - `f0 = midi_to_hz(base_note)`
   - `f_i = f0 * (i+1) * (1 + spread * rand_bipolar() * 0.02)`
3. For each mode use a biquad bandpass (constant skirt gain) with Q derived from decay:
   - `Q = 0.5 + decay * 20.0`
   - Standard biquad coefficients (not shown here; use textbook).
4. Sum all mode outputs with amplitude scaling (higher modes reduced by brightness).
5. Output = sum of modes, then dry/wet mix.

**Notes:** A simple exciter input can be the incoming audio or a short burst if a trigger/gate input exists.

---

## 6) Reverse / Gated Reverb (Buffer Stutter)

**Module id:** `reverse_gate_reverb`

**Goal:** Dramatic reverse tails or gated bursts using short buffer capture and envelope shaping.

**Inputs/Outputs:**
- Input: stereo interleaved
- Output: stereo interleaved

**Parameters:**
- `window_ms` (100..2000, default 600)
- `mode` (reverse, gate, stutter; default reverse)
- `trigger` (threshold or periodic; default periodic)
- `threshold` (-60..0 dB, default -24 dB)
- `gate_ms` (20..500, default 120)
- `mix` (0..1, default 1.0)

**Algorithm:**
1. Maintain a capture buffer of length `window_ms * sr` per channel.
2. Capture policy:
   - If `trigger=periodic`: capture new window every `window_ms`.
   - If `trigger=threshold`: capture when RMS exceeds `threshold`.
3. Playback:
   - Reverse mode: read index `read = start + (window_len - 1 - pos)`.
   - Gate mode: read forward but apply gate envelope `g[n]`.
   - Stutter mode: loop the captured window forward.
4. Envelope (reverse/gate):
   - `g[n] = exp(-n / (gate_ms * sr / 1000))` or linear ramp.
5. Mix with dry signal.

**Notes:** This is not a full reverb; if needed, pass output through existing reverb for longer tails.

---

## Implementation Notes (Shared)
- All effects should avoid heap allocations in `process`.
- Use existing `InterpolatedDelayLine`, `FilterState`, and STFT utilities where possible.
- All parameters should map to normalized UI controls consistent with existing modules.
