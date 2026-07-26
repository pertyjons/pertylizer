//! Read-only patch and script dependency analysis.

use super::*;

/// Which modulation consumers read an element (a param/port/macro) as a source.
/// A single element can be read by several at once, so these are independent
/// flags rather than one enum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SourceKinds {
    matrix: bool,
    script: bool,
    audio_script: bool,
}

impl SourceKinds {
    fn add(&mut self, kind: SourceKind) {
        match kind {
            SourceKind::Matrix => self.matrix = true,
            SourceKind::Script => self.script = true,
            SourceKind::AudioScript => self.audio_script = true,
        }
    }

    #[must_use]
    fn to_markers(self) -> ModMarkers {
        ModMarkers {
            matrix_source: self.matrix,
            script_source: self.script,
            audio_script_source: self.audio_script,
            // Destinations are set by `markers_for_param`, not source-kind roll-up.
            matrix_dest: false,
            grid_dest: false,
        }
    }
}

/// The kind of consumer reading a source, derived from the consuming module type.
#[derive(Debug, Clone, Copy)]
enum SourceKind {
    Matrix,
    Script,
    AudioScript,
}

/// Patch analysis: counts module types to enable smart display names and filtering.
///
/// Built once per frame from the current panels. Used for:
/// - Numbered module titles ("LFO 1" / "LFO 2" when 2+ LFOs, "LFO" when only 1)
/// - Filtering mod matrix dropdown choices (hide "LFO 2" source if only 1 LFO exists)
/// - Modulation-source lookups for the per-knob/port/footer markers (so a module
///   read via the Mod Matrix or a script shows it even with zero cables)
pub(crate) struct PatchAnalysis {
    /// How many of each module type exist.
    module_counts: HashMap<ModuleType, u16>,
    /// Modules read as a modulation source, keyed by source port/param `name`, with
    /// the set of consumer kinds per name (S1.5b). Drives the per-knob/port source
    /// marker when a `name` matches a parameter `type_id` (a `detune` param) or an
    /// output port (`out`). A source may be read via a Mod Matrix slot's scalar
    /// address, a Mod Matrix slot's YAMS script, a Script module, or an AudioScript.
    sources: HashMap<ModuleId, HashMap<String, SourceKinds>>,
    /// Modules referenced as a Mod Matrix destination, with the set of modulated
    /// parameter `type_id`s per module (S1.5a) — drives the per-knob marker; the
    /// keys alone roll up to the module-header badge. Scripts write via ports, so
    /// destinations are Mod-Matrix-only.
    mod_matrix_destinations: HashMap<ModuleId, HashSet<String>>,
    /// Macros (`velocity`, `mod_wheel`, …) read as a modulation source, with the
    /// consumer kinds per macro (S1.5b). Macros have no `ModuleId` to badge, so
    /// they get the macro-source rail.
    macros: HashMap<MacroSource, SourceKinds>,
    /// Parameters written by a Mod Grid graph, keyed by module, with the set of
    /// modulated `type_id`s (the grid sibling of `mod_matrix_destinations`).
    /// Supplied by the caller (it needs the song's mod-graph pool + the edited
    /// instrument id, which `from_panels` doesn't otherwise see).
    grid_dest_params: HashMap<ModuleId, HashSet<String>>,
}

/// Extracted script-read sources for one scripted slot: the module params/ports
/// and macros the compiled script reads (`ScriptInput::Source`). Cached per
/// `(module, slot, text)` so [`PatchAnalysis::from_panels`] doesn't recompile
/// every frame.
#[derive(Clone, Default)]
pub(super) struct ScriptSourceRefs {
    /// `(source module, member name)` for each `SrcAddr::Module` the script reads.
    modules: Vec<(ModuleId, String)>,
    /// Each macro the script reads.
    macros: Vec<MacroSource>,
}

/// Per-slot source-reference cache: `(module, slot) → (script text, refs)`. Skips
/// recompiling a slot whose text is unchanged, so the per-frame cost scales with
/// *changed* scripts, not all of them.
pub(super) type ScriptSourceCache = HashMap<(ModuleId, u8), (String, ScriptSourceRefs)>;

