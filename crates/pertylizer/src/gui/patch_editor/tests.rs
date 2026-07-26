//! Tests for `patch_analysis_tests`.

use super::*;

/// Seed a Mod Matrix panel's slot source/dest **addresses** (S1.5c), mirroring
/// what `sync_module_params` writes from the engine. `slot_num` is 1-based.
fn matrix_panel(slot_setups: &[(usize, &str, &str, bool)]) -> ModulePanelState {
    let mut state =
        ModulePanelState::new(ModuleId::new(ModuleType::ModMatrix, 1), Pos2::new(0.0, 0.0));
    for (slot_num, src, dst, enabled) in slot_setups {
        let slot = (*slot_num - 1) as u8;
        state.slot_addrs.insert(
            ModMatrixParam::SlotSource(slot, None).name().to_string(),
            (*src).to_string(),
        );
        state.slot_addrs.insert(
            ModMatrixParam::SlotDestination(slot, None)
                .name()
                .to_string(),
            (*dst).to_string(),
        );
        state.param_values.insert(
            ModMatrixParam::SlotEnabled(slot, true).name().to_string(),
            if *enabled { 1.0 } else { 0.0 },
        );
    }
    state
}

fn stub_panel(mt: ModuleType, instance: u16) -> (ModuleId, ModulePanelState) {
    let id = ModuleId::new(mt, instance);
    (id, ModulePanelState::new(id, Pos2::ZERO))
}

#[test]
fn indexed_mseg_params_sync_to_unique_descriptor_keys() {
    use synth_core::{BipolarValue, Describable, MsegParam, NormalizedValue, Seconds};

    let id = ModuleId::new(ModuleType::Mseg, 1);
    let mut editor = PatchEditor::new();
    editor.add_module(id, synth_modules::Mseg::new().descriptor());
    editor.sync_module_params(
        id,
        &[
            Param::Mseg(MsegParam::SegmentTime(0, Seconds::new(0.25))),
            Param::Mseg(MsegParam::SegmentTime(1, Seconds::new(0.75))),
            Param::Mseg(MsegParam::SegmentLevel(1, NormalizedValue::new(0.4))),
            Param::Mseg(MsegParam::SegmentCurve(1, BipolarValue::new(-0.5))),
        ],
    );

    let panel = editor.panels.get(&id).expect("MSEG panel");
    assert_eq!(panel.param_values.get("Seg 0 Time"), Some(&0.25));
    assert_eq!(panel.param_values.get("Seg 1 Time"), Some(&0.75));
    assert_eq!(panel.param_values.get("Seg 1 Level"), Some(&0.4));
    assert_eq!(panel.param_values.get("Seg 1 Curve"), Some(&-0.5));
    assert!(
        !panel.param_values.contains_key("Seg Time"),
        "ambiguous runtime names must not enter the GUI cache"
    );
}

/// Address-based resolution: `env-6.out → flt-3.cutoff` flags exactly env-6
/// and flt-3 (the real instances the addresses name), even at non-canonical
/// instance numbers. Modules the addresses don't name — and the other
/// envelope — must stay unflagged.
#[test]
fn analysis_marks_source_and_destination_by_address() {
    let mut panels = HashMap::new();
    panels.insert(
        ModuleId::new(ModuleType::ModMatrix, 2),
        matrix_panel(&[(1, "env-6.out", "flt-3.cutoff", true)]),
    );
    for (id, state) in [
        stub_panel(ModuleType::Envelope, 5),
        stub_panel(ModuleType::Envelope, 6),
        stub_panel(ModuleType::Filter, 3),
    ] {
        panels.insert(id, state);
    }

    let analysis = PatchAnalysis::from_panels(&panels, &mut HashMap::new(), HashMap::new());
    assert!(analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Envelope, 6)));
    assert!(analysis.is_mod_matrix_destination(ModuleId::new(ModuleType::Filter, 3)));
    // Per-parameter destination marker (S1.5a): only the addressed param
    // ("cutoff") is a destination on flt-3, not its other knobs.
    let flt3 = ModuleId::new(ModuleType::Filter, 3);
    let cutoff = analysis.markers_for_param(flt3, "cutoff");
    assert!(cutoff.matrix_dest && !cutoff.matrix_source);
    assert!(analysis.markers_for_param(flt3, "resonance").is_empty());
    // The source's output *port* carries the matrix source marker (S1.5 port
    // variant), while a non-referenced port stays clear.
    let env6 = ModuleId::new(ModuleType::Envelope, 6);
    assert!(analysis.markers_for_port(env6, "out").matrix_source);
    assert!(analysis.markers_for_port(env6, "level").is_empty());
    // Instances the addresses don't name (and which aren't present) must not
    // be flagged.
    assert!(!analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Envelope, 2)));
    assert!(!analysis.is_mod_matrix_destination(ModuleId::new(ModuleType::Filter, 1)));
    // The other envelope is not the slot's source.
    assert!(!analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Envelope, 5)));
}

