//! Track-scope Mod Grid modulation must change the rendered level, through the
//! real offline render path (`render_arrangement_to_buffer`, the same path MCP
//! `render_to_wav` / `analyze_*` use). Guards the "LFO on a track's volume"
//! primary use case end to end.
//!
//! (A live investigation once showed track modulation "not working"; the root
//! cause was an orphaned track→instrument link in that song — the track pointed
//! at an instrument id that no longer existed, so *no* track-level control
//! applied. The engine/builder logic itself is exercised here and in
//! `synth_engine`'s `mod_grid_track_volume_offset_accumulates`.)

mod common;

use std::sync::Arc;

use parking_lot::RwLock;
use synth_core::ModuleType;
use synth_sequencer::{
    AudioTapNode, AudioTapSource, AutomationTarget, GlobalParam, MacroNode, ModConnection,
    ModGraphScope, ModNodeConfig, ModNodeId, ModTarget, ModuleNode, SeqInstrumentId, Song,
    TrackParam, TransportNode, TransportSource,
};

use pertylizer::audio::arrangement_render::render_arrangement_to_buffer;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};

use common::{
    add_env_amp_out_tail, build_sustained_note_song, left_rms, set_first_track_fader,
    setup_with_patch, sustain_patch,
};

/// Add a Track-scoped graph modulating this-track Volume by +0.5, assigned to
/// `track`. Includes orphan hosted modules + transport + audio_tap alongside the
/// Macro → Target routing (mirroring a real, cluttered graph) to prove they
/// don't interfere.
fn add_track_volume_graph(song: &Arc<RwLock<Song>>, track: synth_sequencer::TrackId) {
    let mut s = song.write();
    let gid = s.create_mod_graph("Mod 1");
    s.set_mod_graph_scope(gid, ModGraphScope::Track);
    s.assign_mod_graph(gid, &[track]);
    let g = s.mod_graph_mut(gid).unwrap();
    let module = |mt| {
        ModNodeConfig::Module(ModuleNode {
            module_type: mt,
            params: Default::default(),
            seed: None,
        })
    };
    g.try_insert_node(ModNodeId::new(1), module(ModuleType::Mseg))
        .unwrap();
    g.try_insert_node(ModNodeId::new(2), module(ModuleType::EnvelopeFollower))
        .unwrap();
    g.try_insert_node(ModNodeId::new(3), module(ModuleType::Euclidean))
        .unwrap();
    g.try_insert_node(ModNodeId::new(4), module(ModuleType::RandomGates))
        .unwrap();
    g.try_insert_node(
        ModNodeId::new(5),
        ModNodeConfig::Transport(TransportNode {
            source: TransportSource::BeatPhase,
        }),
    )
    .unwrap();
    g.try_insert_node(
        ModNodeId::new(6),
        ModNodeConfig::Macro(MacroNode {
            name: "M".into(),
            value: 1.0,
        }),
    )
    .unwrap();
    g.try_insert_node(
        ModNodeId::new(7),
        ModNodeConfig::AudioTap(AudioTapNode {
            source: AudioTapSource::Master,
        }),
    )
    .unwrap();
    g.try_insert_node(
        ModNodeId::new(8),
        ModNodeConfig::Target(ModTarget {
            target: AutomationTarget::Track {
                track: None,
                param: TrackParam::Volume,
            },
            amount: 0.5,
            combine: Default::default(),
        }),
    )
    .unwrap();
    g.try_connect(ModConnection::new(
        ModNodeId::new(6),
        "out",
        ModNodeId::new(8),
        "in",
    ))
    .unwrap();
}

#[test]
fn track_scope_volume_modulation_changes_the_render() {
    let rig = setup_with_patch(&sustain_patch());
    let (song, _pattern, track) = build_sustained_note_song("t");
    // Base track fader 0.4 so a +0.5 offset lands in range (→ 0.9).
    set_first_track_fader(&song, 0.4, 0.0);

    // Baseline: no mod graph.
    let base_rms = {
        let shared = McpSharedState::with_song(Arc::clone(&song));
        let base =
            render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 1920)
                .expect("baseline render");
        left_rms(&base.samples)
    };
    assert!(
        base_rms > 1e-4,
        "baseline must produce sound, got {base_rms}"
    );

    // With a track-scope graph modulating this track's volume by +0.5.
    add_track_volume_graph(&song, track);
    let mod_rms = {
        let shared = McpSharedState::with_song(Arc::clone(&song));
        let modded =
            render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 1920)
                .expect("modulated render");
        left_rms(&modded.samples)
    };

    // 0.9 / 0.4 = 2.25×; require a clear, unambiguous increase.
    assert!(
        mod_rms > base_rms * 1.5,
        "track-scope volume modulation had no effect: base_rms={base_rms}, mod_rms={mod_rms} \
         (expected ~2.25× louder)"
    );
}

