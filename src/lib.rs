// Crate-level clippy allows for synth-appropriate exceptions.
// See CLAUDE.md "Godkända undantag" for rationale.

// Pedantic lints som är för strikta för detta projekt
#![allow(clippy::must_use_candidate)] // Inte alla getters behöver #[must_use]
#![allow(clippy::missing_const_for_fn)] // const fn överallt är onödigt
#![allow(clippy::module_name_repetitions)] // FilterMode är tydligare än filter::Mode
#![allow(clippy::similar_names)] // peak_l/peak_r, left/right är OK
#![allow(clippy::too_many_lines)] // process() funktioner kan vara långa
#![allow(clippy::unnecessary_wraps)] // Option/Result för API-konsistens
#![allow(clippy::use_self)] // ModuleId::new() är tydligare än Self::new() ibland

// Casts - normalt i audio/DSP-kod
#![allow(clippy::cast_precision_loss)] // usize/u64 -> f32 är standard i audio
#![allow(clippy::cast_possible_truncation)] // f32 -> usize med bounds check är OK
#![allow(clippy::cast_sign_loss)] // Där värdet garanterat är positivt
#![allow(clippy::cast_lossless)] // as-casts är tydligare än From i DSP

// Matematik och DSP
#![allow(clippy::suboptimal_flops)] // mul_add() är inte alltid snabbare/tydligare
#![allow(clippy::many_single_char_names)] // t, x, y, a, b, c är standard i matematik
#![allow(clippy::float_cmp)] // Exakta jämförelser i tester och constants

// Dokumentation (intern kod, inte publikt API)
#![allow(clippy::doc_markdown)] // Backticks i docs är pedantiskt
#![allow(clippy::missing_errors_doc)] // Errors-sektion krävs inte internt
#![allow(clippy::missing_panics_doc)] // Panics-sektion krävs inte internt

