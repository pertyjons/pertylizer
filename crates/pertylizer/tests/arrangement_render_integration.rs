//! Integration test for the offline arrangement renderer
//! (`crate::audio::arrangement_render::render_arrangement_to_buffer`).
//!
//! Builds a real `SynthEngine` + `SynthSession`, installs a sustaining patch
//! on a single instrument, constructs an isolated `Song` with a four-note
//! pattern placed on a track, then renders a tick range covering the pattern
//! and asserts that:
//!
//! - The buffer length matches the wall-clock duration of the tick range.
//! - The rendered audio is not silent (RMS comfortably above the noise floor).
//! - `analyze_mix_buffer` reports a finite LUFS reading and a sane peak.
//! - An empty arrangement (no placements) produces a quiet buffer rather
//!   than panicking.
//!
//! This is the smoke test for the full `engine snapshot → offline render →
//! analyze` pipeline that the `analyze_mix_bus` and `analyze_section` MCP
//! tools depend on.

mod common;

use std::sync::Arc;

use synth_core::AudioProcessor;
use synth_core::ModuleType;
use synth_core::audio::DeviceSampleRate as HwSampleRate;
use synth_engine::SynthEngine;
use synth_sequencer::{Duration as SeqDuration, PatternTick, Song};

use pertylizer::audio::arrangement_render::render_arrangement_to_buffer;
use pertylizer::audio::mix_analysis::analyze_mix_buffer;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};
use pertylizer::session::SynthSession;

use common::{
    TEST_SR, build_arpeggio_song, build_single_pattern_song, left_rms, right_rms, setup_with_patch,
    sustain_patch,
};

/// `osc → amp → out` with a `scr` Script module driving `amp.cv` from slot 1.
/// `script` is the YAMS source installed on that slot; with `out = 0.8` the amp
/// passes the oscillator at 0.8 gain, with `out = 0.0` it is fully closed.
fn scripted_amp_patch(script: &str) -> Patch {
    let mut patch = Patch::new("ScriptedAmp");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    let mut scr = ModuleBuilder::new(1, ModuleType::Script).build();
    scr.scripts.insert("1".to_string(), script.to_string());
    patch.add_module(scr);

    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch.add_connection("scr-1", "out1", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}

fn render_scripted_amp_rms(script: &str) -> f32 {
    let rig = setup_with_patch(&scripted_amp_patch(script));
    let song = build_single_pattern_song("Scripted", &[(PatternTick(0), 60, SeqDuration(3840))]);
    let shared = McpSharedState::with_song(song);
    let rendered =
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared.song, 0, 3840)
            .expect("scripted-amp render should succeed");
    left_rms(&rendered.samples)
}

#[test]
fn offline_render_installs_script_module_cv() {
    // Regression (§1): the offline loader replayed params + bypass but skipped
    // `module_snap.scripts`, so a `scr`-driven CV evaluated to 0 → silence
    // regardless of the installed script. With the script installed the amp CV
    // is 0.8 and the render must be audible.
    let rms = render_scripted_amp_rms("out = 0.8");
    assert!(
        rms > 0.01,
        "scripted amp CV (out = 0.8) should render audible content (left RMS = {rms}); \
         the offline loader likely dropped the Script-module script again"
    );
}

#[test]
fn offline_render_script_value_reaches_graph() {
    // Decisive A-vs-B check mirroring the MCP repro: only the script's *value*
    // distinguishes the two renders. `out = 0.8` opens the amp, `out = 0.0`
    // closes it — if scripts were dropped, both would render identically silent.
    let loud = render_scripted_amp_rms("out = 0.8");
    let quiet = render_scripted_amp_rms("out = 0.0");
    assert!(loud > 0.01, "out = 0.8 should be audible (RMS = {loud})");
    assert!(
        quiet < 0.001,
        "out = 0.0 should fully close the amp (RMS = {quiet})"
    );
    assert!(
        loud > quiet * 10.0,
        "the script value must drive the CV (loud {loud} vs quiet {quiet})"
    );
}

/// `osc → spp → out`, with a Mod Matrix slot routing a constant script
/// (`out = 0.9`) to the ADDRESS-based destination `spp-1.x`. `spp` is not in the
/// legacy `ModDestination` enum, so the destination only survives the offline
/// snapshot→reload if the loader preserves the full `DestAddr`. ER is off so the
/// binaural direct path gives a clean L/R imbalance when `x` moves right.
fn spp_address_routed_patch() -> Patch {
    let mut patch = Patch::new("SppAddrRouted");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::SpatialPanner)
            .param_f("er_level", 0.0)
            .param_f("direct_level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    let mut mmx = ModuleBuilder::new(1, ModuleType::ModMatrix)
        .param_choice("slot_1_dest", "spp-1.x")
        .param_b("slot_1_enabled", true)
        .build();
    // Slot 1's offset is the script output (constant 0.9) → spp.x hard right.
    mmx.scripts.insert("1".to_string(), "out = 0.9".to_string());
    patch.add_module(mmx);

    patch.add_connection("osc-1", "out", "spp-1", "in");
    patch.add_connection("spp-1", "out_l", "out-1", "in_l");
    patch.add_connection("spp-1", "out_r", "out-1", "in_r");
    patch
}