/// The YAMS compile dialect for a script-hosting module's editor / dep-graph
/// compile, derived from the module type. Single source of truth is
/// [`ModuleType::script_is_audio_rate`] / [`ModuleType::script_uses_control_ports`]
/// (also what `session::compile_mod_script` uses for the real install).
fn script_compile_opts(module_type: ModuleType) -> synth_script::CompileOptions {
    synth_script::CompileOptions {
        audio_rate: module_type.script_is_audio_rate(),
        control_ports: module_type.script_uses_control_ports(),
        ..synth_script::CompileOptions::default()
    }
}

/// Compile `src` (with the given dialect) and return its resolved source inputs.
/// Empty for a blank or uncompilable script (the live editor already flags
/// compile errors). Shared by every caller that walks a script's `inputs`, so the
/// compile boilerplate lives in one place.
fn compiled_script_inputs(
    src: &str,
    opts: &synth_script::CompileOptions,
) -> Vec<synth_core::script::ScriptInput> {
    if src.trim().is_empty() {
        return Vec::new();
    }
    let (program, _diags) = synth_script::compile(src, opts);
    // `into_bound` needs an owned source string only for persistence/inspection;
    // we read `inputs` and discard it, so an empty string is fine here.
    program.map_or_else(Vec::new, |p| p.into_bound(String::new()).inputs)
}

/// Collect the modules and macros a script reads as sources (`ScriptInput::Source`).
fn extract_script_sources(src: &str, module_type: ModuleType) -> ScriptSourceRefs {
    let mut refs = ScriptSourceRefs::default();
    for input in compiled_script_inputs(src, &script_compile_opts(module_type)) {
        match input {
            synth_core::script::ScriptInput::Source(SrcAddr::Module {
                module_type,
                instance,
                name,
            }) => {
                refs.modules.push((
                    ModuleId::new(module_type, instance),
                    name.as_str().to_string(),
                ));
            }
            synth_core::script::ScriptInput::Source(SrcAddr::Macro(m)) => refs.macros.push(m),
            _ => {}
        }
    }
    refs
}