/// Disabled slots must drop out of the reference sets so toggling a
/// slot off in the matrix UI clears the badge immediately.
#[test]
fn disabled_slot_clears_references() {
    let mut panels = HashMap::new();
    panels.insert(
        ModuleId::new(ModuleType::ModMatrix, 1),
        matrix_panel(&[(1, "lfo-1.out", "osc-1.pitch", false)]),
    );
    for (id, state) in [
        stub_panel(ModuleType::Lfo, 1),
        stub_panel(ModuleType::Oscillator, 1),
    ] {
        panels.insert(id, state);
    }
    let analysis = PatchAnalysis::from_panels(&panels, &mut HashMap::new(), HashMap::new());
    assert!(!analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Lfo, 1)));
    assert!(!analysis.is_mod_matrix_destination(ModuleId::new(ModuleType::Oscillator, 1)));
}

/// A disabled Mod Matrix slot must clear its *script*-read sources too, not just
/// the scalar-address ones — the routing is off, so no markers.
#[test]
fn disabled_slot_clears_script_sources() {
    let mut panels = HashMap::new();
    let mm = ModuleId::new(ModuleType::ModMatrix, 1);
    let mut state = ModulePanelState::new(mm, Pos2::ZERO);
    state
        .slot_scripts
        .insert(0, "out = lfo-1.out * velocity".to_string());
    state.param_values.insert(
        ModMatrixParam::SlotEnabled(0, true).name().to_string(),
        0.0, // disabled
    );
    panels.insert(mm, state);
    let (lfo_id, lfo_state) = stub_panel(ModuleType::Lfo, 1);
    panels.insert(lfo_id, lfo_state);

    let analysis = PatchAnalysis::from_panels(&panels, &mut HashMap::new(), HashMap::new());
    assert!(analysis.markers_for_module(lfo_id).is_empty());
    assert!(analysis.markers_for_macro(MacroSource::Velocity).is_empty());
}

/// A parameter written by a Mod Grid graph is marked as a grid destination
/// (per-param and in the module roll-up), independent of the Mod Matrix.
#[test]
fn grid_dest_params_mark_the_parameter() {
    let flt = ModuleId::new(ModuleType::Filter, 1);
    let mut grid: HashMap<ModuleId, std::collections::HashSet<String>> = HashMap::new();
    grid.entry(flt).or_default().insert("cutoff".to_string());

    let analysis = PatchAnalysis::from_panels(&HashMap::new(), &mut HashMap::new(), grid);
    let cutoff = analysis.markers_for_param(flt, "cutoff");
    assert!(cutoff.grid_dest, "cutoff must be a grid destination");
    assert!(!cutoff.matrix_dest, "and not a mod-matrix destination");
    assert!(
        !analysis.markers_for_param(flt, "resonance").grid_dest,
        "an unmodulated param is not marked"
    );
    assert!(
        analysis.markers_for_module(flt).grid_dest,
        "the module roll-up reflects the grid destination"
    );
}

