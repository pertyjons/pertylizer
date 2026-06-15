# Pertylizer Screenshots

A visual tour of Pertylizer's main views.

## Patch Editor (Rack)

The modular patch editor where instruments are built by connecting DSP modules
(oscillators, filters, envelopes, LFOs, modulation, effects) into a signal
graph.

### `rack-1.png`

![Rack 1](rack-1.png)

A complete instrument patch: a WaveTable oscillator feeds a Filter, then a
Mixer, then the Stereo output, with an LFO and Envelope driving modulation
inputs.

### `rack-2.png`

![Rack 2](rack-2.png)

The patch editor with the *Add Module* context menu open, showing the module
categories (Audio, Filter, Envelope, LFO, Mixer, Distortion, Modulation,
Generators, Output, Wav Module).

### `rack-3.png`

![Rack 3](rack-3.png)

A larger patch with multiple oscillators, filters, an envelope, a Mod Matrix
and a Chorus — illustrating richer modulation routing.

## Mod Matrix — YAMS Control Scripts

### `yams-1.png`

![YAMS 1](yams-1.png)

The *YAMS Script Demo* instrument with the Mod Matrix expression editor open. Each
Mod Matrix routing (right-hand panel) can run a small YAMS control script instead of
a scalar amount — the script's `out` value becomes the modulation offset, evaluated
per voice in real time. The popup edits slot 5's script
(`out = sin(phasor(0.3) * tau) * smoothstep(0, 1.5, age) * 0.7`, a note-age auto-pan)
with a live compile status and Format / Apply / Clear / Close buttons. A scripted
routing lights its ƒx marker, and the script reads its sources (LFOs, envelopes,
macros like `velocity`/`age`) straight from the expression.

## Sequencer / Arrangement

### `seq-1.png`

![Sequencer 1](seq-1.png)

The song arrangement view: tracks (Drums, Bass, Pad, Lead, Strings, etc.)
laid out on a horizontal timeline with their pattern clips, plus a piano-roll
preview of the selected pattern at the bottom.

## Acoustic World Engine (AWE)

### `awe-1.png`

![AWE 1](awe-1.png)

The Acoustic World Engine — a 3D room simulator with a sound source and a
listener positioned in space, showing reflection paths. The right-hand panel
exposes Room dimensions, Material, Mix, Effects and Spatial parameters.

## Visualizer

The realtime 3D audio visualizer with several scenes driven by the audio
stream.

### `visualizer-1.png`

![Visualizer 1](visualizer-1.png)

Circular spectrum bars with floating particles, reacting to the frequency
content of the audio.

### `visualizer-2.png`

![Visualizer 2](visualizer-2.png)

A checkerboard floor scene with confetti particles, modulated by the audio
signal.

### `visualizer-3.png`

![Visualizer 3](visualizer-3.png)

A wireframe terrain that deforms with the audio, with bouncing spheres on top.