impl PatchAnalysis {
    /// Build from current patch panels. `cache` memoises each scripted slot's
    /// extracted sources so unchanged scripts are not recompiled every frame.
    pub(super) fn from_panels(
        panels: &HashMap<ModuleId, ModulePanelState>,
        cache: &mut ScriptSourceCache,
        grid_dest_params: HashMap<ModuleId, HashSet<String>>,
    ) -> Self {
        let mut module_counts: HashMap<ModuleType, u16> = HashMap::new();
        for id in panels.keys() {
            *module_counts.entry(id.module_type).or_insert(0) += 1;
        }

        let mut sources: HashMap<ModuleId, HashMap<String, SourceKinds>> = HashMap::new();
        let mut mod_matrix_destinations: HashMap<ModuleId, HashSet<String>> = HashMap::new();
        let mut macros: HashMap<MacroSource, SourceKinds> = HashMap::new();

        // Record a module-read source, ignoring addresses that name a module not in
        // the patch. Owned `String` key (cloned only on insert): `markers_for_param`
        // looks up by `&str` per knob per frame, so a `PortName` set would force a
        // global intern lock.
        let record_module = |sources: &mut HashMap<ModuleId, HashMap<String, SourceKinds>>,
                             mid: ModuleId,
                             name: &str,
                             kind: SourceKind| {
            if panels.contains_key(&mid) {
                sources
                    .entry(mid)
                    .or_default()
                    .entry(name.to_string())
                    .or_default()
                    .add(kind);
            }
        };

        for (id, panel) in panels {
            // Only these three module types read modulation sources; the consumer's
            // type is the source kind (matrix / script / audio-script marker).
            let kind = match id.module_type {
                ModuleType::ModMatrix => SourceKind::Matrix,
                ModuleType::Script => SourceKind::Script,
                ModuleType::AudioScript => SourceKind::AudioScript,
                _ => continue,
            };

            // Mod Matrix scalar slot addresses (S1.5c): resolve each enabled slot's
            // source/dest address (mirrored in `slot_addrs`) to the module it names.
            // The f32 index in `param_values` can't represent arbitrary addresses
            // (`lfo-3.out`), so reading it here would miss the picker's targets.
            if id.module_type == ModuleType::ModMatrix {
                for slot in 0..synth_core::MAX_MOD_MATRIX_SLOTS as u8 {
                    let enabled_name = ModMatrixParam::SlotEnabled(slot, true).name();
                    let enabled = panel
                        .param_values
                        .get(enabled_name)
                        .map(|v| *v != 0.0)
                        .unwrap_or(true);
                    if !enabled {
                        continue;
                    }

                    let source_name = ModMatrixParam::SlotSource(slot, None).name();
                    if let Some(addr) = panel.slot_addrs.get(source_name) {
                        match SrcAddr::parse(addr) {
                            Some(SrcAddr::Module {
                                module_type,
                                instance,
                                name,
                            }) => {
                                let mid = ModuleId::new(module_type, instance);
                                record_module(&mut sources, mid, name.as_str(), SourceKind::Matrix);
                            }
                            // A macro source has no `ModuleId` — record it for the rail.
                            Some(SrcAddr::Macro(m)) => macros.entry(m).or_default().add(kind),
                            None => {}
                        }
                    }

                    let dest_name = ModMatrixParam::SlotDestination(slot, None).name();
                    if let Some(addr) = panel.slot_addrs.get(dest_name)
                        && let Some(dst) = DestAddr::parse(addr)
                    {
                        let mid = ModuleId::new(dst.module_type, dst.instance);
                        if panels.contains_key(&mid) {
                            mod_matrix_destinations
                                .entry(mid)
                                .or_default()
                                .insert(dst.param.as_str().to_string());
                        }
                    }
                }
            }

            // Script-read sources (all three consumer kinds): each scripted slot's
            // YAMS text reads modules/macros that are invisible to the cable graph
            // and the scalar addresses above. Compile once per changed slot (cached),
            // with the dialect the module's real install uses.
            for (slot, src) in &panel.slot_scripts {
                // A disabled Mod Matrix slot routes nothing, so its script must not
                // emit markers either — mirroring the scalar-address path above.
                // Script / AudioScript modules have no per-slot enable.
                if id.module_type == ModuleType::ModMatrix {
                    let enabled = panel
                        .param_values
                        .get(ModMatrixParam::SlotEnabled(*slot, true).name())
                        .map(|v| *v != 0.0)
                        .unwrap_or(true);
                    if !enabled {
                        continue;
                    }
                }

                let key = (*id, *slot);
                // Recompute only when the slot's script text changed; on a cache hit
                // read the stored refs by reference (no per-frame Vec clone).
                if !matches!(cache.get(&key), Some((cached_src, _)) if cached_src == src) {
                    cache.insert(
                        key,
                        (src.clone(), extract_script_sources(src, id.module_type)),
                    );
                }
                let refs = &cache[&key].1;
                for (mid, name) in &refs.modules {
                    record_module(&mut sources, *mid, name, kind);
                }
                for m in &refs.macros {
                    macros.entry(*m).or_default().add(kind);
                }
            }
        }

        Self {
            module_counts,
            sources,
            mod_matrix_destinations,
            macros,
            grid_dest_params,
        }
    }

    /// Get count of a specific module type.
    pub(super) fn count(&self, module_type: ModuleType) -> u16 {
        self.module_counts.get(&module_type).copied().unwrap_or(0)
    }

    /// Generate display name for a module.
    ///
    /// Always appends the instance number for consistency,
    /// e.g. "LFO 1", "Oscillator 1", even when only one exists.
    #[must_use]
    pub(super) fn display_name(&self, module_id: ModuleId, base_name: &str) -> String {
        format!("{base_name} {}", module_id.instance)
    }

    /// `true` if any Mod Matrix slot routes from this module (used for node-opacity
    /// dimming, which follows the Mod Matrix specifically).
    pub(super) fn is_mod_matrix_source(&self, module_id: ModuleId) -> bool {
        self.sources
            .get(&module_id)
            .is_some_and(|members| members.values().any(|k| k.matrix))
    }

    /// `true` if any Mod Matrix slot routes to this module.
    pub(super) fn is_mod_matrix_destination(&self, module_id: ModuleId) -> bool {
        self.mod_matrix_destinations.contains_key(&module_id)
    }

    /// The modulation markers a macro carries (S1.5b) — the source kinds that read
    /// it. Macros are never a destination.
    pub(super) fn markers_for_macro(&self, macro_source: MacroSource) -> ModMarkers {
        self.macros
            .get(&macro_source)
            .copied()
            .unwrap_or_default()
            .to_markers()
    }

