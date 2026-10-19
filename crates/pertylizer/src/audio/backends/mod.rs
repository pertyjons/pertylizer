//! Audio backend implementations.

mod cpal_backend;
mod null_backend;
#[cfg(feature = "v2-lowering")]
mod retirement;

pub use cpal_backend::CpalBackend;
pub use null_backend::NullBackend;