/// A control-rate Script module marks the modules/macros its script reads as
/// *Script* sources (teal), distinct from the Mod Matrix — even though there is
/// no cable and no scalar slot address. (`script_panel` builds a Script module;
/// it is defined alongside the feedback-graph tests below.)
#[test]
fn script_module_marks_read_sources() {
    let mut panels = HashMap::new();
    let (scr, scr_state) = script_panel(
        1,
        &[(
            0,
            "src lfo = lfo-1.out\nsrc cut = flt-1.cutoff\nout = lfo * velocity + cut",
        )],
    );
    panels.insert(scr, scr_state);
    for (id, state) in [
        stub_panel(ModuleType::Lfo, 1),
        stub_panel(ModuleType::Filter, 1),
    ] {
        panels.insert(id, state);
    }
    let analysis = PatchAnalysis::from_panels(&panels, &mut HashMap::new(), HashMap::new());

    // The LFO's output port is read by a Script, not the Mod Matrix.
    let lfo = analysis.markers_for_module(ModuleId::new(ModuleType::Lfo, 1));
    assert!(lfo.script_source && !lfo.matrix_source);
    assert!(!analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Lfo, 1)));

    // A param source lights the Script marker on that exact param only.
    let flt1 = ModuleId::new(ModuleType::Filter, 1);
    let cutoff = analysis.markers_for_param(flt1, "cutoff");
    assert!(cutoff.script_source && !cutoff.matrix_source && !cutoff.matrix_dest);
    assert!(analysis.markers_for_param(flt1, "resonance").is_empty());

    // The macro rail reflects the Script kind.
    assert!(
        analysis
            .markers_for_macro(MacroSource::Velocity)
            .script_source
    );
}

/// An AudioScript compiles with the *audio* dialect: `in_l` is a compile error
/// at control rate, so `lfo-1.out` is only extracted (as an AudioScript source,
/// yellow) if the audio-rate dialect was selected for this module type.
#[test]
fn audio_script_uses_audio_dialect() {
    let mut panels = HashMap::new();
    let (asc, mut asc_state) = stub_panel(ModuleType::AudioScript, 1);
    asc_state
        .slot_scripts
        .insert(0, "src lfo = lfo-1.out\nout = in_l * lfo".to_string());
    panels.insert(asc, asc_state);
    let (lfo_id, lfo_state) = stub_panel(ModuleType::Lfo, 1);
    panels.insert(lfo_id, lfo_state);

    let analysis = PatchAnalysis::from_panels(&panels, &mut HashMap::new(), HashMap::new());
    let lfo = analysis.markers_for_module(lfo_id);
    assert!(
        lfo.audio_script_source,
        "audio-rate dialect must compile and record lfo-1.out"
    );
    assert!(!lfo.matrix_source && !lfo.script_source);
}

/// The patch editor must refuse to highlight / allow a drag that would form
/// a cycle, matching the engine's silent rejection. Graph: osc → amp → out.
#[test]
fn would_create_cycle_matches_engine() {
    let osc = ModuleId::new(ModuleType::Oscillator, 1);
    let amp = ModuleId::new(ModuleType::Amplifier, 1);
    let out = ModuleId::new(ModuleType::StereoOutput, 1);

    let mut editor = PatchEditor::new();
    editor
        .connections
        .push(Connection::new(osc, "out", amp, "in"));
    editor
        .connections
        .push(Connection::new(amp, "out_l", out, "in_l"));

    // Closing the loop back to an upstream module is a cycle.
    assert!(PatchEditor::would_create_cycle(
        &editor.connections,
        out,
        osc
    ));
    assert!(PatchEditor::would_create_cycle(
        &editor.connections,
        amp,
        osc
    ));
    // Self-loop.
    assert!(PatchEditor::would_create_cycle(
        &editor.connections,
        amp,
        amp
    ));
    // A normal downstream edge is fine (parallel edge, no loop).
    assert!(!PatchEditor::would_create_cycle(
        &editor.connections,
        osc,
        out
    ));
    assert!(!PatchEditor::would_create_cycle(
        &editor.connections,
        osc,
        amp
    ));
}