    /// The modulation markers of a specific parameter on a module, for the per-knob
    /// marker (S1.5a/b). Source kinds are tracked per member name (a source `name`
    /// that is a parameter such as `detune` marks that knob); destinations are
    /// always a parameter. `param_type_id` is the descriptor `type_id`.
    pub(super) fn markers_for_param(&self, module_id: ModuleId, param_type_id: &str) -> ModMarkers {
        let mut markers = self
            .sources
            .get(&module_id)
            .and_then(|members| members.get(param_type_id))
            .copied()
            .unwrap_or_default()
            .to_markers();
        markers.matrix_dest = self
            .mod_matrix_destinations
            .get(&module_id)
            .is_some_and(|params| params.contains(param_type_id));
        markers.grid_dest = self
            .grid_dest_params
            .get(&module_id)
            .is_some_and(|params| params.contains(param_type_id));
        markers
    }

    /// The source markers of an output port (S1.5, port variant). A port is only
    /// ever a source, never a destination, so this reports source kinds only.
    pub(super) fn markers_for_port(&self, module_id: ModuleId, port_name: &str) -> ModMarkers {
        self.sources
            .get(&module_id)
            .and_then(|members| members.get(port_name))
            .copied()
            .unwrap_or_default()
            .to_markers()
    }

    /// The module-level roll-up of markers for the bottom status-bar badge — the
    /// union of every source/destination role any of the module's params/ports
    /// participates in.
    pub(super) fn markers_for_module(&self, module_id: ModuleId) -> ModMarkers {
        let mut markers = ModMarkers::default();
        if let Some(members) = self.sources.get(&module_id) {
            for k in members.values() {
                markers.matrix_source |= k.matrix;
                markers.script_source |= k.script;
                markers.audio_script_source |= k.audio_script;
            }
        }
        markers.matrix_dest = self.is_mod_matrix_destination(module_id);
        markers.grid_dest = self.grid_dest_params.contains_key(&module_id);
        markers
    }
}

/// The Script-module output slots a YAMS script references, found by compiling it
/// and resolving its `scr-N.outM` source addresses. Only Script outputs are
/// returned — they are the sole scripted slot exposing an addressable output a
/// later script can read back, so they are the only edges that can close a latent
/// feedback loop (LFO / macro / context sources can't). A script that fails to
/// compile yields an empty set (the live editor already flags the compile error).
pub(super) fn script_output_refs(src: &str) -> HashSet<(ModuleId, u8)> {
    // Feedback detection only concerns Script modules — compile in their
    // control-ports dialect so `in1..in4` / `out1..out4` don't break the compile.
    script_refs_from_inputs(&compiled_script_inputs(
        src,
        &script_compile_opts(ModuleType::Script),
    ))
}

/// The Script-module output slots referenced by an already-compiled script's
/// `inputs`. Split out from [`script_output_refs`] so a caller that already
/// compiled the source (the live editor's status line) can extract refs without
/// recompiling.
pub(super) fn script_refs_from_inputs(
    inputs: &[synth_core::script::ScriptInput],
) -> HashSet<(ModuleId, u8)> {
    let mut refs = HashSet::new();
    for input in inputs {
        if let synth_core::script::ScriptInput::Source(SrcAddr::Module {
            module_type: ModuleType::Script,
            instance,
            name,
        }) = input
            && let Some(slot) = synth_modules::script_module::output_port_slot(name.as_str())
        {
            refs.insert((ModuleId::new(ModuleType::Script, *instance), slot as u8));
        }
    }
    refs
}

/// Latent script→script feedback edges across the whole patch, for the ƒx
/// editor's loop warning (§3.5). YAMS sources are address-based and resolved with
/// a one-block latency — they bypass the graph's cable cycle-detection
/// (`drag_cycle_blocked`), so a script reading its own (or a downstream script's)
/// output forms a delayed feedback path the cable checks never see. The delay
/// makes this safe at runtime (no infinite loop / stack overflow — YAMS bytecode
/// is straight-line); the warning exists purely so the user isn't surprised by
/// the one-block feedback.
///
/// Nodes are `(Script ModuleId, 0-based slot)` — the only scripted slot with an
/// addressable output. A Mod Matrix slot can read a script but exposes no output,
/// so it can never close a loop and is not a node. Built only while an expression
/// editor is open (it compiles every installed script).
pub(super) struct ScriptDepGraph {
    /// node → the Script output slots its installed script reads.
    edges: HashMap<(ModuleId, u8), HashSet<(ModuleId, u8)>>,
}