#[test]
fn offline_render_preserves_address_based_mod_matrix_destination() {
    // Regression (§7): the offline loader replayed each module param via an
    // `f32` round-trip (`with_f32(as_f32())`), which collapses the Mod Matrix
    // `SlotDestination(DestAddr)` down to a legacy enum index — dropping any
    // address the legacy `ModDestination` lacks, such as `spp-1.x`. The script
    // then routed to nothing and the spatial modulation rendered as mono. With
    // the full param preserved, `spp.x = 0.9` pans the source hard right.
    let rig = setup_with_patch(&spp_address_routed_patch());
    let song = build_single_pattern_song("Orbit", &[(PatternTick(0), 60, SeqDuration(3840))]);
    let shared = McpSharedState::with_song(song);
    let rendered =
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared.song, 0, 3840)
            .expect("spp address-routed render should succeed");

    let l = left_rms(&rendered.samples);
    let r = right_rms(&rendered.samples);
    assert!(
        l > 1e-4 && r > 1e-4,
        "render should be audible (l={l}, r={r})"
    );
    // Source hard right → the right channel is clearly louder. If the DestAddr
    // were dropped, spp.x would stay 0 and the source would sit centred (l ≈ r).
    assert!(
        r > l * 1.3,
        "address-based spp.x modulation should pan the source right (l={l}, r={r}); \
         the offline loader likely dropped the DestAddr"
    );
}

#[test]
fn renders_audible_arrangement_from_engine_snapshot() {
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    // 2 s at 120 BPM = 4 quarter notes = 3840 ticks.
    let start_tick = 0u64;
    let end_tick = 3840u64;

    let rendered = render_arrangement_to_buffer(
        &rig.session,
        &rig.sample_library,
        &shared.song,
        start_tick,
        end_tick,
    )
    .expect("arrangement render should succeed");

    // Sanity: buffer covers ~2 s at 44.1 kHz stereo.
    let frame_count = rendered.samples.len() / 2;
    let expected_frames = (rendered.duration_seconds * rendered.sample_rate as f32) as usize;
    assert!(
        frame_count.abs_diff(expected_frames) <= 1,
        "buffer length {frame_count} frames doesn't match expected {expected_frames}"
    );
    assert_eq!(rendered.start_tick, start_tick);
    assert_eq!(rendered.end_tick, end_tick);
    assert_eq!(rendered.sample_rate, TEST_SR);
    assert_eq!(rendered.channels, 2);
    assert!(
        rendered.duration_seconds > 1.8 && rendered.duration_seconds < 2.2,
        "expected ~2s render, got {} s",
        rendered.duration_seconds
    );

    // The patch is a sawtooth at full level — the rendered window must be
    // far above the silence floor. Empirically the arpeggio sits around
    // RMS ≈ 0.1; assert a much lower threshold so the test stays robust.
    let rms = left_rms(&rendered.samples);
    assert!(
        rms > 0.01,
        "rendered arrangement should be audible (left RMS = {rms})"
    );

    // No warnings on a clean render of a single-instrument song.
    assert!(
        rendered.warnings.is_empty(),
        "clean render should produce no warnings, got {:?}",
        rendered.warnings
    );

    // Run the mix-bus analyzer on the result; it should produce a finite
    // LUFS reading and a plausible peak/RMS.
    let analysis = analyze_mix_buffer(&rendered.samples, rendered.sample_rate);
    assert!(
        analysis.peak > 0.05 && analysis.peak <= 1.5,
        "peak amplitude should be audible and bounded, got {}",
        analysis.peak
    );
    assert!(
        analysis.rms > 0.005,
        "analyzer RMS should match audible content, got {}",
        analysis.rms
    );
    assert!(
        analysis.lufs_integrated > -50.0 && analysis.lufs_integrated < 0.0,
        "LUFS-I should fall in a plausible range, got {}",
        analysis.lufs_integrated
    );
    assert!(
        analysis.stereo_correlation > 0.95,
        "mono-routed patch should produce highly correlated stereo, got {}",
        analysis.stereo_correlation
    );
    assert!(
        analysis.mono_compat > 0.95,
        "mono-routed patch should be near-perfectly mono-compatible, got {}",
        analysis.mono_compat
    );
}

#[test]
fn master_volume_scales_offline_render() {
    // Regression: the offline renderer used to ignore the master fader, so
    // `set_master_volume` had no effect on `analyze_mix_bus` / `analyze_section`.
    // `new_with_scope` now sends the live master volume to the offline engine.
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);
    let (start_tick, end_tick) = (0u64, 3840u64);

    rig.session.state().master_volume.store(1.0);
    let full = render_arrangement_to_buffer(
        &rig.session,
        &rig.sample_library,
        &shared.song,
        start_tick,
        end_tick,
    )
    .expect("full-volume render should succeed");
    let full_rms = left_rms(&full.samples);

    rig.session.state().master_volume.store(0.5);
    let half = render_arrangement_to_buffer(
        &rig.session,
        &rig.sample_library,
        &shared.song,
        start_tick,
        end_tick,
    )
    .expect("half-volume render should succeed");
    let half_rms = left_rms(&half.samples);

    assert!(
        full_rms > 0.01,
        "full-volume render should be audible (left RMS = {full_rms})"
    );
    // The patch peaks below full scale, so the master gain scales linearly.
    let ratio = half_rms / full_rms;
    assert!(
        (ratio - 0.5).abs() < 0.05,
        "halving the master fader should ~halve RMS, got ratio {ratio} (full {full_rms}, half {half_rms})"
    );
}

