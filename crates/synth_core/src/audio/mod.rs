//! Audio abstraction layer traits and types.
//!
//! This module provides backend-agnostic audio abstractions.
//! Concrete backends (cpal, JACK, etc.) are implemented in the main crate.

mod traits;
mod types;

pub use traits::{AudioBackend, AudioHost, AudioHostTrait, AudioProcessor, AudioStream};
pub use types::*;