// Kodstil
#![allow(clippy::uninlined_format_args)] // format!("{}", x) är OK
#![allow(clippy::redundant_closure_for_method_calls)] // .map(|x| x.foo()) är tydligare ibland
#![allow(clippy::option_if_let_else)] // if let är ofta tydligare än map_or
#![allow(clippy::match_same_arms)] // Ibland för tydlighet
#![allow(clippy::items_after_statements)] // Lokala konstanter i funktioner är OK
#![allow(clippy::wildcard_imports)] // use super::* i tester är OK
#![allow(clippy::needless_pass_by_value)] // Ibland för API-konsistens
#![allow(clippy::unused_self)] // &self för framtida utökning eller API-konsistens
#![allow(clippy::map_unwrap_or)] // .map().unwrap_or() är ofta tydligare
#![allow(clippy::unnecessary_literal_bound)] // &str vs &'static str - pedantiskt
#![allow(clippy::semicolon_if_nothing_returned)] // Stilfråga
#![allow(clippy::explicit_iter_loop)] // for x in vec.iter() är OK
#![allow(clippy::return_self_not_must_use)] // Builders utan #[must_use] är OK
#![allow(clippy::assigning_clones)] // clone_into() är inte alltid tydligare
#![allow(clippy::manual_let_else)] // if let + return är ibland tydligare
#![allow(clippy::match_wildcard_for_single_variants)] // _ för framtida varianter
#![allow(clippy::single_match_else)] // match med else är tydligare ibland
#![allow(clippy::while_float)] // Loopar med float-villkor är OK i DSP
#![allow(clippy::iter_without_into_iter)] // Iterator impl utan IntoIterator är OK
#![allow(clippy::trivially_copy_pass_by_ref)] // &self för Copy-typer är OK för konsistens
#![allow(clippy::significant_drop_in_scrutinee)] // Drop-varningar i match är pedantiskt
#![allow(clippy::significant_drop_tightening)] // Drop-ordning är sällan kritisk
#![allow(clippy::derive_partial_eq_without_eq)] // PartialEq utan Eq för f32-innehåll
#![allow(clippy::struct_field_names)] // field_name som börjar med struct namn är OK
#![allow(clippy::or_fun_call)] // .unwrap_or(Vec::new()) är tydligare ibland
#![allow(clippy::redundant_clone)] // Ibland för tydlighet
#![allow(clippy::manual_midpoint)] // (a + b) / 2 är tydligare än midpoint()
#![allow(clippy::single_char_pattern)] // .split(",") vs .split(',') - stilfråga
#![allow(clippy::manual_inspect)] // .map(|x| { ...; x }) är OK
#![allow(clippy::unnecessary_struct_initialization)] // Struct { ..default } är tydligare
#![allow(clippy::cognitive_complexity)] // Komplexa funktioner är OK i DSP
#![allow(clippy::fn_params_excessive_bools)] // Flera bool-parametrar är OK ibland
#![allow(clippy::struct_excessive_bools)] // Flera bool-fält är OK ibland
#![allow(clippy::iter_on_single_items)] // [x].iter() är OK
#![allow(clippy::iter_on_empty_collections)] // [].iter() är OK
#![allow(clippy::type_repetition_in_bounds)] // where T: A, T: B är tydligare ibland
#![allow(clippy::cast_possible_wrap)] // i32 -> usize där värdet är känt
#![allow(clippy::cloned_instead_of_copied)] // .cloned() för framtida ändring
#![allow(clippy::needless_collect)] // collect() innan iteration är ibland tydligare
#![allow(clippy::ptr_arg)] // &Vec är OK för API-konsistens
#![allow(clippy::manual_non_exhaustive)] // #[non_exhaustive] behövs inte alltid
#![allow(clippy::imprecise_flops)] // hypot() är inte alltid tillgänglig/snabbare
#![allow(clippy::explicit_deref_methods)] // .deref() är tydligare ibland
#![allow(clippy::match_bool)] // match bool är tydligare än if/else ibland
#![allow(clippy::needless_for_each)] // for_each är tydligare ibland
#![allow(clippy::if_not_else)] // if !cond { } else { } är OK
#![allow(clippy::range_plus_one)] // 0..n+1 är tydligare än 0..=n ibland
#![allow(clippy::comparison_to_empty)] // x.is_empty() vs x == "" är stilfråga
#![allow(clippy::missing_fields_in_debug)] // Manuell Debug utan alla fält är OK
#![allow(clippy::mut_mut)] // &mut &mut T är OK ibland
#![allow(clippy::indexing_slicing)] // match på vec[i] är OK
#![allow(clippy::used_underscore_binding)] // _unused som sedan används är OK
#![allow(clippy::iter_nth)] // .iter().nth(n) är tydligare än .get(n) ibland
#![allow(clippy::extra_unused_lifetimes)] // Extra lifetimes för framtida utökning
#![allow(clippy::let_underscore_untyped)] // let _ = expr; utan typ är OK
#![allow(clippy::box_collection)] // Box<Vec<T>> kan vara avsiktligt
#![allow(clippy::needless_lifetimes)] // Explicita lifetimes för tydlighet
#![allow(clippy::elidable_lifetime_names)] // Namngivna lifetimes som kan elideras
#![allow(clippy::set_contains_or_insert)] // HashSet contains + insert är OK
#![allow(clippy::implicit_hasher)] // HashMap<K,V> utan S parameter är OK
#![allow(clippy::ignored_unit_patterns)] // _ för () är OK
#![allow(clippy::needless_pass_by_ref_mut)] // &mut för framtida användning

//! Modular Synthesizer
//!
//! A flexible, modular audio synthesis system with:
//! - Abstract audio backend (supports cpal, with room for JACK, PortAudio, etc.)
//! - Lock-free UI communication
//! - Real-time safe audio processing
//! - Self-describing modules for dynamic UI generation
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                          UI                                  │
//! │  - Reads ModuleDescriptors for widget generation            │
//! │  - Sends EngineCommands via lock-free queue                 │
//! │  - Reads meters/state via atomics                           │
//! └──────────────┬──────────────────────────┬───────────────────┘
//!                │ Commands                  │ State reads
//!                ▼                           │
//! ┌──────────────────────────────────────────▼──────────────────┐
//! │                      SynthEngine                             │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
//! │  │ CommandQueue│  │ ModuleGraph │  │ StateSnapshot       │  │
//! │  │ (lock-free) │  │ (future)    │  │ (atomic values)     │  │
//! │  └─────────────┘  └─────────────┘  └─────────────────────┘  │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │ AudioProcessor trait
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │                      AudioHost                               │
//! │                 (manages streams)                            │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │ AudioBackend trait
//!              ┌────────────┼────────────┐
//!              ▼            ▼            ▼
//!         ┌─────────┐  ┌─────────┐  ┌─────────┐
//!         │  Cpal   │  │  Null   │  │ (future)│
//!         │ Backend │  │ Backend │  │  JACK   │
//!         └─────────┘  └─────────┘  └─────────┘
//! ```
//!
//! # Quick Start
//!
//! ```ignore
//! use modular_synth::{audio, engine};
//!
//! // Create the synth engine
//! let (engine, mut handle) = engine::SynthEngine::new();
//!
//! // Create audio host with default backend
//! let mut host = audio::default_host()?;
//!
//! // Start audio
//! let config = audio::StreamConfig::default();
//! host.start_output(None, &config, engine)?;
//!
//! // Play a note
//! handle.note_on(60, 0.8); // Middle C, velocity 0.8
//!
//! // ... later
//! handle.note_off(60);
//!
//! // Check meters
//! let (peak_l, peak_r) = handle.peak_meters();
//! println!("Peak: L={peak_l:.2} R={peak_r:.2}");
//!
//! // Stop when done
//! host.stop()?;
//! ```
//!
//! # Rust 2024 Features Used
//!
//! This crate uses several Rust 2024 features:
//! - Let chains in if/while expressions
//! - Improved pattern matching
//! - Enhanced error handling