/// A dark patch: sawtooth → lowpass @120 Hz → env-amp → out. The 120 Hz cutoff
/// sits below C4's 261 Hz fundamental, so the note renders very quiet until the
/// filter opens.
fn dark_filter_patch() -> Patch {
    let mut patch = Patch::new("DarkFilter");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .param_f("cutoff", 120.0)
            .build(),
    );
    add_env_amp_out_tail(&mut patch);
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch
}

/// A grid `Module` target sweeping instrument 0's filter cutoff must reach the
/// per-voice DSP through the real render path — proving `AutomationTarget::Module`
/// grid routings apply the additive `apply_mod_offset_addr` offset per active
/// voice (not the old documented no-op).
#[test]
fn module_scope_cutoff_modulation_changes_the_render() {
    let rig = setup_with_patch(&dark_filter_patch());
    let (song, _pattern, _track) = build_sustained_note_song("t");

    // Baseline: closed filter, no mod graph → dark, quiet.
    let base_rms = {
        let shared = McpSharedState::with_song(Arc::clone(&song));
        let base =
            render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 1920)
                .expect("baseline render");
        left_rms(&base.samples)
    };
    assert!(
        base_rms > 1e-5,
        "baseline must produce sound, got {base_rms}"
    );

    // A Global graph: Macro(1.0) → filter-cutoff Module target at amount 1.0.
    // The filter scales a cutoff offset by ×48 semitones, so +1.0 opens ~4
    // octaves (120 Hz → ~1.9 kHz), lifting the fundamental well into the band.
    {
        let mut s = song.write();
        let gid = s.create_mod_graph("Cutoff");
        let g = s.mod_graph_mut(gid).unwrap();
        g.try_insert_node(
            ModNodeId::new(0),
            ModNodeConfig::Macro(MacroNode {
                name: "M".into(),
                value: 1.0,
            }),
        )
        .unwrap();
        g.try_insert_node(
            ModNodeId::new(1),
            ModNodeConfig::Target(ModTarget {
                target: AutomationTarget::Module {
                    instrument: SeqInstrumentId(0),
                    module_type: ModuleType::Filter,
                    instance: 1,
                    param_id: "cutoff".into(),
                },
                amount: 1.0,
                combine: Default::default(),
            }),
        )
        .unwrap();
        g.try_connect(ModConnection::new(
            ModNodeId::new(0),
            "out",
            ModNodeId::new(1),
            "in",
        ))
        .unwrap();
    }

    let mod_rms = {
        let shared = McpSharedState::with_song(Arc::clone(&song));
        let modded =
            render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 1920)
                .expect("modulated render");
        left_rms(&modded.samples)
    };

    // Opening the filter adds the fundamental + harmonics: a clear, large rise.
    assert!(
        mod_rms > base_rms * 1.5,
        "module-scope cutoff modulation had no effect: base_rms={base_rms}, mod_rms={mod_rms}"
    );
}