/// S2.4: the panel script mirror is snapshot-driven. `sync_module_scripts`
/// maps the engine snapshot's 1-based slot keys to the panel's 0-based
/// `slot_scripts`, and clear-fills (a slot absent from the snapshot — script
/// cleared in the engine — drops from the mirror).
#[test]
fn sync_module_scripts_maps_one_based_snapshot_and_clear_fills() {
    let mut editor = PatchEditor::new();
    let id = ModuleId::new(ModuleType::ModMatrix, 1);
    editor
        .panels
        .insert(id, ModulePanelState::new(id, Pos2::ZERO));
    // A stale script that the next snapshot omits — must be dropped.
    editor
        .panels
        .get_mut(&id)
        .unwrap()
        .slot_scripts
        .insert(4, "out = 1".to_string());

    let mut snap = std::collections::BTreeMap::new();
    snap.insert("1".to_string(), "out = velocity".to_string());
    snap.insert("3".to_string(), "out = lfo-1.out".to_string());
    editor.sync_module_scripts(id, &snap);

    let scripts = &editor.panels[&id].slot_scripts;
    assert_eq!(scripts.len(), 2);
    assert_eq!(scripts.get(&0).map(String::as_str), Some("out = velocity"));
    assert_eq!(scripts.get(&2).map(String::as_str), Some("out = lfo-1.out"));
    assert!(
        !scripts.contains_key(&4),
        "clear-fill drops scripts absent from the snapshot"
    );

    // An empty snapshot clears the whole mirror.
    editor.sync_module_scripts(id, &std::collections::BTreeMap::new());
    assert!(editor.panels[&id].slot_scripts.is_empty());
}

/// The panel description mirror is snapshot-driven: `sync_module_description`
/// copies the engine snapshot's value in (seeding the info popup + editor),
/// and an empty snapshot value clears it. A missing panel is a no-op.
#[test]
fn sync_module_description_mirrors_snapshot() {
    let mut editor = PatchEditor::new();
    let id = ModuleId::new(ModuleType::Lfo, 1);
    editor
        .panels
        .insert(id, ModulePanelState::new(id, Pos2::ZERO));

    editor.sync_module_description(id, "wobble LFO for the cutoff");
    assert_eq!(editor.panels[&id].description, "wobble LFO for the cutoff");

    // A cleared snapshot value empties the mirror.
    editor.sync_module_description(id, "");
    assert!(editor.panels[&id].description.is_empty());

    // Syncing an unknown module is a no-op (no panic, no insert).
    let ghost = ModuleId::new(ModuleType::Lfo, 99);
    editor.sync_module_description(ghost, "ignored");
    assert!(!editor.panels.contains_key(&ghost));
}

/// The per-frame highlight set (`recompute_drag_cycle_blocked`) must agree
/// with the per-edge `would_create_cycle` check that gates the actual drop.
/// Graph: osc → amp → out.
#[test]
fn drag_cycle_blocked_matches_per_edge_check() {
    let osc = ModuleId::new(ModuleType::Oscillator, 1);
    let amp = ModuleId::new(ModuleType::Amplifier, 1);
    let out = ModuleId::new(ModuleType::StereoOutput, 1);

    let mut editor = PatchEditor::new();
    editor
        .connections
        .push(Connection::new(osc, "out", amp, "in"));
    editor
        .connections
        .push(Connection::new(amp, "out_l", out, "in_l"));

    let pending = |module, direction| {
        Some(node_canvas::WireDrag {
            from: PatchPort {
                module,
                port: "p".into(),
                direction,
                port_type: WidgetPortType::Audio,
            },
            from_pos: Pos2::ZERO,
            armed_by_drag: true,
        })
    };

    // Dragging from `out`'s OUTPUT blocks its ancestors (amp, osc) + itself.
    editor.pending_wire = pending(out, WidgetPortDirection::Output);
    editor.recompute_drag_cycle_blocked();
    assert_eq!(
        editor.drag_cycle_blocked,
        HashSet::from([out, amp, osc]),
        "output drag blocks ancestors"
    );

    // Dragging from `osc`'s INPUT blocks its descendants (amp, out) + itself.
    editor.pending_wire = pending(osc, WidgetPortDirection::Input);
    editor.recompute_drag_cycle_blocked();
    assert_eq!(
        editor.drag_cycle_blocked,
        HashSet::from([osc, amp, out]),
        "input drag blocks descendants"
    );

    // Dragging from `osc`'s OUTPUT has no ancestors — only the self-loop.
    editor.pending_wire = pending(osc, WidgetPortDirection::Output);
    editor.recompute_drag_cycle_blocked();
    assert_eq!(editor.drag_cycle_blocked, HashSet::from([osc]));

    // No drag → empty.
    editor.pending_wire = None;
    editor.recompute_drag_cycle_blocked();
    assert!(editor.drag_cycle_blocked.is_empty());
}