// Temporarily allow missing docs during development
#![allow(missing_docs)]
#![warn(clippy::all)]
#![allow(clippy::module_inception)]

// Pedantic/nursery lints som behöver vara efter warn(clippy::all)
#![allow(clippy::vec_box)] // Vec<Box<T>> för trait objects
#![allow(clippy::needless_range_loop)] // for i in 0..len { arr[i] }
#![allow(clippy::should_implement_trait)] // next() utan Iterator
#![allow(clippy::manual_range_contains)] // x >= 0 && x <= 127
#![allow(clippy::approx_constant)] // PI-värden
#![allow(clippy::field_reassign_with_default)] // Default::default() + assign

pub mod audio;
pub mod effects;
pub mod engine;
pub mod gui;
pub mod io;
pub mod modules;
pub mod patch;
pub mod patches;
pub mod sequencer;
pub mod types;
pub mod ui;
pub mod visualizers;

#[cfg(test)]
mod tests;

// Re-export commonly used items
pub use audio::{AudioHost, AudioHostTrait, AudioProcessor, StreamConfig};
pub use engine::{EngineCommand, EngineHandle, SynthEngine};

// Re-export voice management
pub use engine::{
    AllocationMode, AllocatorConfig, Connection, GraphError, ModuleGraph, NotePriority,
    StealingStrategy, Voice, VoiceAllocator, VoiceState,
};

// Re-export module types
pub use modules::{
    Amplifier, AudioBuffer, AudioEffect, Describable, Envelope, Filter, FilterType, LadderFilter,
    Lfo, LfoWaveform, Mixer, ModuleCategory, ModuleDescriptor, Oscillator, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, Waveform, WidgetHint,
};

// Re-export effects
pub use effects::{Chorus, Delay, Distortion, Reverb};

// Re-export visualizers
pub use visualizers::{LevelMeter, Oscilloscope, VisualizationBuffer};

// Re-export UI abstractions
pub use ui::{ModulePanel, ParameterWidget, SynthEvent, UiEvent, UiState};

// Re-export GUI abstractions
pub use gui::{GuiBackend, GuiType, SynthGuiConfig, create_backend, print_available_backends};

// Re-export patch system
pub use patch::{
    ConnectionState, ModuleState, ModuleType, Patch, PatchError, PatchSettings, example_patches,
};

// Re-export I/O
pub use io::{PatchInfo, PatchManager};

// Re-export sequencer types
pub use sequencer::{
    // Automation
    AutomationLane,
    AutomationPoint,
    AutomationTarget,
    CurveType,
    Duration,
    // Effects
    EffectCommand,
    InputCommand,
    InputSource,
    InstrumentId,
    Note,
    NoteId,
    NoteName,
    Pattern,
    // IDs
    PatternId,
    PatternTick,
    // Pitch/velocity
    Pitch,
    // Events and input
    SequencerEvent,
    SequencerTrack,
    // Core types
    Song,
    TICKS_PER_QUARTER,
    // Time types
    Tick,
    TimeSignature,
    TrackId,
    TrackerCell,
    // View helpers
    TrackerRow,
    TrackerViewConfig,
    Velocity,
};