/// A grid `Instrument · Volume` target (channel-level, not per-voice) must lift
/// the rendered level — proving the `grid_instrument_offsets` store composes into
/// the channel-bus fader.
#[test]
fn instrument_scope_volume_modulation_changes_the_render() {
    let rig = setup_with_patch(&sustain_patch());
    let (song, _pattern, _track) = build_sustained_note_song("t");

    let base_rms = {
        let shared = McpSharedState::with_song(Arc::clone(&song));
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 1920)
            .map(|b| left_rms(&b.samples))
            .expect("baseline render")
    };
    assert!(
        base_rms > 1e-4,
        "baseline must produce sound, got {base_rms}"
    );

    // Macro(1.0) → Instrument 0 Volume at amount +0.5. Instrument fader default 1.0
    // → (1.0 + 0.5).clamp(0,2) = 1.5× louder.
    {
        let mut s = song.write();
        let gid = s.create_mod_graph("Inst vol");
        let g = s.mod_graph_mut(gid).unwrap();
        g.try_insert_node(
            ModNodeId::new(0),
            ModNodeConfig::Macro(MacroNode {
                name: "M".into(),
                value: 1.0,
            }),
        )
        .unwrap();
        g.try_insert_node(
            ModNodeId::new(1),
            ModNodeConfig::Target(ModTarget {
                target: AutomationTarget::Instrument {
                    instrument: SeqInstrumentId(0),
                    param: synth_sequencer::AutoInstrumentParam::Volume,
                },
                amount: 0.5,
                combine: Default::default(),
            }),
        )
        .unwrap();
        g.try_connect(ModConnection::new(
            ModNodeId::new(0),
            "out",
            ModNodeId::new(1),
            "in",
        ))
        .unwrap();
    }

    let mod_rms = {
        let shared = McpSharedState::with_song(Arc::clone(&song));
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 1920)
            .map(|b| left_rms(&b.samples))
            .expect("modulated render")
    };
    assert!(
        mod_rms > base_rms * 1.3,
        "instrument-volume modulation had no effect: base={base_rms}, mod={mod_rms} (expected ~1.5×)"
    );
}

/// A cheap source feeding a hosted module's control input (`Macro → LFO.rate_cv`)
/// must reach the DSP as a block-constant injection. An LFO whose rate is driven
/// by a Macro renders differently from the same LFO at its base rate — proving the
/// injection changed the module's processing (not a dropped cable).
#[test]
fn cheap_to_module_injection_changes_the_render() {
    let rig = setup_with_patch(&sustain_patch());
    let (song, _pattern, _track) = build_sustained_note_song("t");
    set_first_track_fader(&song, 0.6, 0.0);

    // Baseline graph: LFO (1 Hz) → master volume, no rate injection.
    let gid = {
        let mut s = song.write();
        let gid = s.create_mod_graph("Wobble");
        let g = s.mod_graph_mut(gid).unwrap();
        let mut lfo_params = std::collections::BTreeMap::new();
        lfo_params.insert("rate".to_string(), 1.0);
        g.try_insert_node(
            ModNodeId::new(0),
            ModNodeConfig::Module(ModuleNode {
                module_type: ModuleType::Lfo,
                params: lfo_params,
                seed: None,
            }),
        )
        .unwrap();
        g.try_insert_node(
            ModNodeId::new(1),
            ModNodeConfig::Target(ModTarget {
                target: AutomationTarget::Global(GlobalParam::MasterVolume),
                amount: 0.4,
                combine: Default::default(),
            }),
        )
        .unwrap();
        g.try_connect(ModConnection::new(
            ModNodeId::new(0),
            "out",
            ModNodeId::new(1),
            "in",
        ))
        .unwrap();
        gid
    };
    let base = {
        let shared = McpSharedState::with_song(Arc::clone(&song));
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 1920)
            .expect("baseline render")
    };

    // Add Macro(1.0) → LFO.rate_cv: the injected rate_cv drives the LFO's FM, so
    // it now oscillates faster than its 1 Hz base rate.
    {
        let mut s = song.write();
        let g = s.mod_graph_mut(gid).unwrap();
        g.try_insert_node(
            ModNodeId::new(2),
            ModNodeConfig::Macro(MacroNode {
                name: "Rate".into(),
                value: 1.0,
            }),
        )
        .unwrap();
        g.try_connect(ModConnection::new(
            ModNodeId::new(2),
            "out",
            ModNodeId::new(0),
            "rate_cv",
        ))
        .unwrap();
    }
    let injected = {
        let shared = McpSharedState::with_song(Arc::clone(&song));
        render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 1920)
            .expect("injected render")
    };

    // Without the injection reaching the LFO the two runtimes are identical, so any
    // clear difference proves the cheap→module cable took effect.
    let max_diff = base
        .samples
        .iter()
        .zip(&injected.samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_diff > 1e-3,
        "cheap→module injection had no effect on the LFO (max sample diff {max_diff})"
    );
}