/// A Script-module panel carrying `(0-based slot, source)` YAMS scripts.
fn script_panel(instance: u16, slots: &[(u8, &str)]) -> (ModuleId, ModulePanelState) {
    let id = ModuleId::new(ModuleType::Script, instance);
    let mut state = ModulePanelState::new(id, Pos2::ZERO);
    for (slot, src) in slots {
        state.slot_scripts.insert(*slot, (*src).to_string());
    }
    (id, state)
}

fn graph_of(panels: Vec<(ModuleId, ModulePanelState)>) -> ScriptDepGraph {
    ScriptDepGraph::from_panels_cached(&panels.into_iter().collect(), &mut HashMap::new())
}

/// Run the loop warning for a live `draft` (compiles it to refs first), as the
/// editor does each frame.
fn warn(graph: &ScriptDepGraph, module: ModuleId, slot: u8, draft: &str) -> Option<String> {
    graph.cycle_warning(module, slot, &script_output_refs(draft))
}

/// A script reading its own output is flagged as a self-reference. The draft
/// drives the check, so it fires before the script is even installed.
#[test]
fn cycle_warning_flags_self_reference() {
    let scr = ModuleId::new(ModuleType::Script, 1);
    let graph = graph_of(vec![script_panel(1, &[])]);
    let warning = warn(&graph, scr, 0, "src me = scr-1.out1\nout = me * 0.5")
        .expect("self-reference must warn");
    assert!(warning.contains("feeds back on itself"), "{warning}");
}

/// scr-1.out1 → scr-2.out1 → scr-1.out1 is a two-node cycle. scr-2's edge is
/// installed; scr-1's edge comes from the live draft.
#[test]
fn cycle_warning_flags_two_script_cycle() {
    let scr1 = ModuleId::new(ModuleType::Script, 1);
    let graph = graph_of(vec![
        script_panel(1, &[]),
        script_panel(2, &[(0, "src a = scr-1.out1\nout = a")]),
    ]);
    let warning = warn(&graph, scr1, 0, "src b = scr-2.out1\nout = b").expect("cycle must warn");
    assert!(warning.contains("feedback cycle"), "{warning}");
    assert!(
        warning.contains("scr-1.out1") && warning.contains("scr-2.out1"),
        "{warning}"
    );
}

/// An acyclic chain (scr-1 reads scr-2, scr-2 reads nothing back) is fine.
#[test]
fn cycle_warning_silent_on_acyclic_chain() {
    let scr1 = ModuleId::new(ModuleType::Script, 1);
    let graph = graph_of(vec![
        script_panel(1, &[]),
        script_panel(2, &[(0, "out = 0.5")]),
    ]);
    assert!(warn(&graph, scr1, 0, "src b = scr-2.out1\nout = b").is_none());
}

/// Referencing a non-script source (an LFO) never forms a script cycle, so no
/// warning — only `scr-N.outM` edges count.
#[test]
fn cycle_warning_ignores_non_script_sources() {
    let scr1 = ModuleId::new(ModuleType::Script, 1);
    let graph = graph_of(vec![script_panel(1, &[])]);
    assert!(warn(&graph, scr1, 0, "src l = lfo-1.out\nout = l").is_none());
}

