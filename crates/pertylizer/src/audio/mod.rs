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

pub mod backends;
pub mod export;
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