/// Per-slot compiled-source-reference cache: `(module, slot) → (script text,
/// referenced Script outputs)`. Lets [`ScriptDepGraph::from_panels_cached`] skip
/// recompiling a slot whose text is unchanged.
pub(super) type ScriptRefCache = HashMap<(ModuleId, u8), (String, HashSet<(ModuleId, u8)>)>;

impl ScriptDepGraph {
    /// Build from the installed scripts of every Script-module panel, reusing
    /// `cache` so a slot whose script text is unchanged is not recompiled — the
    /// per-frame cost while the ƒx editor is open then scales with *changed*
    /// scripts, not all of them.
    pub(super) fn from_panels_cached(
        panels: &HashMap<ModuleId, ModulePanelState>,
        cache: &mut ScriptRefCache,
    ) -> Self {
        let mut edges = HashMap::new();
        for (id, panel) in panels {
            if id.module_type != ModuleType::Script {
                continue;
            }
            for (slot, src) in &panel.slot_scripts {
                let key = (*id, *slot);
                let refs = match cache.get(&key) {
                    Some((cached_src, cached_refs)) if cached_src == src => cached_refs.clone(),
                    _ => {
                        let r = script_output_refs(src);
                        cache.insert(key, (src.clone(), r.clone()));
                        r
                    }
                };
                if !refs.is_empty() {
                    edges.insert(key, refs);
                }
            }
        }
        Self { edges }
    }

    /// A human-readable warning if installing the script whose extracted
    /// `draft_refs` are given on `(module_id, slot)` would make that Script slot
    /// read its own output back (self-reference) or sit on a script→script cycle —
    /// both resolve with a one-block delay. `None` when the edited module is not a
    /// Script module, or no loop is formed. `draft_refs` come from the live draft
    /// (not its installed script), so the warning updates as the user types; the
    /// caller passes them already-extracted to avoid a redundant recompile.
    pub(super) fn cycle_warning(
        &self,
        module_id: ModuleId,
        slot: u8,
        draft_refs: &HashSet<(ModuleId, u8)>,
    ) -> Option<String> {
        if module_id.module_type != ModuleType::Script {
            return None;
        }
        let start = (module_id, slot);
        // Direct self-reference is the simplest loop — name it explicitly.
        if draft_refs.contains(&start) {
            return Some(format!(
                "feeds back on itself (scr-{}.out{} reads its own output) — \
                 resolved 1 block late",
                module_id.instance,
                slot + 1,
            ));
        }
        let path = self.find_cycle_path(start, draft_refs)?;
        let chain = path
            .iter()
            .map(|(id, s)| format!("scr-{}.out{}", id.instance, s + 1))
            .collect::<Vec<_>>()
            .join(" → ");
        Some(format!(
            "forms a feedback cycle ({chain}) — resolved 1 block late"
        ))
    }

    /// DFS for a path `start → … → start`, using `start`'s edges from
    /// `start_refs` (the live draft) and every other node's installed edges.
    /// Returns the cycle as a node sequence that begins and ends at `start`, or
    /// `None` if acyclic. The graph is tiny (a handful of script slots), so plain
    /// recursion with a global visited set is ample.
    fn find_cycle_path(
        &self,
        start: (ModuleId, u8),
        start_refs: &HashSet<(ModuleId, u8)>,
    ) -> Option<Vec<(ModuleId, u8)>> {
        let mut path = vec![start];
        let mut visited = HashSet::new();
        visited.insert(start);
        for &next in start_refs {
            if let Some(found) = self.dfs_to(start, next, &mut path, &mut visited) {
                return Some(found);
            }
        }
        None
    }

    /// Recursive helper for [`Self::find_cycle_path`]. Returns the closed cycle
    /// path when `node` can reach `start`. `visited` is global: a node that can't
    /// reach `start` down one branch can't down another either, so it is never
    /// retried.
    fn dfs_to(
        &self,
        start: (ModuleId, u8),
        node: (ModuleId, u8),
        path: &mut Vec<(ModuleId, u8)>,
        visited: &mut HashSet<(ModuleId, u8)>,
    ) -> Option<Vec<(ModuleId, u8)>> {
        if node == start {
            let mut cycle = path.clone();
            cycle.push(start);
            return Some(cycle);
        }
        if !visited.insert(node) {
            return None;
        }
        path.push(node);
        if let Some(neighbours) = self.edges.get(&node) {
            for &next in neighbours {
                if let Some(found) = self.dfs_to(start, next, path, visited) {
                    return Some(found);
                }
            }
        }
        path.pop();
        None
    }
}