/// A Mod Matrix slot exposes no addressable output, so it can never close a
/// loop even when its script reads one — the warning is Script-module only.
#[test]
fn cycle_warning_only_for_script_modules() {
    let mm = ModuleId::new(ModuleType::ModMatrix, 1);
    let graph = graph_of(vec![script_panel(1, &[(0, "src m = scr-1.out1\nout = m")])]);
    assert!(warn(&graph, mm, 0, "src s = scr-1.out1\nout = s").is_none());
}

#[test]
fn insert_at_cursor_splices_at_index() {
    let mut s = "out = ".to_string();
    // Cursor at end → append; returns the new caret past the text.
    let caret = insert_at_cursor(&mut s, Some((6, 6)), "velocity");
    assert_eq!(s, "out = velocity");
    assert_eq!(caret, 14);
    // `None` cursor falls back to the end.
    let mut s2 = "abc".to_string();
    assert_eq!(insert_at_cursor(&mut s2, None, "X"), 4);
    assert_eq!(s2, "abcX");
    // An out-of-range index clamps to the end rather than panicking.
    let mut s3 = "ab".to_string();
    insert_at_cursor(&mut s3, Some((99, 99)), "!");
    assert_eq!(s3, "ab!");
    // A non-empty range REPLACES the selection (like typing over it).
    let mut s4 = "out = lfo".to_string();
    let caret = insert_at_cursor(&mut s4, Some((6, 9)), "velocity");
    assert_eq!(s4, "out = velocity");
    assert_eq!(caret, 14);
}

#[test]
fn derive_src_var_sanitizes_address() {
    assert_eq!(derive_src_var("lfo-1.out"), "lfo1_out");
    assert_eq!(derive_src_var("scr-2.out1"), "scr2_out1");
}

#[test]
fn existing_binding_var_finds_reusable_binding() {
    let draft = "src lfo = lfo-1.out\nout = lfo * 0.5";
    assert_eq!(
        existing_binding_var(draft, "lfo-1.out").as_deref(),
        Some("lfo")
    );
    assert_eq!(existing_binding_var(draft, "env-1.out"), None);
    // A trailing comment on the binding line is ignored when matching.
    let commented = "src lfo = lfo-1.out  # main lfo\nout = lfo";
    assert_eq!(
        existing_binding_var(commented, "lfo-1.out").as_deref(),
        Some("lfo")
    );
}

#[test]
fn insert_module_source_prepends_binding_then_inserts_var() {
    // No existing binding: prepend `src <var> = <addr>` and insert the var.
    let mut draft = "out = ".to_string();
    let caret = insert_module_source(&mut draft, Some((6, 6)), "lfo-1.out");
    assert_eq!(draft, "src lfo1_out = lfo-1.out\nout = lfo1_out");
    // Caret lands past the inserted variable (shifted by the prepended line).
    assert_eq!(&draft[..caret], "src lfo1_out = lfo-1.out\nout = lfo1_out");
}

#[test]
fn insert_module_source_reuses_existing_binding() {
    // A binding for this address already exists → reuse its var, no prepend.
    let mut draft = "src l = lfo-1.out\nout = ".to_string();
    let end = draft.chars().count();
    insert_module_source(&mut draft, Some((end, end)), "lfo-1.out");
    assert_eq!(draft, "src l = lfo-1.out\nout = l");
}

#[test]
fn insert_module_source_avoids_name_collision() {
    // The derived name `lfo1_out` is already taken by a *different* address;
    // the new binding must get a suffix so the script stays compilable.
    let mut draft = "src lfo1_out = env-2.out\nout = ".to_string();
    let end = draft.chars().count();
    insert_module_source(&mut draft, Some((end, end)), "lfo-1.out");
    assert!(draft.contains("src lfo1_out_2 = lfo-1.out"), "{draft}");
    assert!(draft.ends_with("lfo1_out_2"), "{draft}");
}