#[test]
fn empty_arrangement_renders_silently_without_error() {
    // Same engine + patch, but the song has no tracks or placements.
    let rig = setup_with_patch(&sustain_patch());
    let song = Arc::new(synth_sequencer::SharedSong::new(Song::new("Empty")));
    let shared = McpSharedState::with_song(song);

    let rendered =
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared.song, 0, 3840)
            .expect("empty-arrangement render should succeed");

    // The buffer should be silent (or very nearly so) — no notes were
    // played. We allow a tiny non-zero RMS for engine startup transients
    // and pre-allocated buffer noise, but it should be far below an
    // audible threshold.
    let rms = left_rms(&rendered.samples);
    assert!(
        rms < 0.01,
        "empty arrangement should render near-silence, got RMS = {rms}"
    );
}

#[test]
fn render_rejects_inverted_range() {
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    // end_tick <= start_tick should return an explicit error instead of
    // panicking or producing a zero-length buffer.
    let result =
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared.song, 3840, 3840);
    assert!(result.is_err(), "zero-width range should fail");

    let result =
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared.song, 3840, 0);
    assert!(result.is_err(), "inverted range should fail");
}

/// One sustained MIDI 60 note spanning the full pattern length (3840 ticks).
/// Used to exercise the pre-roll path: a render that begins mid-note must
/// still trigger the note via Seek-from-earliest-overlap.
fn build_sustained_note_song() -> Arc<synth_sequencer::SharedSong> {
    build_single_pattern_song("Sustained", &[(PatternTick(0), 60, SeqDuration(3840))])
}

#[test]
fn render_mid_note_pre_rolls_sustained_note() {
    // A render starting after the NoteOn of a sustained note must still
    // produce audible output (the pre-roll path seeds the voice).
    let rig = setup_with_patch(&sustain_patch());
    let song = build_sustained_note_song();
    let shared = McpSharedState::with_song(song);

    // Start 1 quarter (~0.5 s @ 120 BPM) into the 4-quarter sustained note.
    let start_tick = 960u64;
    let end_tick = 2880u64;

    let rendered = render_arrangement_to_buffer(
        &rig.session,
        &rig.sample_library,
        &shared.song,
        start_tick,
        end_tick,
    )
    .expect("mid-note arrangement render should succeed");

    assert_eq!(rendered.start_tick, start_tick);
    assert_eq!(rendered.end_tick, end_tick);
    assert!(
        rendered.duration_seconds > 0.9 && rendered.duration_seconds < 1.1,
        "expected ~1s render, got {} s",
        rendered.duration_seconds
    );

    // The buffer length must match the *visible* range only — the pre-roll
    // prefix must not leak into the returned samples.
    let frame_count = rendered.samples.len() / 2;
    let expected_frames = (rendered.duration_seconds * rendered.sample_rate as f32) as usize;
    assert!(
        frame_count.abs_diff(expected_frames) <= 1,
        "buffer length {frame_count} frames doesn't match visible-range expected {expected_frames}; \
         prefix leakage suspected"
    );

    let rms = left_rms(&rendered.samples);
    assert!(
        rms > 0.01,
        "sustained-note mid-range render should be audible (left RMS = {rms}); \
         pre-roll did not seed the voice"
    );
}

#[test]
fn render_with_no_overlap_skips_preroll() {
    // Arpeggio notes (duration 900) end before the next 960-tick boundary, so
    // a render starting at tick 1920 has no in-flight note crossing it.
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let start_tick = 1920u64;
    let end_tick = 2880u64;

    let rendered = render_arrangement_to_buffer(
        &rig.session,
        &rig.sample_library,
        &shared.song,
        start_tick,
        end_tick,
    )
    .expect("non-overlap render should succeed");

    assert!(
        rendered.warnings.is_empty(),
        "non-overlap render should produce no warnings, got {:?}",
        rendered.warnings
    );
    assert!(
        rendered.duration_seconds > 0.45 && rendered.duration_seconds < 0.55,
        "expected ~0.5s render, got {} s",
        rendered.duration_seconds
    );
}

#[test]
fn render_fails_when_no_instruments_loaded() {
    // Build an engine + session but never add any instruments.
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate::new(TEST_SR),
        buffer_size: synth_core::BufferSize::new(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let sample_library: pertylizer::audio::preview::SharedSampleLibrary = Arc::new(
        std::sync::RwLock::new(synth_sampler::SampleLibrary::default()),
    );
    let result = render_arrangement_to_buffer(&session, &sample_library, &shared.song, 0, 3840);
    assert!(
        result.is_err(),
        "render should fail when no instruments are loaded"
    );
}
