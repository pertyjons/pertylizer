//! Audio abstraction layer.
//!
//! This module provides a backend-agnostic audio API that can work with
//! different audio backends (cpal, JACK, PortAudio, etc.).
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │                    SynthEngine                       │
//! │         (implements AudioProcessor)                  │
//! └─────────────────────┬───────────────────────────────┘
//!                       │
//! ┌─────────────────────▼───────────────────────────────┐
//! │                    AudioHost                         │
//! │              (manages streams)                       │
//! └─────────────────────┬───────────────────────────────┘
//!                       │ AudioBackend trait
//!          ┌────────────┼────────────┐
//!          ▼            ▼            ▼
//!     ┌─────────┐  ┌─────────┐  ┌─────────┐
//!     │  Cpal   │  │  Null   │  │ (future)│
//!     │ Backend │  │ Backend │  │  JACK   │
//!     └─────────┘  └─────────┘  └─────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use pertylizer::audio::*;
//!
//! // Create a simple processor
//! struct SineWave {
//!     phase: f32,
//!     frequency: f32,
//! }
//!
//! impl AudioProcessor for SineWave {
//!     fn process(&mut self, output: &mut [f32], context: &AudioCallbackContext) {
//!         let phase_inc = self.frequency / context.sample_rate.as_f32();
//!         for sample in output.chunks_mut(context.channels as usize) {
//!             let value = (self.phase * std::f32::consts::TAU).sin() * 0.3;
//!             for channel in sample {
//!                 *channel = value;
//!             }
//!             self.phase = (self.phase + phase_inc) % 1.0;
//!         }
//!     }
//! }
//!
//! // Create backend and host
//! let backend = CpalBackend::new()?;
//! let mut host = AudioHost::new(Box::new(backend));
//!
//! // Start audio
//! let processor = SineWave { phase: 0.0, frequency: 440.0 };
//! let config = StreamConfig::default();
//! host.start_output(None, &config, processor)?;
//!
//! // Audio is now playing...
//! std::thread::sleep(std::time::Duration::from_secs(2));
//!
//! host.stop()?;
//! ```

pub mod analysis;
pub mod arrangement_render;
pub mod backends;
pub mod export;
pub mod input;
pub mod mix_analysis;
pub mod preview;
mod traits;
mod types;

pub use backends::{CpalBackend, NullBackend};
pub use traits::{AudioBackend, AudioHost, AudioHostTrait, AudioProcessor, AudioStream};
pub use types::*;

/// Create a default audio host using the CPAL backend.
pub fn default_host() -> AudioResult<AudioHost> {
    let backend = CpalBackend::new()?;
    Ok(AudioHost::new(Box::new(backend)))
}

/// Create a null audio host for testing.
pub fn null_host() -> AudioHost {
    AudioHost::new(Box::new(NullBackend::new()))
}

/// Replay a module snapshot's YAMS scripts onto its Script (`scr`) / Mod-Matrix
/// (`mmx`) / AudioScript (`asc`) slots in an offline-render engine.
///
/// Both offline renderers (`arrangement_render` and `preview`/`OfflineNoteSession`)
/// must do this — without it every scripted module renders silent (an
/// uninstalled script makes the module output zero), which diverges from the
/// live engine. Compilation runs here, off the audio thread; only the shared
/// `Arc<BoundScript>` crosses to the engine. Slot keys are 1-based; a bad key or
/// compile error is recorded in `warnings` (prefixed with `ctx`) and skipped
/// rather than aborting the whole render. The `AudioScript` module compiles in
/// the audio-rate dialect; control-rate hosts use the default dialect.
pub(crate) fn replay_module_scripts(
    handle: &mut synth_engine::EngineHandle,
    instrument_id: synth_engine::instrument::InstrumentId,
    module_snap: &synth_engine::ModuleStateSnapshot,
    warnings: &mut Vec<String>,
    ctx: &str,
) {
    let module_id = module_snap.id;
    for (slot_key, source) in &module_snap.scripts {
        let slot = match crate::session::parse_mod_slot_key(slot_key) {
            Ok(slot) => slot,
            Err(msg) => {
                warnings.push(format!("{ctx}: {module_id} slot {slot_key} script: {msg}"));
                continue;
            }
        };
        let mt = module_id.module_type;
        let script = match crate::session::compile_mod_script(
            source,
            mt.script_is_audio_rate(),
            mt.script_uses_control_ports(),
        ) {
            Ok(script) => script,
            Err(msg) => {
                warnings.push(format!("{ctx}: {module_id} slot {slot_key} script: {msg}"));
                continue;
            }
        };
        // Rebuild the script module's descriptor so the render's voice nodes see
        // its knobs (cross-script reads); `None` for the Mod Matrix.
        let descriptor = crate::session::build_script_descriptor(mt, &script.params);
        if !handle.send_blocking(synth_engine::EngineCommand::SetModScript {
            instrument_id: Some(instrument_id),
            module_id,
            slot,
            script: Some(script),
            descriptor,
        }) {
            warnings.push(format!(
                "{ctx}: failed to enqueue script for module {module_id} slot {slot_key}"
            ));
        }
    }
}

/// Pending-command count below the ring's 256-slot capacity at which an offline
/// load drains. Leaves headroom so a batch enqueued between two drain checks
/// can't push past capacity (and get silently dropped by `send_blocking`).
const OFFLINE_LOAD_DRAIN_THRESHOLD: u64 = 192;

/// Process one silent block so the engine drains its **entire** queued command
/// ring (`process_commands` pops until empty). During an offline load the
/// sequencer is Stopped, so this advances no audio — it only applies the queued
/// commands. Use at phase boundaries where the next step must observe the
/// commands' effect (e.g. instruments applied before `SetModGrid` resolves their
/// id mapping).
pub(crate) fn drain_command_queue(
    engine: &mut synth_engine::SynthEngine,
    buffer: &mut [f32],
    ctx: &synth_core::AudioCallbackContext,
) {
    buffer.fill(0.0);
    engine.process(buffer, ctx);
}

/// Drain the command ring only when it nears the fixed 256-slot capacity, so a
/// bulk offline load can't overflow it however many commands one instrument (or
/// one huge voice graph) enqueues. Cheap when the ring has room — two atomic
/// loads and no processing. `send_blocking` silently *drops* once the ring is
/// full, so this must run often enough during a large load (per module /
/// connection) to keep the pending count below capacity. `enqueued` is bumped by
/// the sender per push and `processed` by the audio-thread drain, on the same
/// shared `CommandSync`, so their difference is the exact ring occupancy.
pub(crate) fn drain_if_ring_filling(
    engine: &mut synth_engine::SynthEngine,
    handle: &synth_engine::EngineHandle,
    buffer: &mut [f32],
    ctx: &synth_core::AudioCallbackContext,
) {
    let sync = &handle.state.command_sync;
    let pending = sync.enqueued().saturating_sub(sync.processed());
    if pending >= OFFLINE_LOAD_DRAIN_THRESHOLD {
        drain_command_queue(engine, buffer, ctx);
    }
}
